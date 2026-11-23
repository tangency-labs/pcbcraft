# Schematic PDF writer

`pcbcraft_engine::schematic_pdf::export` takes an immutable `Project` and an
explicit `SheetTarget` and returns one complete PDF 1.4 document as bytes. This
is an engine API, not a File menu action or an MCP command. The caller owns
where, whether and how to publish the bytes. No native project format changes.

The sheet's [paper record](native-project-storage.md#sheet-paper-records)
sets the page size and origin; the [frame and parameters](sheet-frames-and-parameters.md)
come from the same resolver used on the canvas. The paper clips the drawing;
export does not fit or move off-paper objects. Unknown sheet addresses and legacy
sheets without adopted paper return an error rather than printing the current
sheet or making up a paper size.

Symbols, wires and frame strokes are vector paths. Visible text, including
reference designators and named net labels, is PDF text mapped to the original
Unicode for searching and copying. As on the canvas, a quarter-turned symbol's
unmoved Reference and Value sit to the right of its body, not on its pin axis.
The regular stroke face is already bundled
with the engine; characters it cannot draw appear as `?` while retaining their
original search text. This slice uses fixed black ink. Multi-page output,
bookmarks, links, additional font faces, PDF/A, styles and variants are not
implemented here.
