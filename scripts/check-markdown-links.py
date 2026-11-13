"""Check repository-relative Markdown links without network access.

Supported syntax: inline links/images, full and collapsed reference links, shortcut
references with definitions, angle-bracket destinations, and HTML href/src attributes.
Fenced and inline code, URLs, root-relative paths, and fragment-only links are skipped.
This is not a general Markdown renderer or an anchor-heading validator.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit

FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
REFERENCE_DEF = re.compile(r"^ {0,3}\[([^]\n]+)\]:\s*(<[^>\n]+>|\S+)")
INLINE_START = re.compile(r"!?\[[^]\n]+\]\(")
REFERENCE_USAGE = re.compile(r"!?\[([^]\n]+)\]\[([^]\n]*)\]")
SHORTCUT_USAGE = re.compile(r"(?<!!)\[([^]\n]+)\]")
HTML_DEST = re.compile(r"\b(?:href|src)\s*=\s*([\"'])(.*?)\1", re.IGNORECASE)


def _masked(text: str) -> str:
    """Mask comments and inline code while retaining positions and newlines."""
    text = re.sub(r"<!--[\s\S]*?-->", lambda m: re.sub(r"[^\n]", " ", m.group()), text)
    out: list[str] = []
    index = 0
    while index < len(text):
        if text[index] != "`":
            out.append(text[index])
            index += 1
            continue
        end = index
        while end < len(text) and text[end] == "`":
            end += 1
        marker = text[index:end]
        closing = text.find(marker, end)
        if closing == -1:
            out.append(text[index:end])
            index = end
            continue
        segment = text[index : closing + len(marker)]
        out.append(re.sub(r"[^\n]", " ", segment))
        index = closing + len(marker)
    return "".join(out)


def _visible_lines(text: str) -> list[str]:
    """Ignore fenced blocks (including their opening and closing markers)."""
    fence_char = ""
    fence_len = 0
    result: list[str] = []
    without_comments = re.sub(r"<!--[\s\S]*?-->",
                              lambda m: re.sub(r"[^\n]", " ", m.group()), text)
    for line in without_comments.splitlines():
        match = FENCE.match(line)
        if fence_char:
            if match and match.group(1)[0] == fence_char and len(match.group(1)) >= fence_len and not line[match.end():].strip():
                fence_char = ""
            result.append("")
        elif match:
            fence_char, fence_len = match.group(1)[0], len(match.group(1))
            result.append("")
        else:
            result.append(_masked(line))
    return result


def _normalized(label: str) -> str:
    return " ".join(label.split()).casefold()


def _destination(line: str, index: int) -> tuple[str, int] | None:
    """Read an inline destination after '(', including nested path parentheses."""
    pos = index
    while pos < len(line) and line[pos].isspace():
        pos += 1
    if pos >= len(line):
        return None
    if line[pos] == "<":
        stop = line.find(">", pos + 1)
        if stop < 0:
            return None
        value = line[pos + 1: stop]
        pos = stop + 1
    else:
        start = pos
        depth = 0
        while pos < len(line):
            char = line[pos]
            if char.isspace() and depth == 0:
                break
            if char == "(":
                depth += 1
            elif char == ")":
                if depth == 0:
                    break
                depth -= 1
            pos += 1
        value = line[start:pos]
    tail = line[pos:]
    closing = re.match(r"\s*(?:[\"'][^\"']*[\"']\s*)?\)", tail)
    if closing is None:
        return None
    return value, pos + closing.end()


def _local_target(repo: Path, source: Path, target: str) -> bool:
    target = target.strip()
    if not target or target.startswith(("#", "/", "//")):
        return True
    parsed = urlsplit(target)
    if parsed.scheme or parsed.netloc:
        return True
    relative = unquote(parsed.path)
    if not relative:
        return True
    candidate = (source.parent / relative).resolve()
    # Repository-local links must not escape the checkout.
    return candidate.is_relative_to(repo) and candidate.exists()


def find_broken_links(repo: Path, files: list[Path]) -> list[str]:
    """Return actionable source:line diagnostics for missing local destinations."""
    repo = repo.resolve()
    problems: list[str] = []
    for source in files:
        lines = _visible_lines(source.read_text(encoding="utf-8"))
        references: dict[str, str] = {}
        for line in lines:
            match = REFERENCE_DEF.match(line)
            if match:
                dest = match.group(2)
                references.setdefault(_normalized(match.group(1)), dest[1:-1] if dest.startswith("<") else dest)
        for number, line in enumerate(lines, 1):
            if REFERENCE_DEF.match(line):
                continue
            targets: list[str] = []
            masked = list(line)
            for match in INLINE_START.finditer(line):
                found = _destination(line, match.end())
                if not found:
                    continue
                target, end = found
                targets.append(target)
                masked[match.start():end] = " " * (end - match.start())
            remainder = "".join(masked)
            for match in REFERENCE_USAGE.finditer(remainder):
                label = _normalized(match.group(2) or match.group(1))
                if label in references:
                    targets.append(references[label])
                masked[match.start():match.end()] = " " * (match.end() - match.start())
            remainder = "".join(masked)
            for match in SHORTCUT_USAGE.finditer(remainder):
                label = _normalized(match.group(1))
                if label in references:
                    targets.append(references[label])
            for match in HTML_DEST.finditer(line):
                targets.append(match.group(2))
            for target in targets:
                if not _local_target(repo, source, target):
                    problems.append(f"{source.relative_to(repo)}:{number}: missing local link target: {target}")
    return problems


def main(argv: list[str] | None = None) -> int:
    """Check documentation relative to the repository root and return 1 on errors."""
    if argv is None:
        argv = sys.argv[1:]
    if argv:
        print("Usage: python scripts/check-markdown-links.py (from repository root)", file=sys.stderr)
        return 2
    repo = Path(__file__).resolve().parent.parent
    files = [p for p in (repo / "README.md", repo / "CONTRIBUTING.md") if p.is_file()]
    files.extend(sorted((repo / "docs").rglob("*.md")))
    problems = find_broken_links(repo, files)
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        print(f"Found {len(problems)} broken local link(s).", file=sys.stderr)
        return 1
    print(f"Checked {len(files)} Markdown file(s): no broken local links.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
