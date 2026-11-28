"""Build the searchable mdBook from the repository's canonical Markdown guides."""

import posixpath
import re
import shutil
import subprocess
import sys
from html import escape
from html.parser import HTMLParser
from pathlib import Path, PurePosixPath
from tempfile import TemporaryDirectory
from urllib.parse import quote, unquote, urlsplit, urlunsplit


BOOK = Path(__file__).resolve().parent
ROOT = BOOK.parent.parent
OUTPUT = ROOT / "target" / "docs-book"
REPOSITORY = "https://github.com/tangency-labs/pcbcraft"
INTRODUCTION = PurePosixPath("docs/book/introduction.html")
SUMMARY_LINK = re.compile(r"(?<=\]\()([^\s()]+)(?=\)\s*$)", re.MULTILINE)
EXAMPLE = "examples/dual-led.pc"


def stage_book(stage: Path) -> None:
    """Copy the listed chapters and assets with their repository-relative paths."""
    source = stage / "src"
    source.mkdir()
    shutil.copyfile(BOOK / "book.toml", stage / "book.toml")

    def stage_chapter(match: re.Match[str]) -> str:
        chapter = (BOOK / match[0]).resolve()
        relative = chapter.relative_to(ROOT)
        destination = source / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(chapter, destination)
        return relative.as_posix()

    summary = (BOOK / "SUMMARY.md").read_text(encoding="utf-8")
    (source / "SUMMARY.md").write_text(SUMMARY_LINK.sub(stage_chapter, summary), encoding="utf-8")
    shutil.copytree(BOOK / "images", source / "docs/book/images")
    example = source / EXAMPLE
    example.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(ROOT / EXAMPLE, example)


def book_url(output: Path, page: Path, origin: PurePosixPath, link: str, *, image: bool) -> str:
    """Resolve local URLs and send links outside the book to the repository."""
    # The edit template includes mdBook's temporary source directory.
    edit_prefix = f"{REPOSITORY}/edit/main/"
    if link.startswith(f"{edit_prefix}src/"):
        return edit_prefix + link.removeprefix(f"{edit_prefix}src/")
    url = urlsplit(link)
    if url.scheme or url.netloc or not url.path or url.path.startswith("/"):
        return link

    relative = PurePosixPath(posixpath.normpath((origin.parent / unquote(url.path)).as_posix()))
    if (output / relative).is_file():
        if origin == page:
            return link
        path = quote(posixpath.relpath(relative, page.parent))
        return urlunsplit(("", "", path, url.query, url.fragment))
    if image:
        raise ValueError(f"{page}: missing book image {link!r}")

    # mdBook emits .html links for Markdown sources, including unlisted chapters.
    if relative.suffix == ".html" and not (ROOT / relative).exists():
        relative = relative.with_suffix(".md")
    target = (ROOT / relative).resolve()
    if not target.is_relative_to(ROOT) or not target.exists():
        raise ValueError(f"{page}: unresolved book link {link!r}")
    view = "tree" if target.is_dir() else "blob"
    path = quote(target.relative_to(ROOT).as_posix())
    return urlunsplit(("https", "github.com", f"{urlsplit(REPOSITORY).path}/{view}/main/{path}",
                       url.query, url.fragment))


class ChapterLinks(HTMLParser):
    """Collect replacements for rendered anchor and image URLs while preserving all other HTML."""

    def __init__(self, output: Path, page: Path, html: str) -> None:
        super().__init__()
        self.output = output
        self.page = page
        self.in_chapter = False
        self.starts = [0, *(match.end() for match in re.finditer("\n", html))]
        self.replacements: list[tuple[int, str, str]] = []
        self.feed(html)
        self.close()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == "main":
            self.in_chapter = True
        attribute = {"a": "href", "img": "src"}.get(tag)
        if attribute is None:
            return
        url = dict(attrs).get(attribute)
        if url is None:
            return
        # The generated homepage repeats the first chapter verbatim.
        origin = INTRODUCTION if self.in_chapter and self.page == Path("index.html") else self.page
        link = book_url(self.output, self.page, origin, url, image=tag == "img")
        if link == url:
            return
        line, column = self.getpos()
        start = self.starts[line - 1] + column
        original = self.get_starttag_text()
        updated = re.sub(rf"""(\s{attribute}\s*=\s*)(?:"[^"]*"|'[^']*'|[^\s>]+)""",
                         lambda match: f'{match[1]}"{escape(link, quote=True)}"',
                         original, count=1, flags=re.IGNORECASE)
        self.replacements.append((start, original, updated))

    def handle_endtag(self, tag: str) -> None:
        if tag == "main":
            self.in_chapter = False


def build() -> None:
    """Render into staging and validate repository links before replacing the preview."""
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(prefix="docs-book-", dir=OUTPUT.parent) as temporary:
        stage = Path(temporary)
        stage_book(stage)
        output = stage / "html"
        subprocess.run(["mdbook", "build", str(stage), "--dest-dir", str(output)], check=True)

        for file in output.rglob("*.html"):
            html = file.read_text(encoding="utf-8")
            links = ChapterLinks(output, file.relative_to(output), html)
            if links.replacements:
                for start, original, updated in reversed(links.replacements):
                    html = html[:start] + updated + html[start + len(original):]
                file.write_text(html, encoding="utf-8")

        if OUTPUT.exists():
            shutil.rmtree(OUTPUT)
        output.rename(OUTPUT)


if __name__ == "__main__":
    try:
        build()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(f"Book build failed: {error}")
    print(f"Book built at {OUTPUT / 'index.html'}")
