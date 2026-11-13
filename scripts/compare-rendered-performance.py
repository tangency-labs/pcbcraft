#!/usr/bin/env python3
"""Compare identical rendered replays without equating GPU readback with presentation."""
import argparse
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("cpu", Path(__file__).with_name("compare-performance.py"))
cpu = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cpu)


def validate(report):
    """Require complete real-image evidence and consistent raw sample counts/quantiles."""
    if report.get("schema") != "pcbcraft-rendered-performance-report/1":
        raise ValueError("unsupported rendered report")
    if report.get("target") not in ("native-rendered", "browser-rendered") or not report.get("renderer"):
        raise ValueError("rendered target and renderer are required")
    size = report.get("image_size")
    if not isinstance(size, list) or len(size) != 2 or any(type(x) is not int or x <= 0 for x in size):
        raise ValueError("missing rendered dimensions")
    if len(report.get("last_image_sha256", "")) != 64:
        raise ValueError("missing image digest")
    shaped = dict(report, schema="pcbcraft-performance-report/1")
    metrics = cpu.validate(shaped)
    captures = sum(len(x["samples_ms"]) for name, x in metrics.items() if name.endswith(".input_to_gpu_readback"))
    if not captures or report.get("captures") != captures:
        raise ValueError("rendered capture count does not match samples")
    return metrics


def compare(before, after):
    """Refuse changes in renderer, viewport, environment, artifacts or measurement boundaries."""
    old, new = validate(before), validate(after)
    for key in ("target", "debug_assertions", "artifact_sha256", "project_sha256", "recipe", "trace_id", "iterations", "warmup_frames", "renderer", "image_size", "input_policy", "idle_between_iterations_ms", "unmeasured"):
        if before[key] != after[key]:
            raise ValueError(f"comparison mismatch: {key}")
    for key in ("hardware", "os", "runtime", "compiler", "profile", "power"):
        if before["environment"][key] != after["environment"][key]:
            raise ValueError(f"comparison mismatch: environment.{key}")
    states = lambda report: [{key: event.get(key) for key in ("focused", "visible", "minimized", "occluded")}
                             for event in report.get("window_observations", [])]
    if states(before) != states(after):
        raise ValueError("comparison mismatch: observed window visibility/focus transitions")
    if old.keys() != new.keys():
        raise ValueError("rendered measurement boundaries differ")
    lines = [f"Rendered comparison: {before['environment']['revision']} → {after['environment']['revision']}",
             "Input begins at synthetic framework injection; GPU readback includes screenshot transfer and browser frame delay.",
             "These timings do not establish OS-input-to-visible or compositor presentation latency.", "",
             "| Boundary | Before p95 ms | After p95 ms | After p99 ms | Max ms |",
             "| --- | ---: | ---: | ---: | ---: |"]
    for name in sorted(new):
        metric = new[name]
        lines.append(f"| {name} | {old[name]['p95_ms']:.6f} | {metric['p95_ms']:.6f} | {metric['p99_ms']:.6f} | {metric['max_ms']:.6f} |")
    lines += ["", "Unmeasured: " + "; ".join(after["unmeasured"])]
    if after["debug_assertions"] or after["environment"]["profile"] != "release":
        lines.append("Diagnostic build; not a shipping-profile baseline.")
    return "\n".join(lines)


def main():
    """Read immutable before/after evidence and print the matched comparison."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    args = parser.parse_args()
    try:
        print(compare(json.loads(args.before.read_text()), json.loads(args.after.read_text())))
    except (ValueError, KeyError, TypeError) as error:
        parser.exit(2, f"incomparable rendered reports: {error}\n")


if __name__ == "__main__":
    main()
