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

from ci import check, native, policy as select, qualification, windows, window_smoke

ROOT = Path(__file__).resolve().parent.parent


class SelectionTests(unittest.TestCase):
    """Every omitted build is an explicit policy case; unknown inputs retain all coverage."""

    def test_documentation_and_issue_templates_need_no_build(self):
        for event in ("pull_request", "push"):
            selected = select.select(event, ["docs/editor-guide.md", "README.md", ".github/ISSUE_TEMPLATE/bug_report.yml"])
            self.assertFalse(any(selected.values()))

    def test_code_selects_full_qualification_before_staging(self):
        for path in ("crates/engine/src/board/drc.rs", "crates/storage/src/folder.rs",
                     "crates/app/src/recent_projects.rs", "apps/pcbcraft/src/main.rs"):
            self.assertEqual(select.select("pull_request", [path]), select.full())

    def test_configuration_and_unknown_paths_fail_closed(self):
        for path in ("Cargo.lock", "crates/app/Cargo.toml", "deny.toml", "scripts/ci/select.py",
                     "vendor/new-backend/file.rs", "docs/schemas/part.schema.json", "crates/new-crate/src/lib.rs",
                     "future-configuration.toml", "../README.md", r"docs\guide.md",
                     "crates/geometry/tests/fixtures/step/new.step", "crates/app/assets/new.json"):
            with self.subTest(path=path):
                self.assertEqual(select.select("pull_request", [path]), select.full())

    def test_full_events_missing_history_and_main_code_run_everything(self):
        for event in ("schedule", "workflow_dispatch", "merge_group", "unknown"):
            self.assertEqual(select.select(event, []), select.full())
        self.assertEqual(select.select("pull_request", None), select.full())
        self.assertEqual(select.select("push", ["crates/engine/src/board/drc.rs"]), select.full())



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
        self.assertEqual(set(output.read_text().splitlines()), {f"{key}=false" for key in (*select.OUTPUTS, "quick")} | {"mode=docs"})


class CompletionTests(unittest.TestCase):
    """The required result cannot turn a cancelled, skipped or broken selected job green."""

    @staticmethod
    def needs(selected):
        if "mode" not in selected:
            selected = {**{key: str(value).lower() for key, value in selected.items()}, "quick": "false", "mode": "full" if any(selected.values()) else "docs"}
        return {**{name: {"result": "success"} for name in check.ALWAYS},
                "changes": {"result": "success", "outputs": selected},
                **{name: {"result": "success" if selected[key] == "true" else "skipped"} for name, key in check.JOBS.items()}}

    def test_documentation_skips_are_successful(self):
        self.assertEqual(check.failures(self.needs(select.select("pull_request", ["docs/editor-guide.md"]))), [])

    def test_selected_job_cannot_be_skipped_cancelled_or_missing(self):
        original = self.needs(select.full())
        for name, selection in check.JOBS.items():
            if original["changes"]["outputs"][selection] != "true":
                continue
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

    def test_quick_feedback_passes_without_requiring_full_qualification(self):
        selected = select.plan("pull_request", {}, ["Cargo.toml"])
        self.assertTrue(selected["quick"] == "true" and selected["mode"] == "quick")
        self.assertEqual(check.failures(self.needs(selected)), [])

    def test_qualification_cannot_skip_a_platform_or_lose_its_stage(self):
        needs = self.needs(select.full())
        needs["changes"]["outputs"]["windows"] = "false"
        self.assertTrue(check.failures(needs))
        needs = self.needs(select.full())
        del needs["changes"]["outputs"]["mode"]
        self.assertTrue(check.failures(needs))


class StageTests(unittest.TestCase):
    """Review approval gates test execution; unknown paths cannot bypass native approval."""

    def test_drafts_never_compile_even_with_approval(self):
        selected = select.plan("pull_request", {"pull_request": {"draft": True, "labels": [{"name": select.APPROVAL}]}}, None)
        self.assertEqual(selected["mode"], "draft")
        self.assertFalse(any(selected[key] == "true" for key in (*select.OUTPUTS, "quick")))

    def test_unapproved_code_config_and_unknown_paths_only_compile(self):
        for paths in (["crates/app/src/lib.rs"], ["Cargo.toml"], ["new.fixture"], None):
            selected = select.plan("pull_request", {}, paths)
            self.assertEqual(selected["mode"], "quick")
            self.assertEqual(selected["quick"], "true")
            self.assertFalse(any(selected[key] == "true" for key in select.OUTPUTS if key != "licenses"))

    def test_dependency_inputs_keep_license_checks_before_merge(self):
        for path in ("Cargo.lock", "Cargo.toml", "crates/app/Cargo.toml", "deny.toml"):
            for labels in ([], [{"name": select.APPROVAL}]):
                selected = select.plan("pull_request", {"pull_request": {"labels": labels}}, [path])
                self.assertEqual(selected["licenses"], "true")
        self.assertEqual(select.plan("pull_request", {}, ["crates/app/src/lib.rs"])["licenses"], "false")

    def test_approval_and_later_pushes_run_native_without_other_platforms(self):
        for action in ("labeled", "synchronize", "ready_for_review"):
            payload = {"action": action, "label": {"name": select.APPROVAL}, "pull_request": {"labels": [{"name": select.APPROVAL}]}}
            selected = select.plan("pull_request", payload, ["crates/engine/src/board.rs"])
            self.assertEqual(selected["mode"], "approved")
            self.assertEqual(selected["native"], "true")
            self.assertTrue(all(selected[key] == "false" for key in select.OUTPUTS if key != "native"))
            self.assertEqual(selected["quick"], "false")

    def test_unrelated_labels_carry_proof_and_approval_removal_returns_to_quick(self):
        payload = {"action": "labeled", "label": {"name": "enhancement"}, "pull_request": {"labels": [{"name": select.APPROVAL}]}}
        self.assertEqual(select.plan("pull_request", payload, ["Cargo.toml"])["mode"], "carry")
        payload = {"action": "unlabeled", "label": {"name": select.APPROVAL}, "pull_request": {"labels": []}}
        self.assertEqual(select.plan("pull_request", payload, ["Cargo.toml"])["mode"], "quick")

    def test_docs_need_no_approval_and_full_events_need_no_label(self):
        self.assertEqual(select.plan("pull_request", {}, ["README.md"])["mode"], "docs")
        for event in ("merge_group", "schedule", "workflow_dispatch", "push"):
            selected = select.plan(event, {}, ["Cargo.toml"])
            self.assertTrue(all(selected[key] == "true" for key in select.OUTPUTS))


class QualificationTests(unittest.TestCase):
    """A green quick run or another revision is never evidence of native approval."""

    def candidate(self, **updates):
        return {"id": 10, "head_sha": "a" * 40, "display_title": "CI: " + "b" * 40,
                "event": "pull_request", "status": "completed", "conclusion": "success", **updates}

    def proof(self, runs, jobs=None):
        if jobs is None:
            jobs = [{"name": name, "conclusion": "success"} for name in qualification.PR_JOBS]
        return qualification.passing_run(runs, "a" * 40, "b" * 40, 99, lambda _: jobs)

    def test_only_the_exact_head_and_merge_commit_can_carry(self):
        self.assertEqual(self.proof([self.candidate()]), (True, False))
        for updates in ({"head_sha": "c" * 40}, {"display_title": "CI: " + "c" * 40}, {"id": 99}, {"event": "push"}):
            self.assertEqual(self.proof([self.candidate(**updates)]), (False, False))

    def test_failed_missing_and_skipped_tests_cannot_carry(self):
        for name in qualification.PR_JOBS:
            for status in ("skipped", "failure", "cancelled", None):
                jobs = [{"name": job, "conclusion": status if job == name else "success"} for job in qualification.PR_JOBS]
                self.assertFalse(self.proof([self.candidate()], jobs)[0])
        self.assertEqual(self.proof([self.candidate()], []), (False, False))

    def test_waits_only_for_a_matching_running_candidate(self):
        self.assertEqual(self.proof([self.candidate(status="in_progress", conclusion=None)]), (False, True))
        self.assertEqual(self.proof([self.candidate(status="completed", conclusion="failure")]), (False, False))

    def test_revoked_or_stale_approval_is_rejected(self):
        env = {"GITHUB_REPOSITORY": "owner/repo", "CI_PR_NUMBER": "404", "CI_HEAD_SHA": "a" * 40}
        current = {"head": {"sha": "a" * 40}, "labels": [{"name": select.APPROVAL}], "draft": False}
        with patch.dict(os.environ, env), patch("ci.qualification.api", return_value=current):
            qualification.require_current_approval()
        for updates in ({"labels": []}, {"head": {"sha": "c" * 40}}, {"draft": True}):
            with patch.dict(os.environ, env), patch("ci.qualification.api", return_value={**current, **updates}), self.assertRaises(RuntimeError):
                qualification.require_current_approval()

    def test_waiter_stops_on_failure_or_deadline(self):
        env = {"GITHUB_REPOSITORY": "owner/repo", "CI_HEAD_SHA": "a" * 40, "GITHUB_SHA": "b" * 40, "GITHUB_RUN_ID": "99"}
        for pending in (False, True):
            with patch.dict(os.environ, env), patch("ci.qualification.require_current_approval"), patch("ci.qualification.api", return_value={"workflow_runs": []}), patch("ci.qualification.passing_run", return_value=(False, pending)), self.assertRaises(RuntimeError):
                qualification.wait_for_pass(timeout=0)

    def test_waiter_rechecks_approval_before_accepting_a_completed_run(self):
        env = {"GITHUB_REPOSITORY": "owner/repo", "CI_HEAD_SHA": "a" * 40, "GITHUB_SHA": "b" * 40, "GITHUB_RUN_ID": "99"}
        with patch.dict(os.environ, env), patch("ci.qualification.require_current_approval") as approval, patch("ci.qualification.api", return_value={"workflow_runs": []}), patch("ci.qualification.passing_run", side_effect=[(False, True), (True, False)]), patch("ci.qualification.time.sleep") as sleep, patch("builtins.print"):
            qualification.wait_for_pass()
        self.assertEqual(approval.call_count, 2)
        sleep.assert_called_once_with(20)


class NativeSelectionTests(unittest.TestCase):
    """Reverse dependency coverage includes consumers; unknown build inputs never select nothing."""

    @staticmethod
    def metadata():
        graph = {"units": [], "geometry": ["units"], "engine": ["geometry"],
                 "app": ["engine"], "web": ["app"], "cli": ["engine"], "unrelated": []}
        return {"workspace_root": "/workspace", "workspace_members": list(graph),
                "packages": [{"id": name, "name": name, "manifest_path": f"/workspace/crates/{name}/Cargo.toml",
                              "dependencies": [{"name": dep, "rename": "an_alias", "kind": "build", "target": "cfg(windows)"} for dep in deps]}
                             for name, deps in graph.items()]}

    def test_transitive_consumers_and_renamed_target_build_dependencies(self):
        self.assertEqual(native.affected(self.metadata(), ["crates/units/src/lib.rs"]), ["app", "cli", "engine", "geometry", "units", "web"])
        self.assertEqual(native.affected(self.metadata(), ["crates/app/src/lib.rs"]), ["app", "web"])
        self.assertEqual(native.affected(self.metadata(), ["crates/app/src/lib.rs", "crates/cli/src/main.rs"]), ["app", "cli", "web"])

    def test_unknown_deleted_fixture_and_configuration_inputs_select_all(self):
        for paths in (None, ["Cargo.lock"], ["crates/app/Cargo.toml"], ["crates/app/build.rs"],
                      ["crates/geometry/tests/fixtures/new.step"], ["crates/removed/src/lib.rs"],
                      ["/outside/source.rs"], ["crates/app/../../outside.rs"]):
            self.assertIsNone(native.affected(self.metadata(), paths), paths)

    def test_documentation_mixed_with_code_does_not_broaden_selection(self):
        self.assertEqual(native.affected(self.metadata(), ["README.md", "crates/app/src/lib.rs"]), ["app", "web"])

    def test_pr_commands_omit_doctests_and_opt_in_harness(self):
        commands = native.commands(["app", "web"], False)
        self.assertEqual(len(commands), 2)
        self.assertIn("--tests", commands[1])
        self.assertNotIn("--workspace", commands[0])
        self.assertIn("web", commands[1])
        self.assertFalse(any("performance-harness" in command for command in commands))

    def test_full_events_retain_workspace_doctests_and_harness(self):
        commands = native.commands(None, True)
        self.assertEqual(len(commands), 4)
        self.assertIn("--workspace", commands[1])
        self.assertNotIn("--tests", commands[1])
        self.assertIn("performance::", commands[3])
        with self.assertRaises(ValueError):
            native.commands([], False)

    def test_native_main_routes_approved_packages_and_propagates_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            event = Path(temp) / "event.json"
            event.write_text("{}", encoding="utf-8")
            env = {"CI_MODE": "approved", "GITHUB_EVENT_PATH": str(event)}
            with patch.dict(os.environ, env), patch("ci.native.subprocess.check_output", return_value=json.dumps(self.metadata())), patch("ci.native.changed_paths", return_value=["crates/app/src/lib.rs"]), patch("ci.native.subprocess.run") as run, patch("builtins.print"):
                native.main()
            self.assertEqual([call.args[0] for call in run.call_args_list], native.commands(["app", "web"], False))
            with patch.dict(os.environ, {"CI_MODE": "full"}), patch("ci.native.subprocess.check_output", return_value=json.dumps(self.metadata())), patch("ci.native.subprocess.run", side_effect=subprocess.CalledProcessError(1, ["cargo"])) as run, patch("builtins.print"), self.assertRaises(subprocess.CalledProcessError):
                native.main()
            self.assertEqual(run.call_count, 1)

    def test_invalid_native_stage_fails_before_running_cargo(self):
        with patch.dict(os.environ, {"CI_MODE": "quick"}), patch("ci.native.subprocess.check_output") as metadata, self.assertRaises(ValueError):
            native.main()
        metadata.assert_not_called()

    def test_api_failure_never_becomes_a_passing_result(self):
        env = {"GITHUB_REPOSITORY": "owner/repo", "CI_HEAD_SHA": "a" * 40, "GITHUB_SHA": "b" * 40, "GITHUB_RUN_ID": "99"}
        with patch.dict(os.environ, env), patch("ci.qualification.require_current_approval"), patch("ci.qualification.api", side_effect=OSError("disconnected")), self.assertRaises(OSError):
            qualification.wait_for_pass()


class WindowsTests(unittest.TestCase):
    """Full validation remains available; the native smoke must actually run and finish."""

    def test_full_suite_retains_workspace_lints_tests_and_harness(self):
        commands = windows.commands()
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
