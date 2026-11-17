"""Carry completed full checks across unrelated labels without launching another build."""
import json
import os
import re
import time
from urllib.request import Request, urlopen

FULL_JOBS = {"changes", "documentation-links", "library-schemas", "native-tests",
             "windows-tests", "windows-smoke", "web", "geometry-webassembly", "licenses", "checks"}


def api(path: str):
    """Use the read-only workflow token; network failures fail qualification closed."""
    request = Request(os.environ.get("GITHUB_API_URL", "https://api.github.com") + path,
                      headers={"Authorization": "Bearer " + os.environ["GH_TOKEN"],
                               "Accept": "application/vnd.github+json"})
    with urlopen(request, timeout=20) as response:
        return json.load(response)


def require_current_approval() -> None:
    """A stale or revoked approval cannot finish a PR's merge-readiness check."""
    repo, number = os.environ["GITHUB_REPOSITORY"], os.environ["CI_PR_NUMBER"]
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", repo) or not number.isdecimal():
        raise RuntimeError("Invalid repository or PR number")
    pr = api(f"/repos/{repo}/pulls/{number}")
    if pr.get("head", {}).get("sha") != os.environ["CI_HEAD_SHA"] or pr.get("draft"):
        raise RuntimeError("PR head or draft state changed; use the current run")
    if not any(label.get("name") == "approved-for-ci" for label in pr.get("labels", [])):
        raise RuntimeError("CI approval was removed; full qualification is required")


def passing_run(runs: list[dict], head: str, merge: str, current: int, jobs) -> tuple[bool, bool]:
    """Accept only a completed full run for this exact PR head and checked-out merge commit."""
    pending = False
    for run in runs:
        if (run.get("id") == current or run.get("head_sha") != head
                or run.get("display_title") != f"CI: {merge}"
                or run.get("event") != "pull_request"):
            continue
        if run.get("status") != "completed":
            pending = True
        elif run.get("conclusion") == "success":
            results = {job["name"]: job.get("conclusion") for job in jobs(run["id"])}
            if all(results.get(name) == "success" for name in FULL_JOBS):
                return True, pending
    return False, pending


def wait_for_pass(timeout: float = 3600) -> None:
    """Wait only for existing runs, never rebuild; cancellation releases the read-only waiter."""
    repo, head, merge = os.environ["GITHUB_REPOSITORY"], os.environ["CI_HEAD_SHA"], os.environ["GITHUB_SHA"]
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", repo) or not all(re.fullmatch(r"[0-9a-f]{40}", sha) for sha in (head, merge)):
        raise RuntimeError("Invalid qualification target")
    current, deadline = int(os.environ["GITHUB_RUN_ID"]), time.monotonic() + timeout
    while True:
        require_current_approval()
        runs = api(f"/repos/{repo}/actions/workflows/ci.yml/runs?head_sha={head}&per_page=100")["workflow_runs"]
        passed, pending = passing_run(runs, head, merge, current,
                                     lambda run: api(f"/repos/{repo}/actions/runs/{run}/jobs?filter=latest&per_page=100")["jobs"])
        if passed:
            print("The exact PR head and merge commit already passed every full check.")
            return
        if not pending or time.monotonic() >= deadline:
            raise RuntimeError("No full passing run for this merge commit; reapply approved-for-ci to run qualification")
        time.sleep(20)


if __name__ == "__main__":
    wait_for_pass()
