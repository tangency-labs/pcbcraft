"""Select jobs conservatively; missing comparisons and unknown paths run every check."""
from __future__ import annotations

import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess

OUTPUTS = ("native", "windows", "windows_full", "web", "wasi", "licenses")
APPROVAL = "approved-for-ci"
LIGHT_ROOT = frozenset({"README.md", "CONTRIBUTING.md", "VISION.md", "AGENTS.md",
                        "ATTRIBUTION.md", "CHANGELOG.md", "LICENSE-MIT", "LICENSE-APACHE"})


def full() -> dict[str, bool]:
    """Every check, including the complete Windows suite, for qualification or uncertainty."""
    return dict.fromkeys(OUTPUTS, True)


def select(event: str, paths: list[str] | None) -> dict[str, bool]:
    """Only recognized documentation avoids builds; stage selection defers PR platform checks.

    Staging is applied separately so a feedback run never claims full validation.
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
        # Sources, fixtures, manifests and unknown paths are build inputs. The PR stage
        # decides between quick compilation and native checks; other events run all platforms.
        return full()
    return result


def plan(event: str, payload: dict, paths: list[str] | None) -> dict[str, str]:
    """PRs get quick feedback or approved native tests; other build events cover all platforms.

    Drafts do no compilation. Unrelated label events carry only an approved native pass for the
    same head, instead of repeating builds or accepting skipped qualification jobs.
    """
    selected = select(event, paths)
    mode = "full" if any(selected.values()) else "docs"
    quick = False
    if event == "pull_request":
        pr = payload.get("pull_request", {})
        approved = any(label.get("name") == APPROVAL for label in pr.get("labels", []))
        action = payload.get("action")
        if pr.get("draft"):
            mode, selected = "draft", dict.fromkeys(OUTPUTS, False)
        elif any(selected.values()):
            if action in {"labeled", "unlabeled"} and payload.get("label", {}).get("name") != APPROVAL and approved:
                mode, selected = "carry", dict.fromkeys(OUTPUTS, False)
            elif approved:
                mode, selected = "approved", dict.fromkeys(OUTPUTS, False)
                selected["native"] = True
            else:
                mode, selected, quick = "quick", dict.fromkeys(OUTPUTS, False), True
            if mode in {"quick", "approved"}:
                selected["licenses"] = paths is None or any(
                    path in {"Cargo.toml", "Cargo.lock", "deny.toml"} or path.endswith("/Cargo.toml")
                    for path in paths)
    return {**{key: str(value).lower() for key, value in selected.items()},
            "quick": str(quick).lower(), "mode": mode}


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
    result = plan(event, payload, changed_paths(event, payload))
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        for key, value in result.items():
            output.write(f"{key}={value}\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
