"""Run Windows integration coverage and stage the existing real-window regression once."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import subprocess

def commands() -> list[list[str]]:
    """Run the complete suite only in an approved or full validation stage."""
    base = ["cargo", "test", "--locked", "--profile", "ci"]
    return [["cargo", "clippy", "--locked", "--profile", "ci", "--workspace", "--all-targets", "--", "-D", "warnings"],
            base + ["--workspace", "--message-format=json"],
            ["cargo", "clippy", "--locked", "--profile", "ci", "-p", "pcbcraft-app", "--features", "performance-harness", "--all-targets", "--", "-D", "warnings"],
            base + ["-p", "pcbcraft-app", "--features", "performance-harness", "performance::"]]


def run_workspace_tests(command: list[str]) -> Path:
    """Stream test output and capture the lifecycle executable from the same workspace build."""
    executables = []
    with subprocess.Popen(command, text=True, encoding="utf-8", stdout=subprocess.PIPE) as process:
        for line in process.stdout:
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                print(line, end="", flush=True)
                continue
            if not isinstance(item, dict) or "reason" not in item:
                print(line, end="", flush=True)
            elif item["reason"] == "compiler-message":
                rendered = item["message"].get("rendered")
                if rendered:
                    print(rendered, end="", flush=True)
            elif item["reason"] == "compiler-artifact" and item["target"]["name"] == "native_window_lifecycle" and item.get("executable"):
                executables.append(Path(item["executable"]))
        if returncode := process.wait():
            raise subprocess.CalledProcessError(returncode, command)
    if len(executables) != 1 or executables[0].suffix != ".exe":
        raise RuntimeError("Cargo did not report exactly one Windows lifecycle executable")
    return executables[0]


def stage_lifecycle(destination: Path, executable: Path) -> None:
    """Copy the workspace build's lifecycle executable and any adjacent MSVC symbols."""
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executable, destination / "native_window_lifecycle.exe")
    # Keep MSVC's optional line-table symbols under the name recorded in the executable.
    symbols = executable.with_suffix(".pdb")
    if symbols.is_file():
        shutil.copy2(symbols, destination / symbols.name)
    for source in (Path("LICENSE-MIT"), Path("LICENSE-APACHE"), Path("crates/geometry/SOLIDCORE-NOTICE")):
        shutil.copy2(source, destination / source.name)


def main() -> None:
    """Propagate every command failure and publish an artifact only after selected tests pass."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    lint, workspace_tests, *harness_checks = commands()
    print("Running: " + " ".join(lint), flush=True)
    subprocess.run(lint, check=True)
    print("Running: " + " ".join(workspace_tests), flush=True)
    executable = run_workspace_tests(workspace_tests)
    for command in harness_checks:
        print("Running: " + " ".join(command), flush=True)
        subprocess.run(command, check=True)
    stage_lifecycle(args.artifacts, executable)


if __name__ == "__main__":
    main()
