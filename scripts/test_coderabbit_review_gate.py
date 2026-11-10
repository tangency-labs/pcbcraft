"""Exercise review, approval, override and queue failure paths with original API records."""

import copy
import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

SPEC = importlib.util.spec_from_file_location("gate", Path(__file__).with_name("coderabbit-review-gate.py"))
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)
SHA = "a" * 40
OTHER_SHA = "b" * 40
QUEUE_SHA = "c" * 40
EVENT = {"pull_request": {"number": 1}}
QUEUE_EVENT = {"merge_group": {"base_ref": "refs/heads/main", "head_sha": QUEUE_SHA}}


def reviewed(sha=SHA, **changes):
    """Build an original completed-review record, with optional failure variations."""
    result = {
        "id": 10, "name": "CodeRabbit", "app": {"id": gate.CODERABBIT_APP_ID},
        "head_sha": sha, "status": "completed", "conclusion": "success",
        "output": {"title": "Finished", "summary": "Result: Success - Review completed\nRun ID: test"},
    }
    result.update(changes)
    return result


def approval(sha=SHA, **changes):
    """Build a CodeRabbit approval bound to a specific PR commit."""
    result = {
        "id": 20, "user": {"id": gate.CODERABBIT_USER_ID},
        "state": "APPROVED", "commit_id": sha,
    }
    result.update(changes)
    return result


def entry(number, sha, position, head):
    """Build one queue entry whose temporary head contains its predecessors."""
    return {
        "position": position, "headCommit": {"oid": head},
        "pullRequest": {"number": number, "headRefOid": sha},
    }


class FakeGitHub(gate.GitHub):
    """Implement metadata/check endpoints locally, without tokens or network access."""

    def __init__(self):
        super().__init__("example/pcbcraft", "unused", "https://api.invalid")
        self.head = SHA
        self.runs = {SHA: [reviewed()]}
        self.reviews = {1: [approval()]}
        self.entries = []
        self.published = []
        self.permission = {"permission": "write", "user": {"type": "User"}}
        self.ancestry = "ahead"

    def repo_request(self, method, path, data=None):
        """Model pagination and check publication; unexpected endpoints fail the test."""
        parts = urlsplit(path)
        page = int(parse_qs(parts.query).get("page", [1])[0])
        if method == "GET" and parts.path == "/pulls/1":
            return {"state": "open", "base": {"repo": {"full_name": self.repository}}, "head": {"sha": self.head}}
        if method == "GET" and parts.path.endswith("/reviews"):
            number = int(parts.path.split("/")[2])
            return copy.deepcopy(self.reviews.get(number, [])[(page - 1) * 100:page * 100])
        if method == "GET" and parts.path.startswith("/commits/"):
            sha = parts.path.split("/")[2]
            return {"check_runs": copy.deepcopy(self.runs.get(sha, [])[(page - 1) * 100:page * 100])}
        if method == "GET" and parts.path.endswith("/permission"):
            return self.permission
        if method == "GET" and parts.path.startswith("/compare/"):
            return {"status": self.ancestry}
        if method == "POST" and parts.path == "/check-runs":
            result = {"id": 1000 + len(self.published), "app": {"id": gate.GITHUB_ACTIONS_APP_ID}, **copy.deepcopy(data)}
            self.published.append(result)
            self.runs.setdefault(result["head_sha"], []).append(result)
            return result
        if method == "PATCH" and parts.path.startswith("/check-runs/"):
            result = next(c for c in self.published if c["id"] == int(parts.path.split("/")[2]))
            result.update(copy.deepcopy(data))
            return result
        raise AssertionError(f"Unexpected endpoint: {method} {path}")

    def queue_entries(self, branch):
        """Return a snapshot of this fake queue for the expected branch."""
        if branch != "main":
            raise AssertionError("Wrong queue branch")
        return copy.deepcopy(self.entries)


class GateTests(unittest.TestCase):
    """Check that only reviews plus approvals, or explicit authorized overrides, pass."""

    def test_completed_review(self):
        self.assertEqual(gate.review_state(reviewed()), "reviewed")

    def test_rate_limit_skip_error_and_queue_success_are_not_reviews(self):
        for conclusion in ("neutral", "skipped", "failure", "cancelled", "timed_out"):
            with self.subTest(conclusion=conclusion):
                self.assertEqual(gate.review_state(reviewed(conclusion=conclusion)), "blocked")
        for title in ("Review rate limited", "Review skipped", "Pull request review passed", "Unknown"):
            with self.subTest(title=title):
                self.assertEqual(gate.review_state(reviewed(output={"title": title})), "blocked")

    def test_missing_or_running_review_waits(self):
        self.assertEqual(gate.review_state(None), "waiting")
        self.assertEqual(gate.review_state(reviewed(status="in_progress")), "waiting")

    def test_app_identity_head_and_newest_attempt(self):
        records = [reviewed(), reviewed(id=11, conclusion="neutral"), reviewed(id=12, app={"id": 1}), reviewed(OTHER_SHA, id=13)]
        run = gate.latest_check(records, "CodeRabbit", gate.CODERABBIT_APP_ID, SHA)
        self.assertEqual(run["id"], 11)
        self.assertEqual(gate.review_state(run), "blocked")

    def test_checks_and_reviews_paginate(self):
        api = FakeGitHub()
        api.runs[SHA] = [reviewed(id=n, app={"id": 1}) for n in range(100)] + [reviewed(id=101)]
        api.reviews[1] = [approval(id=n, user={"id": 1}) for n in range(100)] + [approval(id=101)]
        self.assertEqual(len(api.checks(SHA)), 101)
        self.assertEqual(api.latest_approval(1, SHA)["id"], 101)

    def test_old_approval_or_human_approval_does_not_satisfy_rabbit(self):
        api = FakeGitHub()
        api.reviews[1] = [approval(OTHER_SHA), approval(id=21, user={"id": 123})]
        self.assertIsNone(api.latest_approval(1, SHA))

    def test_dismissed_or_newer_negative_review_replaces_approval(self):
        api = FakeGitHub()
        for state in ("DISMISSED", "CHANGES_REQUESTED", "COMMENTED"):
            api.reviews[1] = [approval(), approval(id=21, state=state)]
            self.assertEqual(api.latest_approval(1, SHA)["state"], state)

    def test_successful_review_and_approval_publish_success(self):
        api = FakeGitHub()
        gate.run(api, EVENT, "pull_request_target", "https://example.invalid/run")
        self.assertEqual(api.published[-1]["head_sha"], SHA)
        self.assertEqual(api.published[-1]["conclusion"], "success")

    def test_rate_limit_and_requested_changes_publish_failure(self):
        for mode in ("rate_limit", "findings"):
            api = FakeGitHub()
            if mode == "rate_limit":
                api.runs[SHA] = [reviewed(conclusion="neutral")]
            else:
                api.reviews[1] = [approval(state="CHANGES_REQUESTED")]
            with self.subTest(mode=mode), self.assertRaises(RuntimeError):
                gate.run(api, EVENT, "pull_request_target", "https://example.invalid/run")
            self.assertEqual(api.published[-1]["conclusion"], "failure")

    def test_missing_review_timeout(self):
        api = FakeGitHub()
        api.runs[SHA] = []
        with self.assertRaisesRegex(RuntimeError, "Timed out"):
            gate.wait_for_reviews(api, EVENT, "pull_request_target", (SHA, [(1, SHA)]), clock=iter([0, gate.WAIT_SECONDS]).__next__)

    def test_head_change_blocks(self):
        api = FakeGitHub()
        api.head = OTHER_SHA
        with self.assertRaisesRegex(RuntimeError, "changed"):
            gate.wait_for_reviews(api, EVENT, "pull_request_target", (SHA, [(1, SHA)]))

    def test_api_failure_publishes_failure(self):
        api = FakeGitHub()
        with patch.object(api, "checks", side_effect=TimeoutError("API request timed out")):
            with self.assertRaises(TimeoutError):
                gate.run(api, EVENT, "pull_request_target", "https://example.invalid/run")
        self.assertEqual(api.published[-1]["conclusion"], "failure")

    def test_queue_checks_every_predecessor_head(self):
        api = FakeGitHub()
        api.entries = [entry(1, SHA, 1, "earlier"), entry(2, OTHER_SHA, 2, QUEUE_SHA)]
        api.runs[OTHER_SHA] = [reviewed(OTHER_SHA, conclusion="neutral")]
        self.assertEqual(gate.targets(api, QUEUE_EVENT, "merge_group"), (QUEUE_SHA, [(1, SHA), (2, OTHER_SHA)]))
        with self.assertRaises(RuntimeError):
            gate.run(api, QUEUE_EVENT, "merge_group", "https://example.invalid/run")
        self.assertEqual(api.published[-1]["head_sha"], QUEUE_SHA)
        self.assertEqual(api.published[-1]["conclusion"], "failure")

    def test_queue_success_is_based_on_original_pr_reviews(self):
        api = FakeGitHub()
        api.entries = [entry(1, SHA, 1, QUEUE_SHA)]
        api.runs[QUEUE_SHA] = [reviewed(QUEUE_SHA, output={"title": "Pull request review passed"})]
        gate.run(api, QUEUE_EVENT, "merge_group", "https://example.invalid/run")
        self.assertEqual(api.published[-1]["conclusion"], "success")

    def test_batched_entries_can_share_a_synthetic_head(self):
        api = FakeGitHub()
        api.entries = [entry(1, SHA, 1, QUEUE_SHA), entry(2, OTHER_SHA, 2, QUEUE_SHA)]
        api.runs[OTHER_SHA] = [reviewed(OTHER_SHA)]
        api.reviews[2] = [approval(OTHER_SHA)]
        gate.run(api, QUEUE_EVENT, "merge_group", "https://example.invalid/run")
        self.assertEqual(api.published[-1]["conclusion"], "success")

    def test_changed_or_unknown_queue_blocks(self):
        api = FakeGitHub()
        with self.assertRaisesRegex(RuntimeError, "cannot be identified"):
            gate.targets(api, QUEUE_EVENT, "merge_group")

    def test_queue_rejects_a_new_pr_head_not_in_the_merge_commit(self):
        api = FakeGitHub()
        api.entries = [entry(1, SHA, 1, QUEUE_SHA)]
        api.ancestry = "diverged"
        with self.assertRaisesRegex(RuntimeError, "not in this merge group"):
            gate.run(api, QUEUE_EVENT, "merge_group", "https://example.invalid/run")
        self.assertEqual(api.published[-1]["conclusion"], "failure")

    def test_explicit_writer_override_is_audited_and_queue_honors_it(self):
        api = FakeGitHub()
        api.runs[SHA] = [reviewed(conclusion="failure")]
        event = {"inputs": {"pull_request_number": "1", "override_reason": "False positive verified manually"}}
        gate.run(api, event, "workflow_dispatch", "https://example.invalid/run", actor="maintainer")
        override = next(c for c in api.published if c["name"] == gate.OVERRIDE_NAME)
        self.assertIn("@maintainer", override["output"]["summary"])
        self.assertIn("False positive", override["output"]["summary"])
        api.entries = [entry(1, SHA, 1, QUEUE_SHA)]
        gate.run(api, QUEUE_EVENT, "merge_group", "https://example.invalid/run")
        self.assertEqual(api.published[-1]["conclusion"], "success")

    def test_reader_or_bot_cannot_override(self):
        for permission in ({"permission": "read", "user": {"type": "User"}}, {"permission": "write", "user": {"type": "Bot"}}):
            api = FakeGitHub()
            api.permission = permission
            with self.assertRaisesRegex(RuntimeError, "Only a human"):
                gate.record_override(api, "actor", "Reason", (SHA, [(1, SHA)]), "https://example.invalid/run")
            self.assertEqual(api.published, [])

    def test_override_requires_reason_and_is_bound_to_pr_commit_and_app(self):
        api = FakeGitHub()
        with self.assertRaises(RuntimeError):
            gate.record_override(api, "maintainer", " ", (SHA, [(1, SHA)]), "https://example.invalid/run")
        gate.record_override(api, "maintainer", "Reason", (SHA, [(1, SHA)]), "https://example.invalid/run")
        records = api.checks(SHA)
        self.assertTrue(gate.has_override(records, 1, SHA))
        self.assertFalse(gate.has_override(records, 2, SHA))
        self.assertFalse(gate.has_override(records, 1, OTHER_SHA))
        gate.record_override(api, "maintainer", "Separate PR reason", (SHA, [(2, SHA)]), "https://example.invalid/run")
        self.assertTrue(gate.has_override(api.checks(SHA), 1, SHA))
        records[-1]["app"]["id"] = 1
        self.assertFalse(gate.has_override(records, 1, SHA))


if __name__ == "__main__":
    unittest.main()
