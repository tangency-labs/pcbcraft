"""Cross-platform performance regression gate tests without wall-clock timing assertions.

The fixtures follow the report `board_perf budgets` writes: the action table is read from
`BUDGETS` in `crates/app/src/performance/budget.rs`. A renamed or reordered action there fails
these tests; a renamed boundary is followed by the fixture, so only the boundary names the tests
spell out (`encode.packed`, `drc.synchronous`, `command.move_tracks`) fail when renamed.
"""
import copy
import importlib.util
import json
import math
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
SCRIPT_PATH = SCRIPTS / "check-performance-regression.py"
PERFORMANCE_SRC = SCRIPTS.parent / "crates" / "app" / "src" / "performance"
spec = importlib.util.spec_from_file_location("check_performance_regression", SCRIPT_PATH)
gate_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate_mod)

# Route pointer moves that reverse a route in one iteration of the reference-v1 trace; the
# Rust suite pins this count (`budget_suite_reports_each_action_against_its_budget`).
REVERSALS_PER_ITERATION = 6
ITERATIONS = 2
# Other replay boundaries every reference-v1 report carries, one sample per iteration.
OTHER_METRICS = ("replay.start.up.ui", "replay.layer.down.ui", "replay.cancel.up.ui", "replay.shared-edit.command")


def _rust_sources():
    paths = {
        "budget.rs": PERFORMANCE_SRC / "budget.rs",
        "replay.rs": PERFORMANCE_SRC / "replay.rs",
        "performance.rs": PERFORMANCE_SRC.parent / "performance.rs",
    }
    return {name: path.read_text(encoding="utf-8") for name, path in paths.items()}


def _string_constants(sources):
    constants = {}
    for text in sources.values():
        for name, value in re.findall(r'const ([A-Z_]+): &str = "([^"]*)";', text):
            constants[name] = value
    return constants


def _rust_literal(token, constants):
    token = token.strip()
    if token.startswith('"'):
        return token.strip('"')
    return constants[token]


def _rust_budget_table():
    """Return (action, boundary, budget_p95_ms, basis) for every row of the Rust `BUDGETS`."""
    sources = _rust_sources()
    constants = _string_constants(sources)
    block = re.search(r"pub const BUDGETS: &\[Budget\] = &\[(.*?)\n\];", sources["budget.rs"], re.S).group(1)
    rows = []
    for action, boundary, budget, basis in re.findall(
        r'action: "([^"]+)",\s*boundary: Boundary::Metric\(([^)]+)\),\s*p95_ms: (None|Some\([0-9.]+\)),\s*basis: ("[^"]*"|[A-Z_]+)\s*[,}]',
        block,
    ):
        limit = None if budget == "None" else float(budget[5:-1])
        rows.append((action, _rust_literal(boundary, constants), limit, _rust_literal(basis, constants)))
    return rows, constants


def _rust_unmeasured():
    """Return the `unmeasured` list a budget report carries: the CPU report's, then the suite's."""
    sources = _rust_sources()
    cpu = re.search(r"unmeasured: \[(.*?)\]", sources["performance.rs"], re.S).group(1)
    suite = re.search(r"const UNMEASURED: &\[&str\] = &\[(.*?)\];", sources["budget.rs"], re.S).group(1)
    return re.findall(r'"([^"]*)"', cpu) + re.findall(r'"([^"]*)"', suite)


BUDGET_TABLE, RUST_CONSTANTS = _rust_budget_table()
ROUTE_REVERSAL_METRIC = RUST_CONSTANTS["ROUTE_REVERSAL_METRIC"]
UNMEASURED = _rust_unmeasured()


def _metric(metric_id, samples):
    ordered = sorted(samples)
    quantile = lambda pct: ordered[math.ceil(len(ordered) * pct / 100) - 1]
    return {
        "id": metric_id,
        "samples_ms": list(samples),
        "p50_ms": quantile(50),
        "p95_ms": quantile(95),
        "p99_ms": quantile(99),
        "max_ms": ordered[-1],
    }


def _action(action, boundary, metrics, budget_p95_ms, basis):
    samples = [s for m in metrics if m["id"] == boundary for s in m["samples_ms"]]
    row = {
        "action": action,
        "boundary": boundary,
        "samples": len(samples),
        "p50_ms": None,
        "p95_ms": None,
        "budget_p95_ms": budget_p95_ms,
        "budget_basis": basis,
        "verdict": "unmeasured",
    }
    if samples:
        ordered = sorted(samples)
        row["p50_ms"] = ordered[math.ceil(len(ordered) * 50 / 100) - 1]
        row["p95_ms"] = ordered[math.ceil(len(ordered) * 95 / 100) - 1]
        if budget_p95_ms is None:
            row["verdict"] = "no_budget"
        elif row["p95_ms"] < budget_p95_ms:
            row["verdict"] = "measured_pass"
        else:
            row["verdict"] = "measured_miss"
    return row


def _budget_report(revision="base", os_name="linux", scale=None, reversals=REVERSALS_PER_ITERATION, samples=None):
    """Build a report in the shape `board_perf budgets` writes for reference-v1.

    Every metric has one sample per iteration except the route reversal aggregate, which has
    one per reversing pointer move; `reversals=0` drops it, as a trace without a reversal does.
    """
    scale = scale or {}
    samples = samples or {}
    raw = {}
    for index, (_, boundary, _, _) in enumerate(BUDGET_TABLE):
        if boundary == ROUTE_REVERSAL_METRIC:
            if reversals:
                raw[boundary] = [0.5 + 0.1 * i for i in range(reversals * ITERATIONS)]
        else:
            raw[boundary] = samples.get(boundary, [10.0 + 5.0 * index + i for i in range(ITERATIONS)])
    for index, metric_id in enumerate(OTHER_METRICS):
        raw[metric_id] = [1.0 + index + 0.25 * i for i in range(ITERATIONS)]
    metrics = [_metric(metric_id, [s * scale.get(metric_id, 1.0) for s in values]) for metric_id, values in raw.items()]
    actions = [_action(action, boundary, metrics, limit, basis) for action, boundary, limit, basis in BUDGET_TABLE]
    return {
        "schema": "pcbcraft-performance-budgets/1",
        "environment": {
            "revision": revision,
            "hardware": "reference-cpu",
            "os": os_name,
            "runtime": "native",
            "compiler": "rustc 1.100.0",
            "profile": "release",
            "power": "AC",
            "notes": "",
        },
        "target": "native-cpu",
        "debug_assertions": False,
        "artifact_sha256": "a" * 64,
        "project_sha256": "b" * 64,
        "recipe": "reference-v1",
        "iterations": ITERATIONS,
        "geometry": {"tracks": 60, "vias": 0},
        "actions": actions,
        "metrics": metrics,
        "unmeasured": list(UNMEASURED),
    }


def _run_cli(*args):
    return subprocess.run([sys.executable, "-B", str(SCRIPT_PATH), *map(str, args)], capture_output=True, text=True)


class RustShapeTests(unittest.TestCase):
    def test_fixture_follows_the_rust_budget_table(self):
        actions = [row[0] for row in BUDGET_TABLE]
        self.assertEqual(actions, ["open", "save", "drc", "connectivity", "move_commit", "routing_input"])
        boundaries = dict((row[0], row[1]) for row in BUDGET_TABLE)
        self.assertEqual(boundaries["routing_input"], ROUTE_REVERSAL_METRIC)
        self.assertEqual(boundaries["save"], "encode.packed")
        self.assertTrue(any(item.startswith("window routing input") for item in UNMEASURED))

        report = _budget_report()
        counts = {metric["id"]: len(metric["samples_ms"]) for metric in report["metrics"]}
        self.assertEqual(counts.pop(ROUTE_REVERSAL_METRIC), REVERSALS_PER_ITERATION * ITERATIONS)
        self.assertEqual(set(counts.values()), {ITERATIONS})
        routing = next(row for row in report["actions"] if row["action"] == "routing_input")
        self.assertEqual(routing["samples"], 12)


class RegressionGateTests(unittest.TestCase):
    def test_identical_reports_in_the_suite_shape_pass_with_exit_0(self):
        with tempfile.TemporaryDirectory() as tmp:
            base_path = Path(tmp) / "base.json"
            cand_path = Path(tmp) / "cand.json"
            base_path.write_text(json.dumps(_budget_report(revision="base")), encoding="utf-8")
            cand_path.write_text(json.dumps(_budget_report(revision="cand")), encoding="utf-8")
            result = _run_cli("--json", base_path, cand_path)
            self.assertEqual(result.returncode, 0, result.stderr)
            payload = json.loads(result.stdout)
            self.assertTrue(payload["passed"])
            self.assertEqual([row["action"] for row in payload["actions"]], [row[0] for row in BUDGET_TABLE])
            self.assertEqual(payload["not_compared"], [])

    def test_passes_without_slowdown_and_fails_on_deliberate_30_percent_delay(self):
        for os_name in ("linux", "windows", "macos"):
            with self.subTest(os=os_name):
                baseline = _budget_report(revision="base", os_name=os_name)
                clean = gate_mod.gate(baseline, _budget_report(revision="cand", os_name=os_name))
                self.assertTrue(clean["passed"])
                self.assertEqual(clean["regressed_actions"], [])

                slowed = _budget_report(revision="cand-slow", os_name=os_name, scale={"drc.synchronous": 1.30})
                failed = gate_mod.gate(baseline, slowed)
                self.assertFalse(failed["passed"])
                self.assertEqual(failed["regressed_actions"], ["drc"])
                rendered = gate_mod.format_report(failed)
                self.assertIn("FAIL (drc)", rendered)
                self.assertIn("Regressed actions: drc", rendered)
                self.assertIn("+30.0%", rendered)

    def test_route_reversal_30_percent_slowdown_fails_with_exit_1_and_names_routing_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            base_path = Path(tmp) / "base.json"
            slow_path = Path(tmp) / "slow.json"
            base_path.write_text(json.dumps(_budget_report(revision="base")), encoding="utf-8")
            slow_path.write_text(json.dumps(_budget_report(revision="slow", scale={ROUTE_REVERSAL_METRIC: 1.30})), encoding="utf-8")
            result = _run_cli("--json", base_path, slow_path)
            self.assertEqual(result.returncode, 1, result.stderr)
            payload = json.loads(result.stdout)
            self.assertEqual(payload["regressed_actions"], ["routing_input"])

    def test_actions_unmeasured_in_both_reports_are_listed_and_not_compared(self):
        baseline = _budget_report(revision="base", reversals=0)
        routing = next(row for row in baseline["actions"] if row["action"] == "routing_input")
        self.assertEqual(routing["verdict"], "unmeasured")

        clean = gate_mod.gate(baseline, _budget_report(revision="cand", reversals=0))
        self.assertTrue(clean["passed"])
        self.assertEqual(clean["not_compared"], ["routing_input"])
        self.assertNotIn("routing_input", [row["action"] for row in clean["actions"]])
        self.assertIn("Not compared (unmeasured in both reports): routing_input", gate_mod.format_report(clean))

        slowed = gate_mod.gate(baseline, _budget_report(revision="slow", reversals=0, scale={"command.move_tracks": 1.30}))
        self.assertFalse(slowed["passed"])
        self.assertEqual(slowed["regressed_actions"], ["move_commit"])

    def test_refuses_an_action_measured_in_only_one_report_or_with_other_sample_counts(self):
        measured = _budget_report(revision="base")
        unmeasured = _budget_report(revision="idle", reversals=0)
        with self.assertRaisesRegex(ValueError, "unmeasured in the candidate"):
            gate_mod.gate(measured, unmeasured)
        with self.assertRaisesRegex(ValueError, "unmeasured in the baseline"):
            gate_mod.gate(unmeasured, measured)
        with self.assertRaisesRegex(ValueError, "sample count mismatch for action routing_input"):
            gate_mod.gate(measured, _budget_report(revision="fewer", reversals=REVERSALS_PER_ITERATION - 1))

    def test_refuses_reports_with_nothing_measured_in_both(self):
        baseline = _budget_report(revision="base")
        for row in baseline["actions"]:
            row.update(samples=0, p50_ms=None, p95_ms=None, verdict="unmeasured", boundary="absent." + row["action"])
        with self.assertRaisesRegex(ValueError, "no action is measured in both reports"):
            gate_mod.gate(baseline, copy.deepcopy(baseline))

    def test_exact_20_percent_passes_and_above_20_percent_fails(self):
        baseline = _budget_report(revision="base")
        self.assertTrue(gate_mod.gate(baseline, _budget_report(revision="at-limit", scale={"encode.packed": 1.20}))["passed"])

        over_limit = gate_mod.gate(baseline, _budget_report(revision="over-limit", scale={"encode.packed": 1.21}))
        self.assertFalse(over_limit["passed"])
        self.assertEqual(over_limit["regressed_actions"], ["save"])

    def test_exact_20_percent_passes_where_floating_point_rounds_above_it(self):
        self.assertGreater((0.84 - 0.7) / 0.7 * 100.0, 20.0)
        baseline = _budget_report(revision="base", samples={"encode.packed": [0.6, 0.7]})
        at_limit = gate_mod.gate(baseline, _budget_report(revision="at-limit", samples={"encode.packed": [0.6, 0.84]}))
        self.assertTrue(at_limit["passed"], at_limit)

        over_limit = gate_mod.gate(baseline, _budget_report(revision="over-limit", samples={"encode.packed": [0.6, 0.8401]}))
        self.assertEqual(over_limit["regressed_actions"], ["save"])

    def test_zero_baseline_handling(self):
        baseline = _budget_report(revision="base", scale={"encode.packed": 0.0})
        passed = gate_mod.gate(baseline, _budget_report(revision="same", scale={"encode.packed": 0.0}))
        self.assertTrue(passed["passed"])
        save_row = next(row for row in passed["actions"] if row["action"] == "save")
        self.assertEqual(save_row["change_pct"], 0.0)

        failed = gate_mod.gate(baseline, _budget_report(revision="slow", scale={"encode.packed": 0.05}))
        self.assertFalse(failed["passed"])
        self.assertEqual(failed["regressed_actions"], ["save"])
        save_row = next(row for row in failed["actions"] if row["action"] == "save")
        self.assertIsNone(save_row["change_pct"])
        self.assertIn("n/a (zero baseline)", gate_mod.format_report(failed))

    def test_overflowing_percentage_regresses_and_stays_json_safe(self):
        with tempfile.TemporaryDirectory() as tmp:
            base_path = Path(tmp) / "base.json"
            cand_path = Path(tmp) / "cand.json"
            base_path.write_text(json.dumps(_budget_report(revision="base", samples={"encode.packed": [1e-308, 1e-308]})), encoding="utf-8")
            cand_path.write_text(json.dumps(_budget_report(revision="cand", samples={"encode.packed": [1.0, 1.0]})), encoding="utf-8")
            result = _run_cli("--json", base_path, cand_path)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertNotIn("Infinity", result.stdout)
            self.assertNotIn("NaN", result.stdout)
            payload = json.loads(result.stdout, parse_constant=lambda name: self.fail(f"non-standard JSON constant {name}"))
            self.assertEqual(payload["regressed_actions"], ["save"])
            save_row = next(row for row in payload["actions"] if row["action"] == "save")
            self.assertIsNone(save_row["change_pct"])
            self.assertTrue(save_row["regressed"])

            markdown = _run_cli(base_path, cand_path)
            self.assertEqual(markdown.returncode, 1, markdown.stderr)
            self.assertIn("n/a (overflow)", markdown.stdout)
            self.assertNotIn("zero baseline", markdown.stdout)

    def test_cli_refuses_a_sample_too_large_for_a_float_with_exit_2(self):
        with tempfile.TemporaryDirectory() as tmp:
            base_path = Path(tmp) / "base.json"
            huge_path = Path(tmp) / "huge.json"
            base_path.write_text(json.dumps(_budget_report(revision="base")), encoding="utf-8")
            huge = _budget_report(revision="huge")
            encode = next(metric for metric in huge["metrics"] if metric["id"] == "encode.packed")
            encode["samples_ms"][-1] = 10**400
            huge_path.write_text(json.dumps(huge), encoding="utf-8")
            self.assertIn("1" + "0" * 400, huge_path.read_text(encoding="utf-8"))
            self.assertFalse(gate_mod._is_nonneg_finite_number(10**400))
            self.assertTrue(gate_mod._is_nonneg_finite_number(10**300))
            self.assertTrue(gate_mod._is_nonneg_finite_number(1.5))
            result = _run_cli(base_path, huge_path)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("invalid elapsed time: encode.packed", result.stderr)
            self.assertNotIn("Traceback", result.stderr)

    def test_rejects_confounded_or_tampered_reports(self):
        baseline = _budget_report(revision="base")
        for key, bad_value in [
            ("artifact_sha256", "c" * 64),
            ("project_sha256", "d" * 64),
            ("recipe", "dense-v1"),
            ("target", "browser-cpu"),
            ("debug_assertions", True),
            ("geometry", {"tracks": 61, "vias": 0}),
            ("unmeasured", UNMEASURED[:-1]),
        ]:
            with self.subTest(mismatch=key):
                other = copy.deepcopy(baseline)
                other[key] = bad_value
                with self.assertRaisesRegex(ValueError, f"comparison mismatch: {key}"):
                    gate_mod.gate(baseline, other)

        with self.assertRaisesRegex(ValueError, "comparison mismatch: environment.os"):
            gate_mod.gate(baseline, _budget_report(revision="cand", os_name="windows"))

        tampered = copy.deepcopy(baseline)
        tampered["actions"][0]["p95_ms"] = 1.0
        with self.assertRaisesRegex(ValueError, "quantiles do not match raw metrics"):
            gate_mod.validate(tampered)

        tampered_verdict = copy.deepcopy(baseline)
        move_commit = next(row for row in tampered_verdict["actions"] if row["action"] == "move_commit")
        move_commit["verdict"] = "measured_miss"
        with self.assertRaisesRegex(ValueError, "verdict does not match"):
            gate_mod.validate(tampered_verdict)

        tampered_summary = copy.deepcopy(baseline)
        tampered_summary["metrics"][0]["p99_ms"] = 0.0
        with self.assertRaisesRegex(ValueError, "summary does not match raw samples"):
            gate_mod.validate(tampered_summary)

        empty_metric = copy.deepcopy(baseline)
        empty_metric["metrics"].append({"id": "empty", "samples_ms": [], "p50_ms": 0, "p95_ms": 0, "p99_ms": 0, "max_ms": 0})
        with self.assertRaisesRegex(ValueError, "nonempty sample list"):
            gate_mod.validate(empty_metric)

        duplicate_metric = copy.deepcopy(baseline)
        duplicate_metric["metrics"].append(copy.deepcopy(baseline["metrics"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate metric"):
            gate_mod.validate(duplicate_metric)

        for bad_threshold in (-0.1, float("nan"), float("inf"), True):
            with self.subTest(threshold=bad_threshold):
                with self.assertRaisesRegex(ValueError, "max_regression_pct"):
                    gate_mod.gate(baseline, baseline, max_regression_pct=bad_threshold)

    def test_cli_exit_codes_and_json_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base_path = root / "base.json"
            clean_path = root / "clean.json"
            slow_path = root / "slow.json"
            bad_path = root / "bad.json"
            base_path.write_text(json.dumps(_budget_report(revision="base")), encoding="utf-8")
            clean_path.write_text(json.dumps(_budget_report(revision="clean")), encoding="utf-8")
            slow_path.write_text(json.dumps(_budget_report(revision="slow", scale={"command.move_tracks": 1.30})), encoding="utf-8")
            bad_path.write_text("{not json", encoding="utf-8")

            ok = _run_cli(base_path, clean_path)
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertIn("Performance regression gate: PASS", ok.stdout)

            fail = _run_cli("--json", base_path, slow_path)
            self.assertEqual(fail.returncode, 1, fail.stderr)
            payload = json.loads(fail.stdout)
            self.assertFalse(payload["passed"])
            self.assertEqual(payload["regressed_actions"], ["move_commit"])

            invalid = _run_cli(base_path, bad_path)
            self.assertEqual(invalid.returncode, 2)
            self.assertIn("incomparable or invalid budget reports", invalid.stderr)


if __name__ == "__main__":
    unittest.main()
