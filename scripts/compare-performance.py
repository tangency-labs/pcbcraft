#!/usr/bin/env python3
"""Compare CPU reports only when fixture, measurement and environment identities agree."""
import argparse
import json
import math
from pathlib import Path


def validate(report):
    """Reject malformed evidence, including nonfinite samples or stale summary quantiles."""
    if not isinstance(report, dict) or report.get("schema") != "pcbcraft-performance-report/1":
        raise ValueError("unsupported report schema")
    if type(report.get("iterations")) is not int or report["iterations"] < 1:
        raise ValueError("invalid iteration count")
    if not isinstance(report.get("metrics"), list):
        raise ValueError("metrics must be a list")
    metrics = {}
    for metric in report["metrics"]:
        if not isinstance(metric, dict):
            raise ValueError("each metric must be an object")
        name, samples = metric["id"], metric["samples_ms"]
        if not isinstance(name, str) or not name or not isinstance(samples, list):
            raise ValueError("metric requires a name and sample list")
        if name in metrics or len(samples) != report["iterations"]:
            raise ValueError(f"duplicate metric or wrong sample count: {name}")
        if any(isinstance(s, bool) or not isinstance(s, (float, int)) or not math.isfinite(s) or s < 0 for s in samples):
            raise ValueError(f"invalid elapsed time: {name}")
        ordered = sorted(samples)
        for field, percent in [("p50_ms", 50), ("p95_ms", 95), ("p99_ms", 99), ("max_ms", 100)]:
            expected = ordered[math.ceil(len(ordered) * percent / 100) - 1]
            if metric[field] != expected:
                raise ValueError(f"summary does not match raw samples: {name}.{field}")
        metrics[name] = metric
    if not metrics:
        raise ValueError("report has no metrics")
    return metrics


def compare(before, after):
    """Refuse confounded comparisons; revision and narrative notes may differ intentionally."""
    old, new = validate(before), validate(after)
    for key in ["target", "debug_assertions", "artifact_sha256", "project_sha256", "recipe", "trace_id", "iterations", "warmup_frames", "geometry", "unmeasured"]:
        if before[key] != after[key]:
            raise ValueError(f"comparison mismatch: {key}")
    for key in ["hardware", "os", "runtime", "compiler", "profile", "power"]:
        if before["environment"][key] != after["environment"][key]:
            raise ValueError(f"comparison mismatch: environment.{key}")
    if old.keys() != new.keys():
        raise ValueError("measurement boundaries differ; use a matched harness on both revisions")
    lines = [f"CPU comparison: {before['environment']['revision']} → {after['environment']['revision']}",
             f"{after['target']}; {after['iterations']} samples per boundary; artifact {after['artifact_sha256']}",
             "", "| Boundary | Before p95 ms | After p95 ms | Change | After p99 ms | Max ms |", "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for name in sorted(new):
        a, b = old[name]["p95_ms"], new[name]["p95_ms"]
        delta = f"{(b/a-1)*100:+.1f}%" if a else "n/a (zero baseline)"
        lines.append(f"| {name} | {a:.6f} | {b:.6f} | {delta} | {new[name]['p99_ms']:.6f} | {new[name]['max_ms']:.6f} |")
    over = [name for name, metric in new.items() if name.startswith("replay.") and name.endswith(".ui") and metric["p95_ms"] >= 4]
    lines += ["", "#237 budget assessment: CPU replay is a proxy, so this report cannot certify input-to-visible or native event-loop budgets."]
    lines.append("CPU frame p95 ≥ 4 ms: " + (", ".join(sorted(over)) if over else "none in these samples"))
    lines.append("Unmeasured: " + "; ".join(after["unmeasured"]))
    if after["debug_assertions"] or after["environment"]["profile"] != "release":
        lines.append("Diagnostic profile only; do not publish this as a shipping-profile baseline.")
    return "\n".join(lines)


def main():
    """Read two immutable reports and print a reviewable comparison, or fail on mismatches."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    args = parser.parse_args()
    try:
        print(compare(json.loads(args.before.read_text()), json.loads(args.after.read_text())))
    except (ValueError, KeyError, TypeError) as error:
        parser.exit(2, f"incomparable reports: {error}\n")


if __name__ == "__main__":
    main()
