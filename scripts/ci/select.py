"""Select jobs conservatively; missing comparisons and unknown paths run every check."""
from __future__ import annotations

import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess

OUTPUTS = ("native", "windows", "windows_full", "web", "wasi", "licenses")
LIGHT_ROOT = frozenset({"README.md", "CONTRIBUTING.md", "VISION.md", "AGENTS.md",
                        "ATTRIBUTION.md", "CHANGELOG.md", "LICENSE-MIT", "LICENSE-APACHE"})
WINDOWS_PREFIXES = ("apps/pcbcraft/", "crates/app/", "crates/chrome/", "crates/storage/",
                    "crates/mcp/", "crates/craft-control/", "crates/craft-agents/",
                    "crates/engine/src/persistence", "crates/engine/src/library",
                    "crates/engine/src/clipboard", "crates/engine/src/lib.rs")
PACKAGES = frozenset({"app", "chrome", "engine", "geometry", "storage", "mcp", "units",
                      "craft-control", "craft-agents"})


def full() -> dict[str, bool]:
    """Every check, including the complete Windows suite, for qualification or uncertainty."""
    return dict.fromkeys(OUTPUTS, True)


def select(event: str, paths: list[str] | None) -> dict[str, bool]:
    """Known docs avoid builds; known code selects targets; configuration/unknown paths run all.

    Main validates all platforms after code lands because the repository need not use a merge
    queue. PR Windows checks focus on native integration. Nightly/manual/merge groups run all.
    """
    if event not in {"pull_request", "push"} or paths is None:
        return full()
    result = dict.fromkeys(OUTPUTS, False)
    for path in paths:
        parts = PurePosixPath(path).parts
        if not parts or path.startswith("/") or ".." in parts or "\\" in path:
            return full()
        if path in LIGHT_ROOT or (path.startswith("docs/") and path.endswith(".md")):
            continue
        if path.startswith(".github/ISSUE_TEMPLATE/") or path in {".github/pull_request_template.md", ".github/CODEOWNERS", ".github/FUNDING.yml"}:
            continue
        if path in {"Cargo.toml", "Cargo.lock", "deny.toml"} or path.endswith("/Cargo.toml"):
            return full()
        if path.startswith(("rust-toolchain", ".cargo/", ".github/workflows/", "scripts/", "vendor/")):
            return full()
        known = ((len(parts) > 2 and parts[0] == "crates" and parts[1] in PACKAGES)
                 or (len(parts) > 2 and parts[0] == "apps" and parts[1] in {"pcbcraft", "pcbcraft-cli", "pcbcraft-web"}))
        if not known:
            return full()
        result["native"] = True
        # Compile/package the web app for shared/application code, retaining its current coverage.
        result["web"] = True
        if path.startswith(("crates/engine/", "crates/geometry/", "crates/units/")):
            result["wasi"] = True
        if path.startswith(WINDOWS_PREFIXES):
            result["windows"] = True
    if event == "push" and result["native"]:
        return full()
    return result


def changed_paths(event: str, payload: dict, cwd: Path | None = None) -> list[str] | None:
    """Read both sides of renames; invalid/missing Git history requests full validation."""
    if event == "pull_request":
        base = payload.get("pull_request", {}).get("base", {}).get("sha", "")
        head = payload.get("pull_request", {}).get("head", {}).get("sha", "")
        separator = "..."
    elif event == "push":
        base, head, separator = payload.get("before", ""), payload.get("after", ""), ".."
    else:
        return None
    if not all(re.fullmatch(r"[0-9a-fA-F]{40}", sha) and int(sha, 16) for sha in (base, head)):
        return None
    try:
        output = subprocess.check_output(["git", "diff", "--no-renames", "--name-only", "-z", base + separator + head, "--"], stderr=subprocess.PIPE, cwd=cwd)
    except subprocess.CalledProcessError:
        return None
    return [p.decode("utf-8", errors="surrogateescape") for p in output.split(b"\0") if p]


def main() -> None:
    """Emit only fixed boolean keys to GitHub's output file; filenames never become commands."""
    event = os.environ["GITHUB_EVENT_NAME"]
    payload = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
    result = select(event, changed_paths(event, payload))
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        for key, value in result.items():
            output.write(f"{key}={str(value).lower()}\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
