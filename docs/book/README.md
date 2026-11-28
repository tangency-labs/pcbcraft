# Build the PCBcraft documentation book

Install [mdBook 0.5.4](https://github.com/rust-lang/mdBook/releases/tag/v0.5.4)
and make sure `mdbook` is on your PATH. With Python 3.10 or newer, run these
commands from the repository root:

```sh
python -B docs/book/build.py
python -m http.server 8000 --bind 127.0.0.1 --directory target/docs-book
```

Open [the local preview](http://127.0.0.1:8000). Rerun the build after editing
a chapter and reload the preview.

[SUMMARY.md](SUMMARY.md) defines the sidebar and selects which guides go into
the book and its search index. Edit each guide in its existing location. The
build retains links between included chapters and serves the LED flasher
archive alongside the book. Links to other repository files point back to
GitHub.

The screenshots in [images/](images/) were captured from the native PCBcraft
editor with the bundled LED flasher and separate practice schematics. The
Manufacturing screenshot shows a ZIP built from the flasher's headless
fabrication export. For updates, use an isolated settings profile
and disposable project copies. Capture the state described in the nearby
instructions and crop to the relevant controls when that improves legibility.
Each picture links to the full-size image.
