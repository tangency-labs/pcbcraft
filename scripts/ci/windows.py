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


def stage_lifecycle(destination: Path) -> None:
    """Read Cargo's exact executable path; ambiguous/missing artifacts fail instead of globbing."""
    command = ["cargo", "test", "--locked", "--profile", "ci", "-p", "pcbcraft", "--test",
               "native_window_lifecycle", "--no-run", "--message-format=json"]
    result = subprocess.run(command, text=True, encoding="utf-8", stdout=subprocess.PIPE, check=True)
    executables = []
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "native_window_lifecycle" and item.get("executable"):
            executables.append(Path(item["executable"]))
        elif item.get("reason") == "compiler-message":
            print(item.get("message", {}).get("rendered", ""))
    if len(executables) != 1 or executables[0].suffix != ".exe":
        raise RuntimeError("Cargo did not report exactly one Windows lifecycle executable")
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executables[0], destination / "native_window_lifecycle.exe")
    # Keep MSVC's optional line-table symbols under the name recorded in the executable.
    symbols = executables[0].with_suffix(".pdb")
    if symbols.is_file():
        shutil.copy2(symbols, destination / symbols.name)
    for source in (Path("LICENSE-MIT"), Path("LICENSE-APACHE"), Path("crates/geometry/SOLIDCORE-NOTICE")):
        shutil.copy2(source, destination / source.name)


def main() -> None:
    """Propagate every command failure and publish an artifact only after selected tests pass."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    for command in commands():
        print("Running: " + " ".join(command), flush=True)
        subprocess.run(command, check=True)
    stage_lifecycle(args.artifacts)


if __name__ == "__main__":
    main()
