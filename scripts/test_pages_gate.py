"""Check the full CI gates for Pages and nightly packages."""
import ast
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/ci.yml"


def jobs():
    """Read this workflow's top-level job blocks without adding a YAML dependency to CI."""
    source = WORKFLOW.read_text(encoding="utf-8").split("\njobs:\n", 1)[1]
    starts = list(re.finditer(r"^  ([A-Za-z_][A-Za-z0-9_-]*):\s*$", source, re.MULTILINE))
    return {match[1]: source[match.end():starts[index + 1].start() if index + 1 < len(starts) else len(source)]
            for index, match in enumerate(starts)}


def needs(block):
    """Read the inline dependency list used by the aggregate and deployment jobs."""
    match = re.search(r"^    needs: \[([^\]]*)\]$", block, re.MULTILINE)
    return {item.strip() for item in match[1].split(",")} if match else set()


def job_allowed(job, *, ancestor_results=(), cancelled=False, **changes):
    """Evaluate a job guard, including Actions' implicit ancestor success check.

    This bounded evaluator supports the workflow's boolean/comparison grammar and
    status functions; it does not simulate the hosted scheduler or deployment.
    """
    values = {"needs.changes.result": "success", "needs.checks.result": "success",
              "needs.changes.outputs.mode": "full",
              "needs.pages-build.result": "success",
              "github.ref": "refs/heads/main", "github.event_name": "push",
              "github.event.repository.private": False, **changes}
    block = jobs()[job]
    ancestor_results = (*ancestor_results, *(values[f"needs.{name}.result"] for name in needs(block)))
    match = re.search(r"^    if: \$\{\{ (.*?) \}\}$", block, re.MULTILINE)
    expression = match[1] if match else "True"
    status = {"always": True, "cancelled": cancelled,
              "success": not cancelled and all(result == "success" for result in ancestor_results),
              "failure": "failure" in ancestor_results}
    if not re.search(r"\b(?:always|cancelled|success|failure)\(\)", expression):
        if not status["success"]:
            return False
    expression = re.sub(r"\b(always|cancelled|success|failure)\(\)",
                        lambda match: repr(status[match[1]]), expression)
    expression = re.sub(r"\b(?:needs|github)\.[\w.-]+", lambda match: repr(values[match[0]]), expression)
    expression = expression.replace("&&", " and ").replace("||", " or ")
    expression = re.sub(r"!(?!=)", "not ", expression)
    tree = ast.parse(expression, mode="eval")
    allowed = (ast.Expression, ast.BoolOp, ast.And, ast.Or, ast.UnaryOp, ast.Not,
               ast.Compare, ast.Eq, ast.NotEq, ast.Constant)
    if any(not isinstance(node, allowed) for node in ast.walk(tree)):
        raise ValueError("Unsupported Pages guard expression")
    return eval(compile(tree, str(WORKFLOW), "eval"), {"__builtins__": {}}, {})


def build_allowed(**changes):
    """Evaluate the build guard for the public-main publication policy cases."""
    return job_allowed("pages-build", **changes)


class PagesGateTests(unittest.TestCase):
    """A failed/skipped stage or a different revision cannot reach deployment."""

    def test_all_validation_jobs_feed_the_gate_without_a_deployment_cycle(self):
        graph = jobs()
        validation = set(graph) - {"checks", "nightly-packages", "pages-build", "pages-deploy"}
        self.assertEqual(needs(graph["checks"]), validation)
        self.assertEqual(needs(graph["nightly-packages"]), {"changes", "checks"})
        self.assertEqual(needs(graph["pages-build"]), {"changes", "checks"})
        self.assertEqual(needs(graph["pages-deploy"]), {"checks", "pages-build"})

    def test_intentional_upstream_skip_does_not_suppress_qualified_pages_jobs(self):
        # Full main CI skips quick-compile, then the aggregate checks job succeeds.
        for job in ("pages-build", "pages-deploy"):
            with self.subTest(job=job):
                self.assertTrue(job_allowed(job, ancestor_results=("success", "skipped", "success")))

    def test_failed_or_unfinished_dependencies_and_cancellation_cannot_publish(self):
        for job in ("pages-build", "pages-deploy"):
            for result in ("failure", "cancelled", "skipped", "in_progress", "", None):
                with self.subTest(job=job, checks=result):
                    self.assertFalse(job_allowed(job, ancestor_results=("skipped",),
                                                 **{"needs.checks.result": result}))
            with self.subTest(job=job, cancelled=True):
                self.assertFalse(job_allowed(job, cancelled=True))
        for result in ("failure", "cancelled", "skipped", "in_progress", "", None):
            with self.subTest(build=result):
                self.assertFalse(job_allowed("pages-deploy", **{"needs.pages-build.result": result}))

    def test_only_successful_full_main_pushes_and_manual_runs_can_publish(self):
        self.assertTrue(build_allowed())
        self.assertTrue(build_allowed(**{"github.event_name": "workflow_dispatch"}))
        for result in ("failure", "cancelled", "skipped", "in_progress", "", None):
            with self.subTest(result=result):
                self.assertFalse(build_allowed(**{"needs.checks.result": result}))
        for mode in ("docs", "quick", "approved", "carry", "draft", "", None):
            with self.subTest(mode=mode):
                self.assertFalse(build_allowed(**{"needs.changes.outputs.mode": mode}))
        for event in ("pull_request", "pull_request_target", "schedule", "merge_group", "workflow_run"):
            self.assertFalse(build_allowed(**{"github.event_name": event}))
        self.assertFalse(build_allowed(**{"github.ref": "refs/heads/topic"}))
        self.assertFalse(build_allowed(**{"github.event.repository.private": True}))

    def test_build_uses_tested_commit_and_no_independent_workflow_can_deploy(self):
        self.assertIn("ref: ${{ github.sha }}", jobs()["pages-build"])
        self.assertFalse((WORKFLOW.parent / "pages.yml").exists())
        for workflow in WORKFLOW.parent.glob("*.yml"):
            if workflow != WORKFLOW:
                self.assertNotIn("actions/deploy-pages@", workflow.read_text(encoding="utf-8"))

    def test_nightlies_require_full_main_ci(self):
        for event in ("push", "schedule", "workflow_dispatch"):
            with self.subTest(event=event):
                self.assertTrue(job_allowed("nightly-packages", ancestor_results=("skipped",),
                                            **{"github.event_name": event}))
        for changes in (
            {"github.ref": "refs/heads/topic"},
            {"github.event_name": "pull_request"},
            {"github.event_name": "merge_group"},
            {"needs.changes.outputs.mode": "docs"},
            {"needs.checks.result": "failure"},
            {"needs.checks.result": "skipped"},
            {"cancelled": True},
        ):
            with self.subTest(changes=changes):
                self.assertFalse(job_allowed("nightly-packages", **changes))

    def revision_check(self, current, exit_code=0):
        """Run the real final-revision shell check against a local API stub, never GitHub."""
        deploy = jobs()["pages-deploy"]
        script = textwrap.dedent(deploy.split("        run: |\n", 1)[1].split("\n      - ", 1)[0])
        with tempfile.TemporaryDirectory(prefix="pcbcraft-pages-gate-") as temporary:
            directory = Path(temporary)
            stub = directory / "gh"
            stub.write_text(f"#!/bin/sh\nprintf '%s\\n' '{current}'\nexit {exit_code}\n", encoding="utf-8")
            stub.chmod(0o700)
            output = directory / "outputs"
            environment = {**os.environ, "PATH": str(directory) + os.pathsep + os.defpath,
                           "GITHUB_REPOSITORY": "example/pcbcraft", "GITHUB_OUTPUT": str(output),
                           "PAGES_DEPLOY_SHA": "a" * 40}
            result = subprocess.run(["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", script],
                                    env=environment, capture_output=True, text=True)
            return result, output.read_text(encoding="utf-8") if output.exists() else ""

    @unittest.skipIf(os.name == "nt", "the deployment check runs on Ubuntu's bash")
    def test_superseded_commits_skip_and_api_failures_cannot_publish(self):
        result, output = self.revision_check("a" * 40)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "current=true\n")
        result, output = self.revision_check("b" * 40)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "current=false\n")
        result, output = self.revision_check("a" * 40, exit_code=1)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("current=true", output)
        self.assertIn("if: steps.revision.outputs.current == 'true'", jobs()["pages-deploy"])


if __name__ == "__main__":
    unittest.main()
