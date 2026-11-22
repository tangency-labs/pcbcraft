"""Cross-platform performance regression gate tests without wall-clock timing assertions."""
import copy
import importlib.util
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT_PATH = Path(__file__).with_name("check-performance-regression.py")
spec = importlib.util.spec_from_file_location("check_performance_regression", SCRIPT_PATH)
gate_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate_mod)


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


def _action(action, boundary, metrics, budget_p95_ms=None, basis="no target yet"):
    if "*" in boundary:
        prefix, suffix = boundary.split("*", 1)
        samples = [s for m in metrics if m["id"].startswith(prefix) and m["id"].endswith(suffix) for s in m["samples_ms"]]
    else:
        samples = [s for m in metrics if m["id"] == boundary for s in m["samples_ms"]]
    if not samples:
        return {
            "action": action,
            "boundary": boundary,
            "samples": 0,
            "p50_ms": None,
            "p95_ms": None,
            "budget_p95_ms": budget_p95_ms,
            "budget_basis": basis,
            "verdict": "unmeasured",
        }
    ordered = sorted(samples)
    p50 = ordered[math.ceil(len(ordered) * 50 / 100) - 1]
    p95 = ordered[math.ceil(len(ordered) * 95 / 100) - 1]
    if budget_p95_ms is None:
        verdict = "no_budget"
    elif p95 < budget_p95_ms:
        verdict = "measured_pass"
    else:
        verdict = "measured_miss"
    return {
        "action": action,
        "boundary": boundary,
        "samples": len(samples),
        "p50_ms": p50,
        "p95_ms": p95,
        "budget_p95_ms": budget_p95_ms,
        "budget_basis": basis,
        "verdict": verdict,
    }


def _budget_report(revision="base", os_name="linux", scale=None):
    scale = scale or {}
    raw = {
        "decode.cold": [10.0, 12.0, 14.0],
        "encode.packed": [8.0, 9.0, 10.0],
        "drc.synchronous": [20.0, 22.0, 25.0],
        "electrical_connectivity.first": [5.0, 6.0, 7.0],
        "command.move_tracks": [15.0, 18.0, 20.0],
        "replay.reverse.0.ui": [1.0, 1.5, 2.0],
        "replay.reverse.1.ui": [1.2, 1.8, 2.5],
    }
    metrics = []
    for metric_id, samples in raw.items():
        factor = scale.get(metric_id, 1.0)
        metrics.append(_metric(metric_id, [s * factor for s in samples]))
    actions = [
        _action("open", "decode.cold", metrics),
        _action("save", "encode.packed", metrics),
        _action("drc", "drc.synchronous", metrics),
        _action("connectivity", "electrical_connectivity.first", metrics),
        _action("move_commit", "command.move_tracks", metrics, 100.0, "proposed #237 commit target; proxy"),
        _action("routing_input", "replay.reverse.*.ui", metrics, 4.0, "proposed #237 routing-input target; proxy"),
    ]
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
        "recipe": "envelope-v1",
        "iterations": 3,
        "geometry": {"tracks": 65536, "vias": 16384},
        "actions": actions,
        "metrics": metrics,
        "unmeasured": ["presentation", "memory: process RSS on the envelope board"],
    }


class RegressionGateTests(unittest.TestCase):
    def test_passes_without_slowdown_and_fails_on_deliberate_30_percent_delay(self):
        for os_name in ("linux", "windows", "macos"):
            with self.subTest(os=os_name):
                baseline = _budget_report(revision="base", os_name=os_name)
                clean_candidate = _budget_report(revision="cand", os_name=os_name)
                clean = gate_mod.gate(baseline, clean_candidate)
                self.assertTrue(clean["passed"])
                self.assertEqual(clean["regressed_actions"], [])

                slowed_candidate = _budget_report(revision="cand-slow", os_name=os_name, scale={"drc.synchronous": 1.30})
                failed = gate_mod.gate(baseline, slowed_candidate)
                self.assertFalse(failed["passed"])
                self.assertEqual(failed["regressed_actions"], ["drc"])
                rendered = gate_mod.format_report(failed)
                self.assertIn("FAIL (drc)", rendered)
                self.assertIn("Regressed actions: drc", rendered)
                self.assertIn("+30.0%", rendered)

    def test_pooled_action_30_percent_slowdown_fails_and_names_routing_input(self):
        baseline = _budget_report(revision="base")
        slowed = _budget_report(
            revision="cand-slow",
            scale={"replay.reverse.0.ui": 1.30, "replay.reverse.1.ui": 1.30},
        )
        result = gate_mod.gate(baseline, slowed)
        self.assertFalse(result["passed"])
        self.assertEqual(result["regressed_actions"], ["routing_input"])

    def test_exact_20_percent_passes_and_above_20_percent_fails(self):
        baseline = _budget_report(revision="base")
        at_limit = _budget_report(revision="at-limit", scale={"encode.packed": 1.20})
        self.assertTrue(gate_mod.gate(baseline, at_limit)["passed"])

        over_limit = _budget_report(revision="over-limit", scale={"encode.packed": 1.21})
        result = gate_mod.gate(baseline, over_limit)
        self.assertFalse(result["passed"])
        self.assertEqual(result["regressed_actions"], ["save"])

    def test_zero_baseline_handling(self):
        baseline = _budget_report(revision="base", scale={"encode.packed": 0.0})
        same_zero = _budget_report(revision="same", scale={"encode.packed": 0.0})
        passed = gate_mod.gate(baseline, same_zero)
        self.assertTrue(passed["passed"])
        save_row = next(row for row in passed["actions"] if row["action"] == "save")
        self.assertEqual(save_row["change_pct"], 0.0)

        regressed_zero = _budget_report(revision="slow", scale={"encode.packed": 0.05})
        failed = gate_mod.gate(baseline, regressed_zero)
        self.assertFalse(failed["passed"])
        self.assertEqual(failed["regressed_actions"], ["save"])
        save_row = next(row for row in failed["actions"] if row["action"] == "save")
        self.assertIsNone(save_row["change_pct"])

    def test_rejects_confounded_or_tampered_reports(self):
        baseline = _budget_report(revision="base")
        for key, bad_value in [
            ("artifact_sha256", "c" * 64),
            ("project_sha256", "d" * 64),
            ("recipe", "reference-v1"),
            ("target", "browser-cpu"),
            ("debug_assertions", True),
        ]:
            with self.subTest(mismatch=key):
                other = copy.deepcopy(baseline)
                other[key] = bad_value
                with self.assertRaisesRegex(ValueError, f"comparison mismatch: {key}"):
                    gate_mod.gate(baseline, other)

        wrong_os = _budget_report(revision="cand", os_name="windows")
        with self.assertRaisesRegex(ValueError, "comparison mismatch: environment.os"):
            gate_mod.gate(baseline, wrong_os)

        tampered = copy.deepcopy(baseline)
        tampered["actions"][0]["p95_ms"] = 1.0
        with self.assertRaisesRegex(ValueError, "quantiles do not match raw metrics"):
            gate_mod.validate(tampered)

        tampered_verdict = copy.deepcopy(baseline)
        tampered_verdict["actions"][4]["verdict"] = "measured_miss"
        with self.assertRaisesRegex(ValueError, "verdict does not match"):
            gate_mod.validate(tampered_verdict)

        unmeasured = copy.deepcopy(baseline)
        unmeasured["actions"].append(
            {
                "action": "missing_action",
                "boundary": "missing.metric",
                "samples": 0,
                "p50_ms": None,
                "p95_ms": None,
                "budget_p95_ms": 10.0,
                "budget_basis": "test",
                "verdict": "unmeasured",
            }
        )
        with self.assertRaisesRegex(ValueError, "is unmeasured"):
            gate_mod.gate(unmeasured, copy.deepcopy(unmeasured))

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

            ok = subprocess.run(
                [sys.executable, "-B", str(SCRIPT_PATH), str(base_path), str(clean_path)],
                capture_output=True,
                text=True,
            )
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertIn("Performance regression gate: PASS", ok.stdout)

            fail = subprocess.run(
                [sys.executable, "-B", str(SCRIPT_PATH), "--json", str(base_path), str(slow_path)],
                capture_output=True,
                text=True,
            )
            self.assertEqual(fail.returncode, 1, fail.stderr)
            payload = json.loads(fail.stdout)
            self.assertFalse(payload["passed"])
            self.assertEqual(payload["regressed_actions"], ["move_commit"])

            invalid = subprocess.run(
                [sys.executable, "-B", str(SCRIPT_PATH), str(base_path), str(bad_path)],
                capture_output=True,
                text=True,
            )
            self.assertEqual(invalid.returncode, 2)
            self.assertIn("incomparable or invalid budget reports", invalid.stderr)


if __name__ == "__main__":
    unittest.main()
