# Sheet frames, parameters and templates

**Status: proposal, partly implemented.** PR 1 (paper on the sheet), PR 2
(parameters) and PR 3 (bundled templates) are implemented; their storage
contracts are in [native project storage](../formats/native-project-storage.md#sheet-paper-records)
and [parameter records](../formats/native-project-storage.md#parameter-records). PR 4 is a
proposal. Design agreed November 15, 2026 for
[#46](https://github.com/tangency-labs/pcbcraft/issues/46). Interactive
mockup (private):
<https://claude.ai/code/artifact/7119e9c8-def3-4fb0-a498-3e2a431ef947>.

Schematic sheets get paper (A4 to A0, landscape or portrait), a border with
zone markers and a title block. The title block shows parameter values such as
`@sheet.Title`. Most of the model is ported from Coline.

## Current state

- Sheets have paper (PR 1): a size, orientation and top-left corner, stored as
  sheet/9. The schematic range stays ±1 m around the origin (`MAX_CENTER_NM` in
  `crates/engine/src/schematic.rs`), and paper must fit inside it.
- Sheets have a frame (PR 3): the page record names a template by library, name
  and revision, or none for a plain border 5 mm inside the edge. New sheets and
  sheets adopted on open get the Default frame. The engine
  (`crates/engine/src/sheets/template.rs`) turns a template into lines and
  resolved texts for a sheet's paper; `crates/chrome/src/paper.rs` draws them.
  Only bundled templates exist: `basic-frames/default` r1, in
  `libraries/basic-frames/templates/`. Projects don't store template copies yet.
- The Browser's sheet menu has **Paper** and **Frame** submenus; there is no
  Page setup dialog.
- The paper and frame are drawn in the theme's schematic palette, and the canvas
  around the paper stays editable. **Appearance → Show grid outside paper**
  chooses whether the grid continues there.
- The schematic Y axis points down (`canvas_camera.rs`, `screen`). Coline's Y
  axis points up from the paper's bottom-left corner, so ported geometry must be
  flipped.
- Projects and sheets have parameters (PR 2), edited in Properties when
  nothing is selected or with the `parameter.*` commands. Title blocks are the
  first drawn text that resolves references. Components have
  named fields (`schematic/fields.rs`) and library parts have
  `fixed_parameters`, but those are unrelated.

## Decisions

| Topic | Decision |
| --- | --- |
| Template per size | One template fits every paper size. Template items are anchored to a paper corner. Coline uses one template per size; we don't. |
| Default paper | Every sheet has paper. New sheets start on A4 landscape with the Default frame. |
| No template | A page record without `template` has a plain border; the Default frame is always stored explicitly. |
| Template storage | Until template editing (PR 4), sheets reference bundled templates only and projects store no template copies. An unknown reference is refused on open. |
| Frame chooser | A **Frame** submenu (None, Default) beside **Paper** in the Browser's sheet menu. |
| Existing sheets | On open, a sheet without paper gets the Default frame and the smallest A size whose placed paper holds its drawing and margin, with the title block covering none of the drawing, falling back to A0 landscape. Paper is placed around the drawing, with its top-left corner snapped to the grid and clamped to the schematic coordinate range. Near those limits it may not be centered; a size is chosen only if the clamped paper still holds the drawing, so drawing extends outside the paper only with the A0 fallback. Nothing moves. Like the bundled-library migration, adoption happens in memory without an undo step; the next save writes paper to older sheet files. |
| Parameter syntax | Coline's `@scope.Name`, ported as a baseline. |
| Paper colour | Uniform by default: paper and surround use the same theme-following canvas colour, with no paper shadow. Appearance preferences offer Shaded paper or a Custom sheet colour, plus a canvas colour. Overrides are opaque and remembered separately for light and dark themes; Use theme clears an override. Symbols and wires retain the theme palette. Custom symbol colours and print styling remain deferred. |
| Canvas | The paper outline marks the default export area; it is not an editing boundary. Parts can be placed and moved outside it, for example while rearranging circuits. The canvas around the paper keeps the theme's colour wherever the drawing lies. |
| Grid | **Appearance → Show grid outside paper**, on by default, continues the schematic grid across the canvas. Turned off, the grid is drawn on the paper only. There is no mode that hides the paper or its outline. |

## Frame layers

| Layer | Owner | Content |
| --- | --- | --- |
| Paper | Sheet | Size, orientation and `at_nm`, the paper's top-left corner in sheet coordinates. |
| Border and zones | Template | Border inset and zone pitch, generated for the sheet's paper. Columns are numbered 1, 2, 3 from the left; rows are lettered A, B, C from the top. |
| Title block | Template | Lines, texts and later a logo, each anchored to a paper corner. Texts hold parameter references. It can't be selected or moved on the sheet. |

The Default template follows Coline: a border 5 mm inside the paper (a zone
band takes it to 10 mm), and a 120 × 28 mm title block in the bottom-right
corner of the inner border. The block has four 7 mm rows, and a split 70 mm from
its left edge. It shows Title, Project, Sheet, Company, Page, Drawn by, Rev and
Date. Each side of the band gets the nearest whole number of equal zones about
50 mm long, so A4 landscape has 6 columns and 4 rows. Rows skip I and O, as in
ISO 5457. Texts smaller than 3 points on screen are not drawn.

## Parameters

These are ported from Coline. Each value lives at project, sheet or instance
level and is referenced as `@scope.Name`. A reference always names its scope
and never falls back to another scope. A value may hold references, resolved a
few levels deep, so a loop ends as an unknown reference. An unknown reference
is drawn as typed, in orange. `@@` writes a literal `@`, and an `@` that isn't
followed by a scope and a dot stays plain text.

| Reference | Source | Default |
| --- | --- | --- |
| `@sheet.Title` | Sheet parameter | `@sheet.Name` |
| `@sheet.Revision`, `.Date`, `.Company`, `.DrawnBy` | Sheet parameters | `@project.Revision` and so on |
| `@project.Revision`, `.Date`, `.Company`, `.DrawnBy` | Project parameters | Empty |
| `@project.Name`, `@sheet.Name`, `@instance.Name` | Built in, read only | |
| `@instance.Page`, `@instance.Pages` | Built in: page number and count in Browser order | |

The second row is the one change from Coline: a sheet's values point at the
project by default, so you set them once for the project and each sheet can
override them. The `@part` and `@board` scopes come later.

Only values that differ from these defaults are stored, and `Name`, `Page` and
`Pages` can't be set. Editing matches names without case, so `revision` edits
`Revision`.

Commands, shared by the GUI and MCP, with one undo step per edit:
`parameter.set` (set, rename, remove), `parameter.list`, `parameter.resolve`
and `sheet.set_page`. In Properties, a standard value that is not set shows its
inherited value as a hint, leaving a field submits it, clearing a field removes
the value, and **New** adds an empty parameter by name.

## Storage sketch

The `page` record, including the `template` reference, and both `parameters`
objects are final as of sheet/9 and project/22. The template document schema is
final for bundled templates; project template copies are still a sketch.
Coordinates are in nanometres, like the rest of the format.

```jsonc
// sheets/<sheet-uuid>.json
"page": {
  "paper": "a3",
  "orientation": "landscape",
  "at_nm": [0, 0],
  "template": { "library": "basic-frames", "name": "default", "revision": 1 }
},
"parameters": {
  "Title": "Power supply",
  "Revision": "@project.Revision",
  "Date": "@project.Date",
  "Company": "@project.Company",
  "DrawnBy": "@project.DrawnBy"
}

// project.json
"parameters": { "Revision": "B", "Date": "2026-11-15", "Company": "", "DrawnBy": "" }

// libraries/basic-frames/templates/default.r1.json (bundled; abridged)
{
  "schema": "pcbcraft-sheet-template/1",
  "library": "basic-frames",
  "name": "default",
  "revision": 1,
  "title": "Default",
  "description": "Border with zone markers and a title block ...",
  "border": {"inset_nm": 5000000, "zones": {"band_nm": 5000000, "pitch_nm": 50000000, "size_nm": 2500000}},
  "items": [
    {"kind": "rect", "anchor": "bottom_right", "at_nm": [-130000000, -38000000], "size_nm": [120000000, 28000000]},
    {"kind": "line", "anchor": "bottom_right", "from_nm": [-130000000, -31000000], "to_nm": [-10000000, -31000000]},
    {"kind": "text", "anchor": "bottom_right", "at_nm": [-128000000, -34500000], "size_nm": 3500000,
     "align": "left", "style": "value", "text": "@sheet.Title"}
  ]
}
```

Item offsets use the sheet's axes from the anchor's paper corner, so bottom
anchors have negative Y. `size_nm` on a text is measured like component field
text; `style` is `label` or `value`; `align` is `left`, `center` or `right`, with
the anchor at mid height. Unknown fields are refused.

Templates are pinned by revision. Making them a third native library kind next to
symbols and footprints, with copies stored in the project under
`library/templates/<library>/<name>.r<revision>.json`, comes with template
editing (PR 4), when a template's content can first differ from the bundled one.
Schema bumps follow [native project storage](../formats/native-project-storage.md).

## Pull requests

1. **Paper on the sheet.** Sheet schema bump with `page`, the `sheet.set_page`
   command, and themed paper drawn with a plain border. The grid extends outside
   the paper by default; **Appearance → Show grid outside paper** can confine it
   to the paper. Paper is adopted for existing sheets on open. Fit includes paper
   and drawing, while Home thumbnails frame the paper.
2. **Parameters.** Port of Coline's parameter model and resolver for the
   project, sheet and instance scopes, with the `parameter.*` commands, the
   storage fields and a Parameters table in Properties.
3. **Templates (bundled).** Template documents with pinned revisions, the
   bundled `basic-frames` Default template ported from Coline's generator, zone
   markers, `sheet.set_template` and `template.list`, and a Frame submenu.
4. **Template editing and output.** Project template copies as a native library
   kind, editing that reuses the symbol editor's drawing tools, logo images, a
   Page setup dialog if needed, and the same frame in schematic PDF export
   ([#87](https://github.com/tangency-labs/pcbcraft/issues/87)).

Free text, lines and rectangles on sheets, the rest of #46, can be ported from
Coline's figures (`crates/format/src/figure.rs`, `crates/sheet/src/figure.rs`)
in a separate PR.

## Sources

Coline at `bdd89dfb371e622f9ba94f368190c8c086d170d6`, MIT OR Apache-2.0. The
paper-size adaptation is recorded in [ATTRIBUTION.md](../../ATTRIBUTION.md#sheet-paper).
Each later adaptation must record its source, revision, licence and changes there.

| Coline file | Use |
| --- | --- |
| `crates/format/src/sheet.rs` | `PaperFormat`, `SheetSize` (PR 1) |
| `crates/format/src/parameter.rs`, `crates/model/src/params.rs`, `crates/engine/src/commands/parameters.rs` | Parameters (PR 2) |
| `crates/sheet/src/template.rs`, `crates/format/src/library/template.rs`, `crates/engine/src/commands/templates.rs`, `docs/formats/template.md` | Default template and template model (PR 3) |

Port no Coline UI code; build the interface with `crates/chrome`. Don't port
Coline's storage format either, since PCBCraft stores JSON. No KiCad source is
involved.
