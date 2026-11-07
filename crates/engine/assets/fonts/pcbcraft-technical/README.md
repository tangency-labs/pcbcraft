# PCBcraft Technical

`strokes.json` is a renamed, converted font-data derivative of **Relief SingleLine
SVG Regular**, copyright 2022 The Relief SingleLine Project Authors. The font data
remains under **SIL Open Font License 1.1**, with the full license, authors and
contributors retained beside it and embedded in the JSON font metadata. PCBcraft's Rust layout code and Python converter
are original MIT OR Apache-2.0 code; the font data is not relicensed with them.

The upstream font asset is
[`fonts/open_svg/ReliefSingleLineSVG-Regular.svg`](https://github.com/isdat-type/Relief-SingleLine/blob/01dfc5779ec1e9e4b288d96c6c96c23bfccbaf9d/fonts/open_svg/ReliefSingleLineSVG-Regular.svg)
at revision `01dfc5779ec1e9e4b288d96c6c96c23bfccbaf9d` of
[isdat-type/Relief-SingleLine](https://github.com/isdat-type/Relief-SingleLine).
Its SHA-256 is
`75f05a5b64ed6039c9816628ee051d98e16c19148a8268c63f5eccf8382479e2`.
Only that font asset and its license notices were used; no upstream implementation
or KiCad data/code was incorporated.

The original converter flattens cubic Bézier centerlines to polylines, with a
0.5 source-unit control-polygon tolerance and bounded subdivision. It retains 422
single-character Unicode glyphs and proportional advance widths. The output is
renamed **PCBcraft Technical**, revision 1 (`pcbcraft-technical-1`), to distinguish
this converted data from the upstream font. OpenType shaping, ligatures and the
upstream kerning table are not included; the layout enforces a minimum physical
ink gap and uses the font's question mark for unsupported characters.

To reproduce from the pinned, separately downloaded SVG, run from the repository
root:

```sh
python3 tools/prepare_technical_font.py /path/to/ReliefSingleLineSVG-Regular.svg crates/engine/assets/fonts/pcbcraft-technical/strokes.json
```

The converter verifies the input hash before writing deterministic JSON. Runtime
layout needs neither the source SVG, Python, installed fonts nor network access.
The immutable font revision is a native drawing contract; later glyph changes
require a new font identity. The earlier original `pcbcraft-stroke-1` remains
available for projects that pinned its reference style.
