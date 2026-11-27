"""Run native Clippy and tests for affected workspace packages and their dependents."""
import json
import os
from pathlib import Path
import subprocess

if __package__:
    from .policy import changed_paths, select
else:
    from policy import changed_paths, select


def affected(metadata: dict, paths: list[str] | None) -> list[str] | None:
    """Resolve package ownership and reverse dependencies; unknown build inputs select all.

    Cargo metadata includes normal, build, dev and target-specific dependencies. Renamed
    dependencies use their original package names. Missing/removed owners or manifests,
    scripts, fixtures and other unknown inputs conservatively select the workspace.
    """
    if paths is None:
        return None
    root = Path(metadata["workspace_root"]).resolve()
    members = set(metadata["workspace_members"])
    packages = [package for package in metadata["packages"] if package["id"] in members]
    names = {package["name"] for package in packages}
    selected = set()
    for path in paths:
        if not any(select("pull_request", [path]).values()):
            continue
        item = Path(path)
        if item.is_absolute() or ".." in item.parts or "\\" in path or not path.endswith(".rs") or item.name == "build.rs":
            return None
        absolute = root / item
        owners = [package["name"] for package in packages
                  if absolute.is_relative_to(Path(package["manifest_path"]).resolve().parent)]
        if len(owners) != 1:
            return None
        selected.update(owners)
    if not selected:
        return None
    while True:
        expanded = selected | {package["name"] for package in packages
                               if any(dep["name"] in selected for dep in package["dependencies"] if dep["name"] in names)}
        if expanded == selected:
            return sorted(selected)
        selected = expanded


def commands(packages: list[str] | None, full: bool) -> list[list[str]]:
    """PRs run selected unit/integration tests; full events also run doctests and the harness."""
    scope = ["--workspace"] if packages is None else [arg for name in packages for arg in ("-p", name)]
    if not scope:
        raise ValueError("Empty package selection cannot qualify native checks")
    base = ["--locked", "--profile", "ci"]
    result = [["cargo", "clippy", *base, *scope, "--all-targets", "--", "-D", "warnings"],
              ["cargo", "test", *base, *scope, *([] if full else ["--tests"])]]
    if full:
        result += [["cargo", "clippy", *base, "-p", "pcbcraft-app", "--features", "performance-harness", "--all-targets", "--", "-D", "warnings"],
                   ["cargo", "test", *base, "-p", "pcbcraft-app", "--features", "performance-harness", "performance::"]]
    return result


def main() -> None:
    """Fail on metadata or command errors, without silently reducing the selected coverage."""
    mode = os.environ["CI_MODE"]
    if mode not in {"approved", "full"}:
        raise ValueError("Native checks require an approved or full stage")
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], text=True))
    packages = None
    if mode == "approved":
        payload = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
        packages = affected(metadata, changed_paths("pull_request", payload))
        if packages is not None and os.environ.get("PCBCRAFT_COMMAND_CONTRACT_BASE"):
            packages = sorted(set(packages) | {"pcbcraft-engine"})
    print("Native packages: " + ("whole workspace" if packages is None else ", ".join(packages)), flush=True)
    for command in commands(packages, mode == "full"):
        print("Running: " + " ".join(command), flush=True)
        subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
