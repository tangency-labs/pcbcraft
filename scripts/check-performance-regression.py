#!/usr/bin/env python3
"""Gate matched budget-suite reports across Linux, Windows or macOS against p95 regressions."""
import argparse
import json
import math
from pathlib import Path
import sys

BUDGET_SCHEMA = "pcbcraft-performance-budgets/1"
GATE_SCHEMA = "pcbcraft-performance-regression-gate/1"
DEFAULT_MAX_REGRESSION_PCT = 20.0
# Percentage points ignored above the threshold, so that a change of exactly the threshold
# (for example 0.7 ms -> 0.84 ms at 20 %, which computes to 20.000000000000004) still passes.
THRESHOLD_TOLERANCE_PCT = 1e-9
ENVIRONMENT_MATCH_KEYS = ("hardware", "os", "runtime", "compiler", "profile", "power")
REPORT_MATCH_KEYS = (
    "target",
    "debug_assertions",
    "artifact_sha256",
    "project_sha256",
    "recipe",
    "iterations",
    "geometry",
    "unmeasured",
)


def _is_nonneg_finite_number(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    try:
        # An integer too large for a float (for example 10**400) raises OverflowError here.
        return math.isfinite(value) and value >= 0
    except OverflowError:
        return False


def _nearest_rank(ordered, percent):
    return ordered[math.ceil(len(ordered) * percent / 100) - 1]


def _validate_metrics(raw_metrics):
    """Validate raw metrics and their summaries; return them by id.

    A metric's sample count is not tied to the iteration count: the budget suite pools
    every reversing route pointer move of an iteration into one aggregate metric, so that
    metric holds several samples per iteration.
    """
    if not isinstance(raw_metrics, list) or not raw_metrics:
        raise ValueError("metrics must be a nonempty list")
    metrics = {}
    for metric in raw_metrics:
        if not isinstance(metric, dict):
            raise ValueError("each metric must be an object")
        name = metric.get("id")
        samples = metric.get("samples_ms")
        if not isinstance(name, str) or not name:
            raise ValueError("metric requires a nonempty id")
        if name in metrics:
            raise ValueError(f"duplicate metric: {name}")
        if not isinstance(samples, list) or not samples:
            raise ValueError(f"metric {name} requires a nonempty sample list")
        if any(not _is_nonneg_finite_number(sample) for sample in samples):
            raise ValueError(f"invalid elapsed time: {name}")
        ordered = sorted(samples)
        for field, percent in (("p50_ms", 50), ("p95_ms", 95), ("p99_ms", 99), ("max_ms", 100)):
            if metric.get(field) != _nearest_rank(ordered, percent):
                raise ValueError(f"summary does not match raw samples: {name}.{field}")
        metrics[name] = metric
    return metrics


def _samples_for_boundary(boundary, metrics):
    if "*" in boundary:
        if boundary.count("*") != 1:
            raise ValueError(f"invalid pooled boundary pattern: {boundary}")
        prefix, suffix = boundary.split("*", 1)
        pooled = []
        for name, metric in metrics.items():
            if name.startswith(prefix) and name.endswith(suffix):
                pooled.extend(metric["samples_ms"])
        return pooled
    metric = metrics.get(boundary)
    return list(metric["samples_ms"]) if metric is not None else []


def validate(report):
    """Validate a pcbcraft-performance-budgets/1 report and recompute every action's quantiles."""
    if not isinstance(report, dict) or report.get("schema") != BUDGET_SCHEMA:
        raise ValueError("unsupported budget report schema")
    env = report.get("environment")
    if not isinstance(env, dict):
        raise ValueError("environment must be an object")
    for key in ("revision", *ENVIRONMENT_MATCH_KEYS):
        value = env.get(key)
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f"environment.{key} must be a nonempty string")
    if not isinstance(env.get("notes"), str):
        raise ValueError("environment.notes must be a string")
    if not isinstance(report.get("target"), str) or not report["target"].strip():
        raise ValueError("target must be a nonempty string")
    if not isinstance(report.get("debug_assertions"), bool):
        raise ValueError("debug_assertions must be a boolean")
    for key in ("artifact_sha256", "project_sha256", "recipe"):
        value = report.get(key)
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f"{key} must be a nonempty string")
    if type(report.get("iterations")) is not int or report["iterations"] < 1:
        raise ValueError("invalid iteration count")
    if not isinstance(report.get("geometry"), dict):
        raise ValueError("geometry must be an object")
    unmeasured = report.get("unmeasured")
    if not isinstance(unmeasured, list) or any(not isinstance(item, str) for item in unmeasured):
        raise ValueError("unmeasured must be a list of strings")

    metrics = _validate_metrics(report.get("metrics"))

    raw_actions = report.get("actions")
    if not isinstance(raw_actions, list) or not raw_actions:
        raise ValueError("actions must be a nonempty list")
    actions = {}
    for row in raw_actions:
        if not isinstance(row, dict):
            raise ValueError("each action must be an object")
        action = row.get("action")
        boundary = row.get("boundary")
        if not isinstance(action, str) or not action.strip():
            raise ValueError("action name must be a nonempty string")
        if action in actions:
            raise ValueError(f"duplicate action: {action}")
        if not isinstance(boundary, str) or not boundary.strip():
            raise ValueError(f"action {action} requires a nonempty boundary")
        if not isinstance(row.get("budget_basis"), str):
            raise ValueError(f"action {action} requires a string budget_basis")
        sample_count = row.get("samples")
        if type(sample_count) is not int or sample_count < 0:
            raise ValueError(f"action {action} has invalid sample count")
        budget_p95 = row.get("budget_p95_ms")
        if budget_p95 is not None and not _is_nonneg_finite_number(budget_p95):
            raise ValueError(f"action {action} has invalid budget_p95_ms")

        samples = _samples_for_boundary(boundary, metrics)
        if sample_count != len(samples):
            raise ValueError(f"action {action} sample count does not match raw metrics")
        if not samples:
            if row.get("p50_ms") is not None or row.get("p95_ms") is not None or row.get("verdict") != "unmeasured":
                raise ValueError(f"unmeasured action {action} has inconsistent summary fields")
        else:
            ordered = sorted(samples)
            expected_p50 = _nearest_rank(ordered, 50)
            expected_p95 = _nearest_rank(ordered, 95)
            if row.get("p50_ms") != expected_p50 or row.get("p95_ms") != expected_p95:
                raise ValueError(f"action {action} quantiles do not match raw metrics")
            if budget_p95 is None:
                expected_verdict = "no_budget"
            elif expected_p95 < budget_p95:
                expected_verdict = "measured_pass"
            else:
                expected_verdict = "measured_miss"
            if row.get("verdict") != expected_verdict:
                raise ValueError(f"action {action} verdict does not match p95 and budget")
        actions[action] = row
    return metrics, actions


def gate(baseline, candidate, max_regression_pct=DEFAULT_MAX_REGRESSION_PCT):
    """Compare matched baseline and candidate budget reports and fail regressions above the threshold.

    An action unmeasured in both reports (for example routing input on a trace that never
    reverses a route) is not compared and is listed in `not_compared`. An action measured
    in only one report, or with different sample counts, makes the reports incomparable.
    """
    if not _is_nonneg_finite_number(max_regression_pct):
        raise ValueError("max_regression_pct must be finite and non-negative")
    _, old_actions = validate(baseline)
    _, new_actions = validate(candidate)
    for key in REPORT_MATCH_KEYS:
        if baseline[key] != candidate[key]:
            raise ValueError(f"comparison mismatch: {key}")
    for key in ENVIRONMENT_MATCH_KEYS:
        if baseline["environment"][key] != candidate["environment"][key]:
            raise ValueError(f"comparison mismatch: environment.{key}")
    if old_actions.keys() != new_actions.keys():
        raise ValueError("action sets differ between baseline and candidate reports")

    rows = []
    regressed_actions = []
    not_compared = []
    for action, base_row in old_actions.items():
        cand_row = new_actions[action]
        if base_row["boundary"] != cand_row["boundary"]:
            raise ValueError(f"boundary mismatch for action {action}: {base_row['boundary']} vs {cand_row['boundary']}")
        base_unmeasured = base_row["samples"] == 0
        cand_unmeasured = cand_row["samples"] == 0
        if base_unmeasured and cand_unmeasured:
            not_compared.append(action)
            continue
        if base_unmeasured:
            raise ValueError(f"action {action} is unmeasured in the baseline but measured in the candidate")
        if cand_unmeasured:
            raise ValueError(f"action {action} is measured in the baseline but unmeasured in the candidate")
        if base_row["samples"] != cand_row["samples"]:
            raise ValueError(f"sample count mismatch for action {action}: {base_row['samples']} vs {cand_row['samples']}")
        base_p95 = float(base_row["p95_ms"])
        cand_p95 = float(cand_row["p95_ms"])
        if base_p95 == 0.0:
            if cand_p95 == 0.0:
                change_pct = 0.0
                regressed = False
            else:
                change_pct = None
                regressed = True
        else:
            change_pct = ((cand_p95 - base_p95) / base_p95) * 100.0
            if math.isfinite(change_pct):
                regressed = change_pct > max_regression_pct + THRESHOLD_TOLERANCE_PCT
            else:
                # A tiny nonzero baseline can overflow the percentage (1e-308 ms -> 1 ms). JSON has
                # no infinity, so report it as None, as for a zero baseline; the slowdown exceeds
                # every finite threshold, so it regresses.
                change_pct = None
                regressed = True
        if regressed:
            regressed_actions.append(action)
        rows.append(
            {
                "action": action,
                "boundary": base_row["boundary"],
                "samples": cand_row["samples"],
                "baseline_p95_ms": base_p95,
                "candidate_p95_ms": cand_p95,
                "change_pct": change_pct,
                "budget_p95_ms": cand_row["budget_p95_ms"],
                "candidate_verdict": cand_row["verdict"],
                "regressed": regressed,
            }
        )
    if not rows:
        raise ValueError("no action is measured in both reports")

    return {
        "schema": GATE_SCHEMA,
        "passed": len(regressed_actions) == 0,
        "max_regression_pct": float(max_regression_pct),
        "regressed_actions": regressed_actions,
        "not_compared": not_compared,
        "baseline_revision": baseline["environment"]["revision"],
        "candidate_revision": candidate["environment"]["revision"],
        "target": candidate["target"],
        "os": candidate["environment"]["os"],
        "debug_assertions": candidate["debug_assertions"],
        "profile": candidate["environment"]["profile"],
        "artifact_sha256": candidate["artifact_sha256"],
        "project_sha256": candidate["project_sha256"],
        "recipe": candidate["recipe"],
        "iterations": candidate["iterations"],
        "actions": rows,
        "unmeasured": candidate["unmeasured"],
    }


def format_report(result):
    """Format a gate result as readable Markdown naming any regressed actions."""
    status = "PASS" if result["passed"] else f"FAIL ({', '.join(result['regressed_actions'])})"
    lines = [
        f"Performance regression gate: {status}",
        f"{result['baseline_revision']} -> {result['candidate_revision']} ({result['os']}, {result['target']})",
        f"Recipe: {result['recipe']}; iterations: {result['iterations']}; max allowed p95 regression: {result['max_regression_pct']:.1f}%",
        f"Artifact: {result['artifact_sha256']}",
        "",
        "| Action | Boundary | Samples | Baseline p95 ms | Candidate p95 ms | Change | Budget | Gate |",
        "| --- | --- | ---: | ---: | ---: | ---: | --- | --- |",
    ]
    for row in result["actions"]:
        if row["change_pct"] is None and row["baseline_p95_ms"] == 0.0:
            change = "n/a (zero baseline)"
        elif row["change_pct"] is None:
            change = "n/a (overflow)"
        else:
            change = f"{row['change_pct']:+.1f}%"
        budget_text = f"< {row['budget_p95_ms']} ms ({row['candidate_verdict']})" if row["budget_p95_ms"] is not None else row["candidate_verdict"]
        gate_text = "FAIL" if row["regressed"] else "PASS"
        lines.append(
            f"| {row['action']} | {row['boundary']} | {row['samples']} | {row['baseline_p95_ms']:.6f} | {row['candidate_p95_ms']:.6f} | {change} | {budget_text} | {gate_text} |"
        )
    if result["regressed_actions"]:
        lines += ["", "Regressed actions: " + ", ".join(result["regressed_actions"])]
    if result["not_compared"]:
        lines += ["", "Not compared (unmeasured in both reports): " + ", ".join(result["not_compared"])]
    lines += ["", "Unmeasured: " + "; ".join(result["unmeasured"])]
    if result["debug_assertions"] or result["profile"] != "release":
        lines.append("Diagnostic profile only; do not publish this as a shipping-profile baseline.")
    return "\n".join(lines)


def main(argv=None):
    """Compare baseline and candidate budget reports; exit 0 on pass, 1 on regression, 2 on invalid input."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path, help="Path to baseline pcbcraft-performance-budgets/1 JSON report")
    parser.add_argument("candidate", type=Path, help="Path to candidate pcbcraft-performance-budgets/1 JSON report")
    parser.add_argument(
        "--max-regression-pct",
        type=float,
        default=DEFAULT_MAX_REGRESSION_PCT,
        help=f"Maximum allowed p95 slowdown percentage before failing (default: {DEFAULT_MAX_REGRESSION_PCT})",
    )
    parser.add_argument("--json", action="store_true", dest="as_json", help="Emit structured JSON instead of Markdown")
    args = parser.parse_args(argv)
    try:
        baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
        candidate = json.loads(args.candidate.read_text(encoding="utf-8"))
        result = gate(baseline, candidate, max_regression_pct=args.max_regression_pct)
    except (OSError, json.JSONDecodeError, ValueError, KeyError, TypeError) as error:
        parser.exit(2, f"incomparable or invalid budget reports: {error}\n")
    if args.as_json:
        print(json.dumps(result, indent=2))
    else:
        print(format_report(result))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
