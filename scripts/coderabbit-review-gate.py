#!/usr/bin/env python3
"""Require CodeRabbit review and approval, or a scoped, audited human override.

Run from the trusted default-branch checkout, never a contributor's checkout.
Skipped, neutral, failed and synthetic queue checks do not establish a review.
"""

import json
import os
import sys
import time
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

CODERABBIT_APP_ID = 347564
CODERABBIT_USER_ID = 136622811
GITHUB_ACTIONS_APP_ID = 15368
CHECK_NAME = "CodeRabbit reviewed"
OVERRIDE_NAME = "CodeRabbit review override"
WAIT_SECONDS = 25 * 60
POLL_SECONDS = 60


class GitHub:
    """Use the job token for bounded GitHub API requests without logging it."""

    def __init__(self, repository, token, api_url):
        self.repository = repository
        self.token = token
        self.api_url = api_url.rstrip("/")

    def request(self, method, path, data=None):
        """Decode JSON or raise on network/API errors; each request has a timeout."""
        body = None if data is None else json.dumps(data).encode()
        request = Request(
            self.api_url + path,
            data=body,
            method=method,
            headers={
                "Authorization": "Bearer " + self.token,
                "Accept": "application/vnd.github+json",
                "Content-Type": "application/json",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        with urlopen(request, timeout=15) as response:
            return json.load(response)

    def repo_request(self, method, path, data=None):
        """Address the event repository, never an arbitrary URL from PR content."""
        return self.request(method, f"/repos/{self.repository}{path}", data)

    def queue_entries(self, branch):
        """Read every entry for one branch; API errors and partial pages fail closed."""
        owner, name = self.repository.split("/", 1)
        query = """
          query($owner: String!, $name: String!, $branch: String!, $after: String) {
            repository(owner: $owner, name: $name) {
              mergeQueue(branch: $branch) {
                entries(first: 100, after: $after) {
                  nodes { position headCommit { oid }
                    pullRequest { number headRefOid } }
                  pageInfo { hasNextPage endCursor }
                }
              }
            }
          }
        """
        entries, after = [], None
        while True:
            result = self.request("POST", "/graphql", {
                "query": query,
                "variables": {
                    "owner": owner, "name": name, "branch": branch, "after": after,
                },
            })
            if result.get("errors"):
                raise RuntimeError("GitHub could not resolve the merge queue")
            queue = result["data"]["repository"]["mergeQueue"]
            if queue is None:
                raise RuntimeError("Merge queue no longer exists")
            page = queue["entries"]
            entries.extend(page["nodes"])
            if not page["pageInfo"]["hasNextPage"]:
                return entries
            cursor = page["pageInfo"]["endCursor"]
            if not cursor or cursor == after:
                raise RuntimeError("GitHub returned an incomplete queue page")
            after = cursor

    def checks(self, sha):
        """Read every check on the exact commit, including superseded attempts."""
        runs, page = [], 1
        while True:
            query = urlencode({"per_page": 100, "page": page, "filter": "all"})
            result = self.repo_request("GET", f"/commits/{sha}/check-runs?{query}")
            batch = result["check_runs"]
            runs.extend(batch)
            if len(batch) < 100:
                return runs
            page += 1

    def latest_approval(self, number, sha):
        """Read the latest CodeRabbit review; older approvals cannot cover a new head."""
        reviews, page = [], 1
        while True:
            batch = self.repo_request("GET", f"/pulls/{number}/reviews?per_page=100&page={page}")
            reviews.extend(
                review for review in batch
                if review.get("user", {}).get("id") == CODERABBIT_USER_ID
                and review.get("state") != "PENDING"
            )
            if len(batch) < 100:
                latest = max(reviews, key=lambda review: review["id"], default=None)
                return latest if latest and latest.get("commit_id") == sha else None
            page += 1


def latest_check(runs, name, app_id, sha):
    """Select the latest named check from the expected app on the pinned commit."""
    matching = (
        run for run in runs
        if run.get("name") == name
        and run.get("app", {}).get("id") == app_id
        and run.get("head_sha") == sha
    )
    return max(matching, key=lambda run: run["id"], default=None)


def review_state(run):
    """Accept only CodeRabbit's explicit completed-review result, not a green skip."""
    if run is None or run.get("status") != "completed":
        return "waiting"
    output = run.get("output") or {}
    if (
        run.get("conclusion") == "success"
        and output.get("title") == "Finished"
        and (output.get("summary") or "").startswith("Result: Success - Review completed\n")
    ):
        return "reviewed"
    return "blocked"


def override_id(number, sha):
    """Bind an override to one PR and commit, including when a queue checks it."""
    return f"pcbcraft-coderabbit-override:{number}:{sha}"


def has_override(runs, number, sha):
    """Honor only a successful, scoped override published by GitHub Actions."""
    scoped = [run for run in runs if run.get("external_id") == override_id(number, sha)]
    run = latest_check(scoped, OVERRIDE_NAME, GITHUB_ACTIONS_APP_ID, sha)
    return bool(
        run and run.get("status") == "completed" and run.get("conclusion") == "success"
        and run.get("external_id") == override_id(number, sha)
    )


def record_override(api, actor, reason, pinned, details_url):
    """Allow an identified human with repository write access to waive Rabbit for this head."""
    if len(pinned[1]) != 1 or not reason.strip() or len(reason) > 2000:
        raise RuntimeError("An override needs one PR and a reason of 1–2000 characters")
    permission = api.repo_request("GET", f"/collaborators/{actor}/permission")
    if permission.get("permission") not in {"write", "maintain", "admin"} or permission.get("user", {}).get("type") != "User":
        raise RuntimeError("Only a human with repository write access may override a review")
    number, sha = pinned[1][0]
    api.repo_request("POST", "/check-runs", {
        "name": OVERRIDE_NAME, "head_sha": sha, "status": "completed", "conclusion": "success",
        "external_id": override_id(number, sha), "details_url": details_url,
        "output": {
            "title": f"Review overridden by @{actor}",
            "summary": f"PR #{number}, commit `{sha}`.\n\nOverride by @{actor}:\n\n{reason.strip()}",
        },
    })


def targets(api, event, event_name):
    """Pin current PR heads, or all queue entries through this cumulative merge head."""
    if event_name == "merge_group":
        group = event["merge_group"]
        branch = group["base_ref"].removeprefix("refs/heads/")
        entries = api.queue_entries(branch)
        matching = [e for e in entries if (e.get("headCommit") or {}).get("oid") == group["head_sha"]]
        if not matching:
            raise RuntimeError("Merge group changed or cannot be identified; no review accepted")
        # Batched entries can share one synthetic head. Every selected original
        # head must also pass the ancestry check before this group can succeed.
        last_position = max(e["position"] for e in matching)
        members = [e for e in entries if e["position"] <= last_position]
        pins = [(e["pullRequest"]["number"], e["pullRequest"]["headRefOid"]) for e in members]
        if not pins or len({number for number, _ in pins}) != len(pins):
            raise RuntimeError("Merge group has no unambiguous PR membership")
        return group["head_sha"], sorted(pins)
    if event_name == "pull_request_target":
        number = event["pull_request"]["number"]
    elif event_name == "workflow_dispatch":
        number = int(event["inputs"]["pull_request_number"])
    else:
        raise RuntimeError("Unsupported gate event")
    if number <= 0:
        raise RuntimeError("A positive PR number is required")
    pr = api.repo_request("GET", f"/pulls/{number}")
    if pr["state"] != "open" or pr["base"]["repo"]["full_name"] != api.repository:
        raise RuntimeError("Review gate requires an open PR in this repository")
    return pr["head"]["sha"], [(number, pr["head"]["sha"])]


def wait_for_reviews(api, event, event_name, pinned, clock=time.monotonic, sleep=time.sleep):
    """Wait at most 25 minutes for current reviews; terminal non-reviews block immediately."""
    deadline = clock() + WAIT_SECONDS
    while True:
        if targets(api, event, event_name) != pinned:
            raise RuntimeError("PR head or merge group changed while waiting for review")
        waiting = []
        for number, sha in pinned[1]:
            runs = api.checks(sha)
            if has_override(runs, number, sha):
                continue
            state = review_state(latest_check(runs, "CodeRabbit", CODERABBIT_APP_ID, sha))
            if state == "blocked":
                raise RuntimeError(
                    f"PR #{number}: latest CodeRabbit check is not a completed review. "
                    "A skipped, rate-limited, cancelled or failed review blocks merging. "
                    "Complete a new CodeRabbit review, then rerun this gate."
                )
            approval = api.latest_approval(number, sha) if state == "reviewed" else None
            if approval and approval.get("state") == "CHANGES_REQUESTED":
                raise RuntimeError(f"PR #{number}: CodeRabbit has requested changes")
            if state == "waiting" or not approval or approval.get("state") != "APPROVED":
                waiting.append(number)
        if not waiting:
            if event_name == "merge_group":
                # The current queue uses MERGE. Never accept a newer PR head that
                # the old synthetic merge commit does not actually contain.
                for number, sha in pinned[1]:
                    comparison = api.repo_request("GET", f"/compare/{sha}...{pinned[0]}")
                    if comparison["status"] not in {"ahead", "identical"}:
                        raise RuntimeError(f"PR #{number}: reviewed head is not in this merge group")
            # Recheck identities immediately before publishing success.
            if targets(api, event, event_name) != pinned:
                raise RuntimeError("PR head or merge group changed after review")
            return
        if clock() >= deadline:
            raise RuntimeError(f"Timed out waiting for CodeRabbit reviews on PRs {waiting}")
        print(f"Waiting for completed CodeRabbit reviews and approvals on PRs {waiting}", flush=True)
        sleep(min(POLL_SECONDS, max(0, deadline - clock())))


def run(api, event, event_name, details_url, actor="", wait=wait_for_reviews):
    """Publish a check on the actual head; any observed error produces failure, never neutral."""
    pinned = targets(api, event, event_name)
    check = api.repo_request("POST", "/check-runs", {
        "name": CHECK_NAME, "head_sha": pinned[0], "status": "in_progress",
        "details_url": details_url,
        "output": {"title": "Waiting for an actual review", "summary": "Checking current PR commits."},
    })
    try:
        reason = event.get("inputs", {}).get("override_reason", "").strip()
        if event_name == "workflow_dispatch" and reason:
            record_override(api, actor, reason, pinned, details_url)
        wait(api, event, event_name, pinned)
    except Exception as error:
        api.repo_request("PATCH", f"/check-runs/{check['id']}", {
            "status": "completed", "conclusion": "failure",
            "output": {"title": "CodeRabbit review required", "summary": str(error)},
        })
        raise
    api.repo_request("PATCH", f"/check-runs/{check['id']}", {
        "status": "completed", "conclusion": "success",
        "output": {
            "title": "CodeRabbit approved or a maintainer recorded an override",
            "summary": "Verified current commits: " + ", ".join(
                f"PR #{number} (`{sha}`)" for number, sha in pinned[1]
            ),
        },
    })


def main():
    """Read the trusted Actions context; errors leave a failing or missing required check."""
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    repository = os.environ["GITHUB_REPOSITORY"]
    api = GitHub(repository, os.environ["GITHUB_TOKEN"], os.environ["GITHUB_API_URL"])
    details_url = f"{os.environ['GITHUB_SERVER_URL']}/{repository}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    try:
        run(api, event, os.environ["GITHUB_EVENT_NAME"], details_url, os.environ["GITHUB_TRIGGERING_ACTOR"])
    except Exception as error:
        print(f"Review gate blocked: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
