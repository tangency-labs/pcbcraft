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
            base + ["--workspace"],
            ["cargo", "clippy", "--locked", "--profile", "ci", "-p", "pcbcraft-app", "--features", "performance-harness", "--all-targets", "--", "-D", "warnings"],
            base + ["-p", "pcbcraft-app", "--features", "performance-harness", "performance::"]]


def cargo_executable(command: list[str], name: str) -> Path:
    """Require Cargo to name exactly one matching Windows executable."""
    result = subprocess.run(command, text=True, encoding="utf-8", stdout=subprocess.PIPE, check=True)
    executables = []
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == name and item.get("executable"):
            executables.append(Path(item["executable"]))
        elif item.get("reason") == "compiler-message":
            print(item.get("message", {}).get("rendered", ""))
    if len(executables) != 1 or executables[0].suffix != ".exe":
        raise RuntimeError(f"Cargo did not report exactly one Windows {name} executable")
    return executables[0]


def stage_lifecycle(destination: Path) -> None:
    """Read Cargo's exact executable path; ambiguous/missing artifacts fail instead of globbing."""
    command = ["cargo", "test", "--locked", "--profile", "ci", "-p", "pcbcraft", "--test",
               "native_window_lifecycle", "--no-run", "--message-format=json"]
    executable = cargo_executable(command, "native_window_lifecycle")
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executable, destination / "native_window_lifecycle.exe")
    # Keep MSVC's optional line-table symbols under the name recorded in the executable.
    symbols = executable.with_suffix(".pdb")
    if symbols.is_file():
        shutil.copy2(symbols, destination / symbols.name)
    for source in (Path("LICENSE-MIT"), Path("LICENSE-APACHE"), Path("crates/geometry/SOLIDCORE-NOTICE")):
        shutil.copy2(source, destination / source.name)


def stage_startup_launcher(destination: Path) -> None:
    """Exercise release subsystem selection while reusing the CI dependency builds."""
    command = ["cargo", "rustc", "--locked", "--profile", "ci", "-p", "pcbcraft", "--bin", "pcbcraft",
               "--message-format=json", "--", "-C", "debug-assertions=off"]
    executable = cargo_executable(command, "pcbcraft")
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executable, destination / "pcbcraft.exe")


def main() -> None:
    """Propagate every command failure and publish an artifact only after selected tests pass."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    for command in commands():
        print("Running: " + " ".join(command), flush=True)
        subprocess.run(command, check=True)
    stage_lifecycle(args.artifacts)
    stage_startup_launcher(args.artifacts)


if __name__ == "__main__":
    main()
