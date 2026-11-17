"""Execute the Ubuntu workflow's real baseline-selection block on divergent Git histories."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest


WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/ci.yml"
SNAPSHOT = Path("docs/schemas/command-contracts.json")


def comparison_script():
    """Extract the actual run block so the regression covers the workflow shipped to CI."""
    section = WORKFLOW.read_text().split("      - name: Compare command contracts with the base branch\n", 1)[1]
    section = section.split("\n      - ", 1)[0]
    return textwrap.dedent(section.split("        run: |\n", 1)[1])


@unittest.skipUnless(os.name == "posix" and shutil.which("bash") and shutil.which("git"), "Ubuntu Bash workflow regression")
class CommandContractWorkflowTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="pcbcraft-contract-workflow-")
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "-q", "-b", "target")
        self.git("config", "user.name", "Contract fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "core.autocrlf", "false")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.capture = self.root / "captured.json"
        self.runner_temp = self.root / "runner"
        self.runner_temp.mkdir()
        # Capture the baseline passed to the production comparator, without rebuilding Rust.
        cargo = self.bin / "cargo"
        cargo.write_text('#!/bin/sh\nwhile [ "$#" -gt 0 ] && [ "$1" != "--" ]; do shift; done\nshift\ncp "$1" "$CAPTURE_PATH"\nexit "${COMPARATOR_EXIT:-0}"\n')
        cargo.chmod(0o755)

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, text=True, stderr=subprocess.STDOUT).strip()

    def contract(self, version, required=False):
        params = {"type": "object", "properties": {"value": {"type": "string"}}, "additionalProperties": False}
        if required:
            params["required"] = ["value"]
        return {"command_api_version": version, "commands": {"test.command": {"params": params}}, "window_commands": {}}

    def write_snapshot(self, value):
        path = self.repo / SNAPSHOT
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, indent=2) + "\n")

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD")

    def history(self, ancestor_snapshot, target_snapshot):
        """Make a PR merge tree with a target that has advanced since the branch point."""
        (self.repo / "seed").write_text("fixture\n")
        if ancestor_snapshot is not None:
            self.write_snapshot(ancestor_snapshot)
        ancestor = self.commit("branch point")
        self.git("checkout", "-q", "-b", "pull-request")
        self.write_snapshot(self.contract(2, required=True))
        head = self.commit("PR tightens contract at version two")
        self.git("checkout", "-q", "target")
        if target_snapshot is not None:
            self.write_snapshot(target_snapshot)
        (self.repo / "target-change").write_text("target advanced\n")
        base = self.commit("current target")
        self.git("checkout", "-q", "pull-request")
        tree = self.git("write-tree")
        merged = self.git("commit-tree", tree, "-p", head, "-p", base, "-m", "synthetic checked-out PR merge result")
        self.git("checkout", "-q", "--detach", merged)
        self.assertEqual(self.git("merge-base", base, head), ancestor)
        return base, head

    def run_comparison(self, base, head, comparator_exit=0):
        env = {**os.environ, "BASE_SHA": base, "HEAD_SHA": head, "RUNNER_TEMP": str(self.runner_temp),
               "CAPTURE_PATH": str(self.capture), "COMPARATOR_EXIT": str(comparator_exit),
               "PATH": str(self.bin) + os.pathsep + os.environ["PATH"]}
        return subprocess.run(["bash", "-e", "-c", comparison_script()], cwd=self.repo, env=env, text=True, capture_output=True)

    def test_target_version_advance_is_the_baseline(self):
        target = self.contract(2)
        result = self.run_comparison(*self.history(self.contract(1), target))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(self.capture.read_text()), target)

    def test_first_target_snapshot_after_branch_point_is_checked(self):
        target = self.contract(2)
        result = self.run_comparison(*self.history(None, target))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.capture.exists(), "the current target has a snapshot even though the branch point does not")
        self.assertEqual(json.loads(self.capture.read_text()), target)

    def test_missing_target_snapshot_preserves_first_snapshot_fallback(self):
        result = self.run_comparison(*self.history(None, None))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.capture.exists())
        self.assertIn("No command contract snapshot", result.stdout)

    def test_comparator_failure_fails_the_workflow_step(self):
        result = self.run_comparison(*self.history(self.contract(1), self.contract(2)), comparator_exit=23)
        self.assertEqual(result.returncode, 23, result.stderr)


if __name__ == "__main__":
    unittest.main()
