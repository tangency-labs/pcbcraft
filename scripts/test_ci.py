"""CI policy regressions: conservative selection, real Git comparisons and strict completion."""
import copy
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from ci import check, policy as select, windows, window_smoke

ROOT = Path(__file__).resolve().parent.parent


class SelectionTests(unittest.TestCase):
    """Every omitted build is an explicit policy case; unknown inputs retain all coverage."""

    def test_documentation_and_issue_templates_need_no_build(self):
        for event in ("pull_request", "push"):
            selected = select.select(event, ["docs/editor-guide.md", "README.md", ".github/ISSUE_TEMPLATE/bug_report.yml"])
            self.assertFalse(any(selected.values()))

    def test_shared_engine_keeps_native_web_and_wasi(self):
        selected = select.select("pull_request", ["crates/engine/src/board/drc.rs"])
        self.assertTrue(selected["native"] and selected["web"] and selected["wasi"])
        self.assertFalse(selected["windows"])

    def test_native_integration_selects_windows_without_the_general_suite(self):
        for path in ("crates/storage/src/folder.rs", "crates/app/src/recent_projects.rs",
                     "crates/engine/src/persistence/documents.rs", "apps/pcbcraft/src/main.rs"):
            with self.subTest(path=path):
                selected = select.select("pull_request", [path])
                self.assertTrue(selected["windows"])
                self.assertFalse(selected["windows_full"])

    def test_configuration_and_unknown_paths_fail_closed(self):
        for path in ("Cargo.lock", "crates/app/Cargo.toml", "deny.toml", "scripts/ci/select.py",
                     "vendor/new-backend/file.rs", "docs/schemas/part.schema.json", "crates/new-crate/src/lib.rs",
                     "future-configuration.toml", "../README.md", r"docs\guide.md"):
            with self.subTest(path=path):
                self.assertEqual(select.select("pull_request", [path]), select.full())

    def test_full_events_missing_history_and_main_code_run_everything(self):
        for event in ("schedule", "workflow_dispatch", "merge_group", "unknown"):
            self.assertEqual(select.select(event, []), select.full())
        self.assertEqual(select.select("pull_request", None), select.full())
        self.assertEqual(select.select("push", ["crates/engine/src/board/drc.rs"]), select.full())

    def test_existing_windows_conditional_sources_select_windows(self):
        for directory in ("crates", "apps"):
            for source in (ROOT / directory).rglob("*.rs"):
                text = source.read_text(encoding="utf-8")
                if re.search(r"#\[cfg\([^]]*\bwindows\b", text):
                    with self.subTest(source=str(source.relative_to(ROOT))):
                        self.assertTrue(select.select("pull_request", [source.relative_to(ROOT).as_posix()])["windows"])

    def test_windows_app_conditional_modules_keep_selected_test_families(self):
        for source in (ROOT / "crates/app/src").rglob("*.rs"):
            if not re.search(r"#\[cfg\([^]]*\bwindows\b", source.read_text(encoding="utf-8")):
                continue
            module = "::".join(source.relative_to(ROOT / "crates/app/src").with_suffix("").parts) + "::"
            with self.subTest(module=module):
                self.assertTrue(any(f in module or f.startswith(module) for f in windows.APP_FILTERS))


class GitComparisonTests(unittest.TestCase):
    """Renames/deletions and unusual filenames use Git data, never shell-interpolated paths."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.git("init", "-q")
        (self.repo / "native file.rs").write_text("original\n", encoding="utf-8")
        self.commit()
        self.base = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        """Confine fixture identity and signing settings to this temporary Git repository."""
        return subprocess.check_output(["git", "-c", "user.name=travle1", "-c", "user.email=ci-fixture@example.invalid",
                                        "-c", "commit.gpgsign=false", *args], cwd=self.repo, text=True, stderr=subprocess.PIPE)

    def commit(self):
        """Write only local fixture history, with the workspace's required co-author trailer."""
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "CI comparison fixture\n\nCo-Authored-By: Codex <noreply@openai.com>")

    def test_rename_reports_both_paths_including_spaces(self):
        (self.repo / "native file.rs").rename(self.repo / "renamed file.rs")
        self.commit()
        head = self.git("rev-parse", "HEAD").strip()
        payload = {"pull_request": {"base": {"sha": self.base}, "head": {"sha": head}}}
        self.assertEqual(set(select.changed_paths("pull_request", payload, self.repo)), {"native file.rs", "renamed file.rs"})

    def test_missing_and_invalid_revisions_request_full_validation(self):
        for before in ("0" * 40, "f" * 40, "--output=/tmp/untrusted"):
            self.assertIsNone(select.changed_paths("push", {"before": before, "after": self.base}, self.repo))

    def test_workflow_entrypoint_runs_in_fresh_python_process(self):
        """Exercise direct script imports and Git pipes, including standard-library module lookup."""
        (self.repo / "README.md").write_text("documentation\n", encoding="utf-8")
        self.commit()
        payload = self.repo / "event.json"
        payload.write_text(json.dumps({"before": self.base, "after": self.git("rev-parse", "HEAD").strip()}), encoding="utf-8")
        output = self.repo / "outputs"
        env = {**os.environ, "GITHUB_EVENT_NAME": "push", "GITHUB_EVENT_PATH": str(payload), "GITHUB_OUTPUT": str(output)}
        result = subprocess.run([sys.executable, "-B", str(ROOT / "scripts/ci/policy.py")], cwd=self.repo, env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(set(output.read_text().splitlines()), {f"{key}=false" for key in select.OUTPUTS})


class CompletionTests(unittest.TestCase):
    """The required result cannot turn a cancelled, skipped or broken selected job green."""

    @staticmethod
    def needs(selected):
        return {**{name: {"result": "success"} for name in check.ALWAYS},
                "changes": {"result": "success", "outputs": {k: str(v).lower() for k, v in selected.items()}},
                **{name: {"result": "success" if selected[key] else "skipped"} for name, key in check.JOBS.items()}}

    def test_documentation_skips_are_successful(self):
        self.assertEqual(check.failures(self.needs(select.select("pull_request", ["docs/editor-guide.md"]))), [])

    def test_selected_job_cannot_be_skipped_cancelled_or_missing(self):
        original = self.needs(select.full())
        for name in check.JOBS:
            for status in ("skipped", "cancelled", "failure", None):
                needs = copy.deepcopy(original)
                needs[name]["result"] = status
                self.assertTrue(check.failures(needs), (name, status))

    def test_selector_and_light_checks_must_succeed(self):
        for name in check.ALWAYS:
            needs = self.needs(select.full())
            needs[name]["result"] = "failure"
            self.assertTrue(check.failures(needs))

    def test_outputs_must_be_complete_boolean_contract(self):
        for key in select.OUTPUTS:
            needs = self.needs(select.full())
            del needs["changes"]["outputs"][key]
            self.assertTrue(check.failures(needs), key)


class WindowsTests(unittest.TestCase):
    """Full validation remains available; the native smoke must actually run and finish."""

    def test_targeted_suite_retains_storage_transport_and_native_families(self):
        commands = windows.commands(False)
        joined = " ".join(" ".join(c) for c in commands)
        for expected in ("pcbcraft-storage", "pcbcraft-mcp", "craft-control", "persistence::", "project_files::", "recent_projects::", "clipboard::"):
            self.assertIn(expected, joined)
        self.assertNotIn("--workspace", joined)
        self.assertTrue(all("ci" in c for c in commands))

    def test_full_suite_retains_workspace_lints_tests_and_harness(self):
        commands = windows.commands(True)
        self.assertIn("--workspace", commands[0])
        self.assertIn("--workspace", commands[1])
        self.assertTrue(any("performance::" in c for c in commands))
        self.assertTrue(any("performance-harness" in c and "clippy" in c for c in commands))

    def test_smoke_rejects_zero_tests_and_nonzero_exit(self):
        for code, output in ((0, "test result: ok. 0 passed; 0 failed;"), (1, "test result: ok. 1 passed; 0 failed;")):
            result = subprocess.CompletedProcess([], code, output, "")
            with patch("builtins.print"), patch("ci.window_smoke.subprocess.run", return_value=result), self.assertRaises(RuntimeError):
                window_smoke.run(Path("fixture.exe"), 1)

    def test_smoke_rejects_stalls(self):
        with patch("ci.window_smoke.subprocess.run", side_effect=subprocess.TimeoutExpired([], 1)), self.assertRaises(RuntimeError):
            window_smoke.run(Path("fixture.exe"), 1)

    def test_smoke_accepts_only_completed_single_test(self):
        result = subprocess.CompletedProcess([], 0, "test result: ok. 1 passed; 0 failed; 0 ignored;", "")
        with patch("builtins.print"), patch("ci.window_smoke.subprocess.run", return_value=result):
            window_smoke.run(Path("fixture.exe"), 1)

    def test_artifact_staging_uses_cargo_path_and_preserves_symbols(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            executable = root / "with spaces" / "native_window_lifecycle-hash.exe"
            executable.parent.mkdir()
            executable.write_bytes(b"fixture executable")
            executable.with_suffix(".pdb").write_bytes(b"fixture symbols")
            artifact = {"reason": "compiler-artifact", "target": {"name": "native_window_lifecycle"}, "executable": str(executable)}
            result = subprocess.CompletedProcess([], 0, json.dumps(artifact) + "\n", "")
            destination = root / "staged"
            with patch("ci.windows.subprocess.run", return_value=result):
                windows.stage_lifecycle(destination)
            self.assertEqual((destination / "native_window_lifecycle.exe").read_bytes(), b"fixture executable")
            self.assertEqual((destination / executable.with_suffix(".pdb").name).read_bytes(), b"fixture symbols")
            self.assertTrue((destination / "SOLIDCORE-NOTICE").is_file())

    def test_artifact_staging_rejects_missing_or_ambiguous_executables(self):
        artifact = {"reason": "compiler-artifact", "target": {"name": "native_window_lifecycle"}, "executable": "fixture.exe"}
        for items in ([], [artifact, artifact]):
            result = subprocess.CompletedProcess([], 0, "\n".join(json.dumps(item) for item in items), "")
            with patch("ci.windows.subprocess.run", return_value=result), self.assertRaises(RuntimeError):
                windows.stage_lifecycle(Path("unused"))


if __name__ == "__main__":
    unittest.main()
