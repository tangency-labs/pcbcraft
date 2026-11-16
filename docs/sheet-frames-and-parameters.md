# Sheet frames, parameters and templates

**Status: proposal, partly implemented.** PR 1 (paper on the sheet) and PR 2
(parameters) are implemented; their storage contracts are in
[native project storage](native-project-storage.md#sheet-paper-records) and
[parameter records](native-project-storage.md#parameter-records). PRs 3–4 are
proposals. Design agreed November 15, 2026 for
[#46](https://github.com/tangency-labs/pcbcraft/issues/46). Interactive
mockup (private):
<https://claude.ai/code/artifact/7119e9c8-def3-4fb0-a498-3e2a431ef947>.

Schematic sheets get paper (A4 to A0, landscape or portrait), a border with
zone markers and a title block. The title block shows parameter values such as
`@sheet.Title`. Most of the model is ported from Coline.

## Current state

- Sheets have paper (PR 1): a size, orientation and top-left corner, stored as
  sheet/9 and drawn with a plain border 5 mm inside the edge. The schematic range
  stays ±1 m around the origin (`MAX_CENTER_NM` in
  `crates/engine/src/schematic.rs`), and paper must fit inside it.
- The Browser's sheet menu has a **Paper** submenu; there is no Page setup
  dialog or frame chooser yet.
- The paper is drawn in the theme's schematic palette, and the canvas around it
  stays editable. **Appearance → Show grid outside paper** chooses whether the
  grid continues there.
- The schematic Y axis points down (`canvas_camera.rs`, `screen`). Coline's Y
  axis points up from the paper's bottom-left corner, so ported geometry must be
  flipped.
- Projects and sheets have parameters (PR 2), edited in Properties when
  nothing is selected or with the `parameter.*` commands. No drawn text resolves
  references yet; title blocks (PR 3) are the first consumer. Components have
  named fields (`schematic/fields.rs`) and library parts have
  `fixed_parameters`, but those are unrelated.

## Decisions

| Topic | Decision |
| --- | --- |
| Template per size | One template fits every paper size. Template items are anchored to a paper corner. Coline uses one template per size; we don't. |
| Default paper | Every sheet has paper. New sheets start on A4 landscape with the Default frame. |
| Existing sheets | On open, a sheet without paper gets the smallest A size whose placed paper holds its drawing and margin, falling back to A0 landscape. Paper is placed around the drawing, with its top-left corner snapped to the grid and clamped to the schematic coordinate range. Near those limits it may not be centered; a size is chosen only if the clamped paper still holds the drawing, so drawing extends outside the paper only with the A0 fallback. Nothing moves. Like the bundled-library migration, adoption happens in memory without an undo step; the next save writes paper to older sheet files. |
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
corner. The block has four 7 mm rows, and a split 70 mm from its left edge. It
shows Title, Project, Sheet, Company, Page, Drawn by, Rev and Date.

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

The `page` record without `template` and both `parameters` objects are final as
of sheet/9 and project/22; the template parts are not.
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

// library/templates/basic-frames/default.r1.json
{
  "schema": "pcbcraft.sheet-template/1",
  "border": { "inset_nm": 5000000, "zones": { "pitch_nm": 50000000 } },
  "items": [
    { "kind": "rect", "anchor": "bottom_right",
      "at_nm": [-120000000, -28000000], "size_nm": [120000000, 28000000] },
    { "kind": "text", "anchor": "bottom_right",
      "at_nm": [-118000000, -24500000], "height_nm": 2500000, "text": "@sheet.Title" }
  ]
}
```

Templates are a third native library kind next to symbols and footprints, and
are pinned by revision. A bundled `basic-frames` library ships the Default
template. Schema bumps follow [native project storage](native-project-storage.md).

## Pull requests

1. **Paper on the sheet.** Sheet schema bump with `page`, the `sheet.set_page`
   command, and themed paper drawn with a plain border. The grid extends outside
   the paper by default; **Appearance → Show grid outside paper** can confine it
   to the paper. Paper is adopted for existing sheets on open. Fit includes paper
   and drawing, while Home thumbnails frame the paper.
2. **Parameters.** Port of Coline's parameter model and resolver for the
   project, sheet and instance scopes, with the `parameter.*` commands, the
   storage fields and a Parameters table in Properties.
3. **Templates as library items.** Template documents with pinned revisions,
   the bundled `basic-frames` library ported from Coline's Default generator,
   zone markers and a frame chooser in Page setup.
4. **Template editing and output.** Editing that reuses the symbol editor's
   drawing tools, logo images, and the same frame in schematic PDF export
   ([#87](https://github.com/tangency-labs/pcbcraft/issues/87)).

Free text, lines and rectangles on sheets, the rest of #46, can be ported from
Coline's figures (`crates/format/src/figure.rs`, `crates/sheet/src/figure.rs`)
in a separate PR.

## Sources

Coline at `bdd89dfb371e622f9ba94f368190c8c086d170d6`, MIT OR Apache-2.0. The
paper-size adaptation is recorded in [ATTRIBUTION.md](../ATTRIBUTION.md#sheet-paper).
Each later adaptation must record its source, revision, licence and changes there.

| Coline file | Use |
| --- | --- |
| `crates/format/src/sheet.rs` | `PaperFormat`, `SheetSize` (PR 1) |
| `crates/format/src/parameter.rs`, `crates/model/src/params.rs`, `crates/engine/src/commands/parameters.rs` | Parameters (PR 2) |
| `crates/sheet/src/template.rs`, `crates/format/src/library/template.rs`, `crates/engine/src/commands/templates.rs`, `docs/formats/template.md` | Default template and template model (PR 3) |

Port no Coline UI code; build the interface with `crates/chrome`. Don't port
Coline's storage format either, since PCBCraft stores JSON. No KiCad source is
involved.
