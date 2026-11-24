# Schematic PDF writer

`pcbcraft_engine::schematic_pdf::export` takes an immutable `Project` and an
explicit `SheetTarget` and returns a one-page PDF 1.4 document as bytes.
`pcbcraft_engine::schematic_pdf::export_project` takes the same immutable
project and returns a PDF with one page per sheet occurrence. Both are engine
APIs, not File menu actions or MCP commands. The caller decides whether and
where to write the bytes. Export does not change the native project format.

## Page order and bookmarks

Export visits root sheets in project sheet order. It emits each page before
its descendants, visiting sibling branches by ascending ID of the sheet symbol
in their parent. The bookmark tree follows that same order and nesting. Each
bookmark shows the sheet name and opens its own page. In project export, the
Default frame's page number and total count the emitted occurrences in PDF order.
Addressed single-sheet export keeps the editor's Browser numbering. The active
editor sheet does not affect the order or bytes.

The exporter associates pages with occurrences, not unique sheet definitions.
PCBcraft's [current project model](../formats/sheet-occurrence-storage.md) still rejects
repeated definitions; exporting a natively authored repeated sheet is a
follow-up once the project model supports it.

## Page contents

The sheet's [paper record](../formats/native-project-storage.md#sheet-paper-records)
sets the page size and origin; the [frame and parameters](../design/sheet-frames-and-parameters.md)
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
original search text. Output uses fixed black ink. On-page navigation links,
additional font faces, PDF/A, styles and variants are not implemented here.
