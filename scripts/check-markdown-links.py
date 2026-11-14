"""Check repository-relative Markdown links without network access.

Supported syntax: inline links/images, full and collapsed reference links, shortcut
references with definitions, angle-bracket destinations, and HTML href/src attributes.
Fenced and inline code, URLs, root-relative paths, and fragment-only links are skipped.
This is not a general Markdown renderer or an anchor-heading validator.
"""
from __future__ import annotations

import html
from bisect import bisect_right
import re
import string
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit

FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
REFERENCE_DEF = re.compile(r"^ {0,3}\[([^]\n]+)\]:\s*(<[^>\n]+>|\S+)")
REFERENCE_LABEL = re.compile(r"\[((?:\\[^\n]|[^\[\]\\\n])*)\]")
INLINE_TOKEN = re.compile(r"<|!?\[")
# Match complete attributes, including quoted values with spaces, newlines or '>'.
# Exact attribute names avoid treating data-href or words in titles as links.
HTML_ATTRIBUTE = re.compile(
    r"""[ \t\n]+([A-Za-z_:][A-Za-z0-9_.:-]*)(?:[ \t\n]*=[ \t\n]*("[^"]*"|'[^']*'|[^ \t\n"'=<>`]+))?"""
)
HTML_TAG = re.compile(r"<[A-Za-z][A-Za-z0-9-]*(?:" + HTML_ATTRIBUTE.pattern + r")*[ \t\n]*/?>")
# Unlike forgiving HTML5 entity parsing, CommonMark only decodes entities with ';'.
MARKDOWN_ENTITY = re.compile(r"&(?:#[0-9]+;|#[xX][0-9a-fA-F]+;|[A-Za-z][A-Za-z0-9]+;)")
# These interrupt or delimit inline parsing even when no blank line precedes them.
ATX_HEADING = re.compile(r"^ {0,3}#{1,6}(?:[ \t]|$)")
THEMATIC_BREAK = re.compile(r"^ {0,3}(?:(?:\*[ \t]*){3,}|(?:_[ \t]*){3,}|(?:-[ \t]*){3,})$")
BLOCKQUOTE = re.compile(r"^ {0,3}>")
LIST_ITEM = re.compile(r"^ {0,3}(?:[-+*]|[0-9]{1,9}[.)])[ \t]+")


def _escaped(text: str, index: int) -> bool:
    """Whether a Markdown delimiter is preceded by an odd number of slashes."""
    count = 0
    while index > count and text[index - count - 1] == "\\":
        count += 1
    return count % 2 == 1


def _masked(text: str) -> str:
    """Mask comments and exact-length inline code spans; keep newlines intact."""
    text = re.sub(r"<!--[\s\S]*?-->", lambda m: re.sub(r"[^\n]", " ", m.group()), text)
    out = list(text)
    runs = list(re.finditer(r"`+", text))
    index = 0
    while index < len(runs):
        opener = runs[index]
        if _escaped(text, opener.start()):
            index += 1
            continue
        closer_index = index + 1
        while closer_index < len(runs):
            candidate = runs[closer_index]
            # Backslashes are literal within a code span: they cannot escape
            # a closing backtick run. Only opening delimiters are escapable.
            if len(candidate.group()) == len(opener.group()):
                break
            closer_index += 1
        if closer_index == len(runs):
            index += 1
            continue
        for position in range(opener.start(), runs[closer_index].end()):
            if text[position] != "\n":
                out[position] = " "
        index = closer_index + 1
    return "".join(out)


def _mask_blocks(lines: list[str]) -> list[tuple[int, str]]:
    """Mask inline code separately for each block, retaining source line numbers.

    Every returned text represents one inline Markdown block. In particular,
    an inline link cannot finish in a following heading or list item. Prefixes
    of quoted lines become spaces so link destinations can continue on later
    quoted lines without changing diagnostics.
    """
    blocks: list[tuple[int, str]] = []
    pending: list[str] = []
    first_line = 1
    kind = "paragraph"

    def add(line: str, number: int) -> None:
        nonlocal first_line
        if not pending:
            first_line = number
        pending.append(line)

    def flush() -> None:
        if pending:
            blocks.append((first_line, _masked("\n".join(pending))))
            pending.clear()

    for number, line in enumerate(lines, 1):
        if not line.strip():
            flush()
            kind = "paragraph"
        elif ATX_HEADING.match(line) or THEMATIC_BREAK.match(line):
            flush()
            blocks.append((number, _masked(line)))
            kind = "paragraph"
        elif (prefix := re.match(r"^(?: {0,3}>[ \t]?)+", line)):
            # Strip the container marker but preserve column offsets in-place.
            content = line[prefix.end():]
            visible = " " * prefix.end() + content
            if ATX_HEADING.match(content) or THEMATIC_BREAK.match(content):
                flush()
                blocks.append((number, _masked(visible)))
                kind = "quoted_nonparagraph"
            else:
                if kind != "blockquote":
                    flush()
                add(visible, number)
                kind = "blockquote"
        elif (item := LIST_ITEM.match(line)):
            # Only ordered lists starting at 1 can interrupt a paragraph.
            ordered = re.match(r"[0-9]{1,9}[.)]", item.group().lstrip())
            if pending and kind == "paragraph" and ordered and int(ordered.group()[:-1]) != 1:
                add(line, number)
            else:
                flush()
                add(line, number)
                kind = "list"
        else:
            # Lazy continuations of paragraph content in a blockquote, or of
            # an existing list-item paragraph, remain in the same block.
            add(line, number)
            if kind not in ("list", "blockquote"):
                kind = "paragraph"
    flush()
    return blocks


def _visible_blocks(text: str) -> list[tuple[int, str]]:
    """Skip fenced code and comments, keeping one block per inline scope."""
    fence_char = ""
    fence_len = 0
    visible: list[str] = []
    without_comments = re.sub(r"<!--[\s\S]*?-->",
                              lambda m: re.sub(r"[^\n]", " ", m.group()), text)
    for line in without_comments.splitlines():
        match = FENCE.match(line)
        if fence_char:
            if (match and match.group(1)[0] == fence_char
                    and len(match.group(1)) >= fence_len
                    and not line[match.end():].strip()):
                fence_char = ""
            visible.append("")
        elif match:
            fence_char, fence_len = match.group(1)[0], len(match.group(1))
            visible.append("")
        else:
            visible.append(line)
    return _mask_blocks(visible)


def _normalized(label: str) -> str:
    return " ".join(label.split()).casefold()


def _label_end(text: str, position: int) -> int | None:
    """Find the closing bracket of a balanced, possibly escaped link label."""
    depth = 1
    while position < len(text):
        if not _escaped(text, position):
            if text[position] == "<" and (tag := HTML_TAG.match(text, position)):
                position = tag.end()
                continue
            if text[position] == "[":
                depth += 1
            elif text[position] == "]":
                depth -= 1
                if depth == 0:
                    return position
        position += 1
    return None


def _link_spaces(text: str, pos: int) -> int | None:
    """Consume spaces/tabs and at most one line ending at a link boundary."""
    while pos < len(text) and text[pos] in " \t":
        pos += 1
    if pos < len(text) and text[pos] == "\n":
        pos += 1
        while pos < len(text) and text[pos] in " \t":
            pos += 1
        if pos < len(text) and text[pos] == "\n":
            return None
    return pos


def _destination(text: str, index: int) -> tuple[str, int] | None:
    """Read an inline destination and optional title across one line break.

    The parser intentionally covers documentation links rather than all Markdown,
    but a destination/title split across lines must still be checked.
    """
    pos = _link_spaces(text, index)
    if pos is None or pos >= len(text):
        return None
    if text[pos] == "<":
        start = pos + 1
        pos = start
        while pos < len(text):
            if text[pos] == "\n":
                return None
            if text[pos] == ">" and not _escaped(text, pos):
                break
            pos += 1
        if pos == len(text):
            return None
        value = text[start:pos]
        pos += 1
    else:
        start = pos
        depth = 0
        while pos < len(text):
            char = text[pos]
            if char in " \t\n" and depth == 0:
                break
            if char == "\n":
                return None
            if char in "()" and not _escaped(text, pos):
                if char == "(":
                    depth += 1
                elif depth == 0:
                    break
                else:
                    depth -= 1
            pos += 1
        if depth:
            return None
        value = text[start:pos]
    had_space = pos < len(text) and text[pos] in " \t\n"
    new_pos = _link_spaces(text, pos)
    if new_pos is None:
        return None
    pos = new_pos
    if had_space and pos < len(text) and text[pos] in "\"' (":
        opener = text[pos]
        closer = ")" if opener == "(" else opener
        pos += 1
        while pos < len(text) and (text[pos] != closer or _escaped(text, pos)):
            if text[pos] == "\n" and text[pos + 1:].lstrip(" \t").startswith("\n"):
                return None
            pos += 1
        if pos >= len(text):
            return None
        pos = _link_spaces(text, pos + 1)
        if pos is None:
            return None
    if pos >= len(text) or text[pos] != ")":
        return None
    return value, pos + 1


def _local_target(repo: Path, source: Path, target: str, *, html_attribute: bool = False) -> bool:
    target = target.strip()
    if html_attribute:
        target = html.unescape(target)
    else:
        target = MARKDOWN_ENTITY.sub(lambda match: html.unescape(match.group()), target)
    # Markdown backslash escapes punctuation in destinations; URL escaping is
    # handled separately after the query/fragment have been stripped.
    target = re.sub(r"\\([" + re.escape(string.punctuation) + r"])", r"\1", target)
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


def _scan_targets(text: str, references: dict[str, str], *, images_only: bool = False) -> list[tuple[int, str, bool]]:
    """Read links and HTML tags in source order, retaining destination offsets.

    Consume a complete link before interpreting its destination as HTML or
    scanning markup inside its title. Scan a link label separately for images
    (including reference images) and HTML tags, but not nested links.
    """
    targets: list[tuple[int, str, bool]] = []
    position = 0
    while (marker := INLINE_TOKEN.search(text, position)) is not None:
        start = marker.start()
        position = marker.end()
        if marker.group() == "<":
            if _escaped(text, start):
                continue
            tag = HTML_TAG.match(text, start)
            if tag:
                for attr in HTML_ATTRIBUTE.finditer(tag.group()):
                    value = attr.group(2)
                    if attr.group(1).lower() in ("href", "src") and value is not None:
                        if value.startswith(('"', "'")):
                            value = value[1:-1]
                        targets.append((start + attr.start(1), value, True))
                position = tag.end()
            continue
        image = marker.group() == "![" and not _escaped(text, start)
        if not image:
            start = marker.end() - 1
        if _escaped(text, start) or (images_only and not image):
            continue
        label_end = _label_end(text, position)
        if label_end is None:
            continue
        label = text[position:label_end]
        end = label_end + 1
        found = _destination(text, end + 1) if text[end:end + 1] == "(" else None
        if found is not None:
            target, end = found
        else:
            reference = REFERENCE_LABEL.match(text, end)
            if reference:
                key = _normalized(reference.group(1) or label)
                end = reference.end()
            elif REFERENCE_LABEL.fullmatch(text[position - 1:label_end + 1]):
                key = _normalized(label)
            else:
                continue
            if key not in references:
                continue
            target = references[key]
        targets.append((start, target, False))
        if not image:
            targets.extend((position + offset, value, is_html)
                           for offset, value, is_html in _scan_targets(label, references, images_only=True))
        position = end
    return targets


def find_broken_links(repo: Path, files: list[Path]) -> list[str]:
    """Return source:line diagnostics, retaining the start of multiline links."""
    repo = repo.resolve()
    problems: list[str] = []
    for source in files:
        blocks = _visible_blocks(source.read_text(encoding="utf-8"))
        references: dict[str, str] = {}
        scan_blocks: list[tuple[int, str]] = []
        for first_line, block in blocks:
            scan_lines: list[str] = []
            for line in block.split("\n"):
                match = REFERENCE_DEF.match(line)
                if match:
                    dest = match.group(2)
                    references.setdefault(_normalized(match.group(1)),
                                          dest[1:-1] if dest.startswith("<") else dest)
                    scan_lines.append("")
                else:
                    scan_lines.append(line)
            scan_blocks.append((first_line, "\n".join(scan_lines)))
        for first_line, text in scan_blocks:
            newlines = [index for index, char in enumerate(text) if char == "\n"]
            targets = _scan_targets(text, references)
            for offset, target, html_attribute in sorted(targets):
                if not _local_target(repo, source, target, html_attribute=html_attribute):
                    number = first_line + bisect_right(newlines, offset)
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
