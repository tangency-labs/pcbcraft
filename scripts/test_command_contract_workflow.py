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


def baseline_script():
    """Extract the actual run block so the regression covers the workflow shipped to CI."""
    section = WORKFLOW.read_text().split("      - name: Read command contract baseline\n", 1)[1]
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
        self.runner_temp = self.root / "runner"
        self.runner_temp.mkdir()
        self.environment = self.runner_temp / "env"
        self.baseline = self.runner_temp / "base-command-contracts.json"

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

    def history(self, ancestor_snapshot, target_snapshot, head_snapshot=None):
        """Make a PR merge tree with a target that has advanced since the branch point."""
        (self.repo / "seed").write_text("fixture\n")
        if ancestor_snapshot is not None:
            self.write_snapshot(ancestor_snapshot)
        ancestor = self.commit("branch point")
        self.git("checkout", "-q", "-b", "pull-request")
        self.write_snapshot(head_snapshot if head_snapshot is not None else self.contract(2, required=True))
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
        return base

    def read_baseline(self, base):
        env = {**os.environ, "BASE_SHA": base, "RUNNER_TEMP": str(self.runner_temp), "GITHUB_ENV": str(self.environment)}
        return subprocess.run(["bash", "-e", "-c", baseline_script()], cwd=self.repo, env=env, text=True, capture_output=True)

    def test_target_version_advance_is_the_baseline(self):
        target = self.contract(2)
        result = self.read_baseline(self.history(self.contract(1), target))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(self.baseline.read_text()), target)
        self.assertEqual(self.environment.read_text(), f"PCBCRAFT_COMMAND_CONTRACT_BASE={self.baseline}\n")

    def test_first_target_snapshot_after_branch_point_is_checked(self):
        target = self.contract(2)
        result = self.read_baseline(self.history(None, target))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.environment.exists(), "the current target has a snapshot even though the branch point does not")
        self.assertEqual(json.loads(self.baseline.read_text()), target)

    def test_missing_target_snapshot_preserves_first_snapshot_fallback(self):
        result = self.read_baseline(self.history(None, None))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.environment.exists())
        self.assertIn("No command contract snapshot", result.stdout)

    def test_unchanged_snapshot_does_not_require_additional_package_tests(self):
        target = self.contract(2)
        result = self.read_baseline(self.history(self.contract(1), target, head_snapshot=target))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.environment.exists())

    def test_missing_base_commit_fails_instead_of_skipping(self):
        result = self.read_baseline("a" * 40)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.environment.exists())


if __name__ == "__main__":
    unittest.main()
