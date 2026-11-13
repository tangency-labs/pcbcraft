#!/usr/bin/env python3
"""Collect isolated native CPU replay resources; profiler timings are never latency baselines."""
import argparse
import hashlib
import json
import platform
import re
import shutil
import subprocess
import time
from pathlib import Path


def allocation_totals(log):
    """Read Valgrind's explicit whole-process heap totals, refusing missing statistics."""
    match = re.search(r"total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated", log)
    if not match:
        raise ValueError("Valgrind did not report allocation totals")
    return dict(zip(("allocations", "frees", "bytes_allocated"), (int(x.replace(",", "")) for x in match.groups())))


def resident_memory(pid):
    """Read observed Linux RSS/high-water values; unavailable hosts return no invented value."""
    try:
        status = Path(f"/proc/{pid}/status").read_text()
    except (OSError, ProcessLookupError):
        return {}
    return {key: int(match.group(1)) * 1024 for key in ("VmRSS", "VmHWM")
            if (match := re.search(rf"^{key}:\s*(\d+) kB$", status, re.MULTILINE))}


def execute(command, log_path, timeout):
    """Bound runtime, sample process RSS, and terminate only the child owned by this collector."""
    samples = []
    start = time.monotonic()
    with log_path.open("x") as log:
        child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        try:
            while child.poll() is None:
                memory = resident_memory(child.pid)
                if memory:
                    samples.append(dict(elapsed_ms=(time.monotonic() - start) * 1000, **memory))
                if time.monotonic() - start > timeout:
                    raise TimeoutError("resource run exceeded its deadline")
                time.sleep(0.01)
            if child.returncode:
                raise RuntimeError(f"resource run exited {child.returncode}; see {log_path}")
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
    return dict(elapsed_ms=(time.monotonic() - start) * 1000, rss_samples=samples,
                observed_peak_rss_bytes=max((x["VmRSS"] for x in samples if "VmRSS" in x), default=None),
                observed_high_water_bytes=max((x["VmHWM"] for x in samples if "VmHWM" in x), default=None))


def main():
    """Keep uninstrumented timings and optional allocation-profile evidence in distinct files."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("artifact", type=Path)
    parser.add_argument("environment", type=Path)
    parser.add_argument("output_directory", type=Path)
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument("--timeout", type=float, default=900)
    parser.add_argument("--allocations", action="store_true", help="also run a separate one-iteration Valgrind allocation profile")
    args = parser.parse_args()
    if not 1 <= args.iterations <= 10000 or not 1 <= args.timeout <= 86400:
        parser.error("iterations must be 1..10000 and timeout 1..86400 seconds")
    executable = args.executable.resolve(strict=True)
    artifact = args.artifact.resolve(strict=True)
    environment = args.environment.resolve(strict=True)
    if args.allocations and not shutil.which("valgrind"):
        parser.error("--allocations requires Valgrind")
    args.output_directory.mkdir()
    output = args.output_directory.resolve()
    command = [str(executable), "run", str(artifact), str(environment), str(args.iterations), str(output / "timings.json")]
    report = dict(schema="pcbcraft-performance-resources/1", executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
                  artifact_sha256=hashlib.sha256(artifact.read_bytes()).hexdigest(), platform=platform.platform(),
                  memory_scope="whole child process; 10 ms RSS sampling; Linux VmHWM where available; not isolated query/index memory",
                  command=command, uninstrumented=execute(command, output / "run.log", args.timeout), allocations=None)
    if args.allocations:
        log = output / "valgrind.log"
        profiler = ["valgrind", "--tool=memcheck", "--leak-check=no", "--log-file=" + str(log)]
        # A separate single replay counts startup, decoding, oracles, fonts and report storage.
        # Its timings include profiler overhead and must never be used as latency baselines.
        profile_command = profiler + [str(executable), "run", str(artifact), str(environment), "1", str(output / "profiled-timings-not-a-baseline.json")]
        resources = execute(profile_command, output / "profile-run.log", args.timeout)
        report["allocations"] = dict(scope="whole process, one replay; includes startup/oracles/reporting; profiler timings are diagnostic only",
                                     command=profile_command, tool_version=subprocess.check_output(["valgrind", "--version"], text=True).strip(),
                                     totals=allocation_totals(log.read_text()), resources=resources)
    with (output / "resources.json").open("x") as file:
        json.dump(report, file, indent=2)
    print(output / "resources.json")


if __name__ == "__main__":
    main()
