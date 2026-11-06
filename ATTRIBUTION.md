# Attribution

PCBCraft's own code is MIT OR Apache-2.0. This file lists what came from elsewhere. Add to it in the same PR whenever you bring something in: source, revision, license, and what changed.

## Adapted from SolidCraft

[SolidCraft](https://github.com/tangency-labs/solidcraft), revision `a43cd99e554de879e2abd548f67ca963919823b0`. Copyright (c) 2026 SolidCraft contributors. MIT OR Apache-2.0, the same texts as [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

| PCBCraft file | SolidCraft source | What changed |
|---|---|---|
| `crates/chrome/src/theme.rs` | `assets/styles/ui/palettes.rs`, `crates/ui-egui/src/theme.rs` | Kept the core palette values, token names and widget styling. Dropped theme files and color vision modes. Added canvas and error tokens. |
| `crates/chrome/src/fonts.rs` | `crates/ui-egui/src/fonts.rs` | Kept the Plex setup, at two weights. Dropped language fallbacks and overrides. |
| `crates/chrome/src/menu.rs` | `crates/ui-egui/src/ui_kit/menu.rs` | Kept the popup surface and row layout. Dropped icons, flyouts, checks and translation. |
| `crates/mcp/src/server.rs` | `crates/mcp/src/server.rs` | Kept the JSON-RPC framing and MCP lifecycle. Replaced the instructions and resources. Dropped file path scoping, since no tool touches files yet. |
| `crates/mcp/src/backend.rs`, `crates/mcp/src/tools.rs` | same paths | Kept the `Backend` seam and tool annotations. Wrote new tools over PCBCraft's commands. |
| `apps/pcbcraft-web/` | `apps/solidcraft-web/` | Kept the Trunk setup, page layout and web runner start. Dropped offline use, installing, files and URL launch options. |
| `.github/workflows/`, `deny.toml`, `rustfmt.toml`, `clippy.toml` | same paths | Cut down to what PCBCraft needs. `deny.toml` adds the license allow list. |

The local transport in `crates/craft-control/` follows the first-request token
and loopback design in SolidCraft's
[`control_server.rs`](https://github.com/tangency-labs/solidcraft/blob/ffc84dbe2e35af7d2eae302d42953271ef7a72b7/apps/solidcraft/src/control_server.rs)
and [control protocol](https://github.com/tangency-labs/solidcraft/blob/ffc84dbe2e35af7d2eae302d42953271ef7a72b7/docs/control-protocol.md)
at `ffc84dbe2e35af7d2eae302d42953271ef7a72b7`, copyright (c) 2026 SolidCraft
contributors, MIT OR Apache-2.0. The crate reuses PCBCraft's stdio framing
and leaves token distribution, discovery, and command dispatch to its callers.

The desktop live MCP setup follows the explicit per-window connection and
copyable configuration in PCBcraft Next's `docs/mcp.md` at the revision below.
PCBCraft's adapter dispatches directly through the shared editor session and
limits live edits to moving a symbol.

The integrated Linux title bar and ribbon dimensions follow SolidCraft's
`crates/ui-egui/src/titlebar.rs` and `crates/ui-egui/src/toolbar.rs` at
`796b7c40d380e22f00a0e45a0a48292b1756620d` (MIT OR Apache-2.0,
copyright (c) 2026 SolidCraft contributors). PCBCraft uses its own compact
window controls and schematic tool groups.
The unsaved-changes question in `crates/app/src/unsaved.rs` follows SolidCraft's
close prompt in `crates/ui-egui/src/documents.rs` and `apps/solidcraft/src/main.rs`
at `c553365a2197a689e749389a6fbf707a38df9b61` (MIT OR Apache-2.0, copyright (c)
2026 SolidCraft contributors): the "Save changes to {name}?" wording, the
Save/Don't Save/Cancel answers, and cancelling the window's close request until
the question is answered. PCBCraft asks about its one open project, waits for
a running file operation, and also asks before New Project and Open.
The filled Save and Report a bug glyphs use the shapes from
`assets/icons/ui/design/svg/flat/save.svg` and `flat/bug_report.svg` at the
same revision, drawn with the active theme colors.

The in-app Assistant follows SolidCraft's desktop chat workflow from
`docs/agent-chat.md` at `bd3053f57be8b862abccc4691156e1865138f490`
(MIT OR Apache-2.0). The header, agent switcher, connection actions,
dock drag preview, keyboard shortcuts and native pop-out adapt
`crates/ui-egui/src/agent_chat/panel.rs` at the same revision. The app bar
entry follows its launcher beside Preferences. The AI preferences page and
Add agent dialog adapt the availability, agents, panel, and startup sections
of `crates/ui-egui/src/agent_chat/settings.rs` at that revision. PCBcraft
profiles use its Codex and Claude Code CLI connections. The status pill, pinned composer and user bubble
adapt the presentation primitives from `crates/ui-egui/src/ui_kit/assistant.rs` to PCBCraft's theme.
Copyright (c) 2026 SolidCraft contributors. The bottom-right drawer and explicit Apply and Discard
follow PCBcraft Next's `docs/assistant.md` at the revision below. The launcher
draws a simplified, theme-colored version of SolidCraft's
`assets/icons/ui/design/svg/flat/assistant.svg` at
`c553365a2197a689e749389a6fbf707a38df9b61` (MIT OR Apache-2.0), as used in
the PCBcraft Next launcher. The Codex and Claude Code process adapters are
implemented in PCBCraft.

## Adapted from PCBcraft Next

PCBcraft Next, revision `568cd495397a91563e563e7163efb226d84fa83a`, MIT OR Apache-2.0. This is the independent prototype.

| PCBCraft file | PCBcraft Next source | What changed |
|---|---|---|
| `crates/engine/src/wire.rs`, `crates/engine/src/schematic/wiring.rs`, `crates/app/src/schematic_canvas/wires.rs`, `crates/app/src/lib.rs` | `docs/wire-editing.md`, `docs/desktop-guide.md` (behavior only) | Followed free-space route starts, orthogonal corners, elbow flipping, cancellation and placement/hover rotation. The explicit endpoint graph, format 3 codec and shared commands are new implementation; no prototype implementation was copied for this change. |
| `libraries/starter/symbols/led.json`, `crates/engine/src/schematic.rs` | `src/document.rs`, `src/definitions.rs`, `src/desktop/canvas.rs` | Adapted the original generic LED definition and drawing to the shared placement command. Preserved numbered terminals and identified their polarity in inspection. The drawing now lives in the starter library as nanometer strokes. |
| `libraries/starter/symbols/capacitor.json`, `libraries/starter/symbols/connector.json` | `src/document.rs`, `src/desktop/canvas.rs` | Added the original generic capacitor and two-pin connector with their names, reference prefixes and default values. Converted their point drawings to nanometer strokes in the starter library. |
| `crates/engine/src/lib.rs`, `crates/app/src/schematic_canvas.rs`, `crates/app/src/lib.rs` | `docs/implementation-status.md` (behavior only) | Followed the prototype's select, rotate, delete and reference/value editing behavior with new engine commands, a format 2 schema and canvas code. No prototype implementation was copied. |
| `crates/app/src/project_browser.rs`, `crates/chrome/src/workspace.rs`, `crates/app/src/lib.rs` | `docs/sheet-workflows.md`, `docs/desktop-guide.md`, `docs/ui-audit.md` (behavior only) | Followed the prototype's Browser tree (Project › Sheets › sheet › Symbols / Nets), with fold arrows separate from row clicks, sheet-local item lists that select without opening a drawer, and Capture/View ribbon tabs where tab changes never edit the project. The tree, tab strip, row glyphs and pick-to-center are new code; no prototype implementation was copied. Sheet creation and hierarchy are not ported yet. |
| `crates/app/src/parts_browser.rs`, `crates/chrome/src/workspace.rs` | `docs/implementation-status.md` (behavior only) | Followed the prototype's searchable parts browser with Small/Medium/Large/List views, click-to-place and drag-to-sheet placement. The card, thumbnails and drag handling are new code over the existing canvas painter; floating and snapping the panel are not ported. |
| `crates/engine/src/schematic/wiring.rs`, `crates/engine/src/lib.rs` | `src/document.rs`, `src/netlist.rs` | Adapted explicit terminal references and net traversal to the existing command session. This slice supports direct pin links on one schematic. |
| `crates/engine/src/schematic.rs`, `crates/engine/src/lib.rs`, `libraries/starter/symbols/resistor.json` | `src/document.rs`, `src/definitions.rs`, `src/session.rs` | Adapted nanometer coordinates, distinct component and symbol identities, and the procedural resistor definition to the existing session and command history. The resistor drawing now lives in the starter library. |
| `crates/app/src/schematic_canvas.rs` | `src/desktop/canvas.rs` | Adapted the schematic grid and resistor zigzag for the shared desktop and browser canvas. Pin endpoints come from the engine and drawing metrics live in `crates/chrome`. |
| `crates/app/src/schematic_canvas/camera.rs`, `crates/app/src/schematic_canvas.rs` | `src/desktop/canvas.rs`, `src/desktop/page.rs` | Adapted the viewport transform, pointer-anchored zoom, middle-button pan, and fit behavior to Tangency’s canvas and chrome. |
| `crates/app/src/schematic_canvas/wires.rs`, `crates/app/src/lib.rs` | `src/desktop/canvas.rs`, `src/desktop/mod.rs`, `src/desktop/wire_geometry.rs` | Adapted pin hit testing, the Wire tool shortcut, the cancelable preview, and a symmetric subset of the orthogonal display routes. Completed routes now dispatch through the shared `schematic.draw_wire` command. |
| `crates/app/src/net_inspector.rs`, `crates/app/src/lib.rs` | `src/desktop/nets.rs` | Adapted the revision-aware connectivity view to Tangency’s existing inspection output. The docked inspector shows connected groups and unconnected pins by symbol reference. |
| `crates/chrome/src/workspace.rs`, `crates/app/src/lib.rs` | `docs/specs/workspace-ui.md`, `docs/screenshots/schematic.png`, `assets/original/place-symbol.svg`, `assets/original/wire.svg` | Followed the schematic workspace layout and adapted the original Place Symbol and Wire paths for theme-colored egui drawing. |
| `crates/engine/src/persistence.rs`, `crates/app/src/project_files/` | `docs/specs/persistence.md`, `src/persistence.rs`, `src/desktop/mod.rs` | Adapted the bounded snapshot and file worker patterns. The Tangency codec uses a new schema and validates its own symbol, wire, and footprint references. |

The workspace ribbon and inspector card also reference SolidCraft's
`crates/ui-egui/src/toolbar.rs` and `crates/ui-egui/src/ui_kit/panel.rs` at
`c553365a2197a689e749389a6fbf707a38df9b61`, copyright (c) 2026 SolidCraft
contributors, MIT OR Apache-2.0. PCBCraft implements these controls in its own
chrome crate with the existing family theme tokens. The Schematic tile uses
the drawing glyph from `assets/icons/ui/design/svg/workspace/ws_drawing.svg`
at that same revision, rendered with theme colors in `crates/chrome/src/workspace.rs`.

The application bar follows the brand, quick actions and document title in
PCBcraft Next's `src/desktop/mod.rs` at the revision above. The mark in
`crates/chrome/src/shell.rs` adapts its original `assets/original/pcbcraft.svg`.
The right-side utilities follow SolidCraft's `toolbar.rs::app_bar`. The File
popup follows PCBcraft Next's `src/desktop/controls.rs` and `src/desktop/mod.rs`.
The glyphs in `crates/chrome/src/menu/icons.rs` are simplified,
theme-colored drawings based on SolidCraft's `assets/icons/ui/design/svg/`
masters `model/new.svg`, `model/open.svg`, `flat/save.svg`,
`flat/export_tray.svg`, `model/rename.svg`, and `flat/close.svg` at
`c553365a2197a689e749389a6fbf707a38df9b61`, copyright (c) 2026 SolidCraft
contributors, MIT OR Apache-2.0. The download arrow points into the tray.
The history arrows follow the family `flat/undo.svg` and `flat/redo.svg` at
the same revision.

The Home page follows SolidCraft's start page in `crates/ui-egui/src/home.rs`
and its surfaces in `crates/ui-egui/src/ui_kit/file_view.rs` at
`c553365a2197a689e749389a6fbf707a38df9b61`, copyright (c) 2026 SolidCraft
contributors, MIT OR Apache-2.0. `crates/chrome/src/home.rs` keeps the
navigation rail, back row, navigation rows, separators, tile grid and action
tile layout, and the `file_*` sizes come from `assets/styles/ui/palettes.rs`
at that revision. The `file_*` colours are derived from PCBCraft's palette the
way SolidCraft's `crates/ui-egui/src/theme_files.rs` fills them in for theme
files that leave them out, with a deeper hover on the light rail. PCBCraft omits
samples, recovery, export and thumbnails, stacks the rail above the
content at narrow widths, and wires the rows to its own project dialogs and
commands. The house glyph in `crates/chrome/src/menu/icons.rs` is a simplified,
theme-colored drawing of `assets/icons/ui/design/svg/flat/home.svg` at the same
revision.

The Help menu under the ? button (`crates/chrome/src/menu.rs::help`,
`crates/app/src/lib.rs::help_menu`) follows the rows and order of SolidCraft's
`crates/ui-egui/src/help.rs` at `c553365a2197a689e749389a6fbf707a38df9b61`
(MIT OR Apache-2.0, copyright (c) 2026 SolidCraft contributors), keeping the
rows PCBCraft has: About, Keyboard Shortcuts, Report an Issue and Connect AI
assistants, which opens PCBCraft's own local connection dialog.

The Preferences window in `crates/chrome/src/settings.rs` follows SolidCraft's
paged window and page sidebar in `crates/ui-egui/src/ui_kit/panel.rs`, its
footer order in `ui_kit/actions.rs`, its settings heading, section and row
layout in `ui_kit/preferences.rs` and `ui_kit/layout.rs`, and its search in
`crates/ui-egui/src/prefs_search.rs` at
`c553365a2197a689e749389a6fbf707a38df9b61`, copyright (c) 2026 SolidCraft
contributors, MIT OR Apache-2.0. Kept: the sidebar and body geometry, OK and
Cancel with Apply between them, Restore page defaults, Cancel restoring the
settings from when the window opened, and search matching a row by its label,
section, page or keywords at a prefix, a word start or a run of three letters,
ignoring accents. The `form_*` and `dialog_*` sizes come from
`assets/styles/ui/palettes.rs`, and their colours are derived from PCBCraft's
palette. PCBCraft's pages, settings, page glyphs and search glyph are its own,
and its search counts matches without drawing them, then draws only the rows
that match.

The sheet-path tabs in `crates/app/src/editor_view.rs` and
`crates/chrome/src/editor.rs` follow the breadcrumb and occurrence-path behavior
described in PCBcraft Next's `docs/sheet-workflows.md` at
`568cd495397a91563e563e7163efb226d84fa83a` (MIT OR Apache-2.0). Next lists sheet
tabs as unfinished. These tabs are new upstream code over its current single-sheet
model; no prototype tab or hierarchy implementation was copied.
The single-view layout in `crates/app/src/editor_workspace.rs` restores the existing
`crates/chrome/src/workspace.rs::sheet_heading` canvas overlay, following Next's
`src/desktop/canvas.rs` and `src/desktop/panels.rs` at the same revision. Its path
and count share the canvas with the floating Browser; tab strips and their panel
clearance are reserved for split views. Heading insets remain in chrome.

The recent-project list in `crates/chrome/src/home.rs` and its `file_recent_height`
metric in `crates/chrome/src/theme.rs` adapt `ui_kit/file_view.rs::recent_row` at
SolidCraft revision `c553365a2197a689e749389a6fbf707a38df9b61` (MIT OR Apache-2.0,
copyright (c) 2026 SolidCraft contributors): shared list surface, 52-point rows,
separators, title/path presentation, hover/focus and path tooltips. PCBCraft uses
its existing folder glyph, a two-line layout at every width, and a separate remove
button, without thumbnails or optional metadata. The 12-entry history bound and
promote/forget behavior follow `home.rs::HomeState` at that same revision.
`crates/app/src/recent_projects.rs`, `home.rs` and `project_files.rs` implement new
application-storage history and reopen through upstream's revision-checked file
worker and unsaved-changes controller; no SolidCraft session or file model is reused.

## Bundled fonts

| File | Source | License |
|---|---|---|
| `crates/chrome/assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf` | IBM Plex Sans, IBM Corp., unmodified, by way of SolidCraft at the revision above | SIL OFL 1.1, text alongside |
| `crates/chrome/assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf` | IBM Plex Mono, IBM Corp., unmodified, by way of SolidCraft at the revision above | SIL OFL 1.1, text alongside |

## Dependencies

`Cargo.lock` lists them, and `deny.toml` is the list of licenses allowed. `epaint_default_fonts`, which egui bundles as its fallback, is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`. A notices file for release binaries is still to do, and is needed before the first release.

The workspace picker on the ribbon's workspace tile (`crates/chrome/src/workspace.rs`,
`picker` and `workspace_row`) follows SolidCraft's `workspace_menu` in
`crates/ui-egui/src/ui_kit/menu.rs` at `c553365a2197a689e749389a6fbf707a38df9b61`
(MIT OR Apache-2.0, copyright (c) 2026 SolidCraft contributors): its width, row
heights, accent bar, icon tile, badges and footer. The tile's caret, the entries'
wording and the keyboard handling follow PCBcraft Next's `src/desktop/workspace.rs`
at `568cd495397a91563e563e7163efb226d84fa83a` (MIT OR Apache-2.0), itself adapted
from the same SolidCraft revision. The workspace icons are drawn with the theme's
icon colour roles (`icon_ink`, `icon_face_*`, `icon_active_*` in
`crates/chrome/src/theme.rs`), whose light and dark values come from SolidCraft's
icon palette as carried in PCBcraft Next's `assets/family/icon-palette.toml`. The
schematic sheet and render sphere follow SolidCraft's `workspace-schematic.svg`
and `workspace-render.svg` from the same file set; the PCB layout and simulation
glyphs follow PCBcraft Next's original `assets/original/workspace-layout.svg` and
`simulate.svg`. The Current badge uses the Home rail's accent pair, which keeps
its small text readable.

The Browser rows (`crates/chrome/src/workspace.rs`, `tree_row`) adapt the row
layout of SolidCraft's `crates/ui-egui/src/ui_kit/tree.rs` at
`c553365a2197a689e749389a6fbf707a38df9b61` (MIT OR Apache-2.0, copyright (c) 2026
SolidCraft contributors): a stable ID per row, a disclosure arrow that folds
without clicking the row, and the selected fill with a semibold, ellipsized
label. Eyes, radios, badges, actions, drag handling and translation are left out,
and the arrow is its own accessible button.

The ribbon groups (`crates/chrome/src/workspace.rs`, `ribbon_group`, `tool_button`
and `ribbon_separator`) follow PCBcraft Next's `tool_group`, `tool_button` and
`group_separator` in `src/desktop/ribbon.rs` and its `RibbonMetrics` in
`src/desktop/theme.rs` at `568cd495397a91563e563e7163efb226d84fa83a`: 56 by 52
point buttons 2 points apart, the glyph and label placement, group padding, the
separator slot, and a caption with a caret 10 points under the buttons that opens
a menu of the group's actions, and the 5 point gutter under the groups. The
menu's check marks for toggles
(`crates/chrome/src/menu.rs`) follow Next's `src/desktop/controls.rs` `MenuItem`.
The code is new; Next's planned, disabled tools are not shown.

The floating Browser card, its collapsing header, the row metrics (26 point rows,
16 point indent, 18 point arrow target, 15 point glyph, label 25 points in), the
"R1  1k" item labels and the breadcrumb with its sheet count drawn on the canvas
follow PCBcraft Next's `src/desktop/panels.rs`, `src/desktop/controls.rs` and
`src/desktop/canvas.rs` at the revision above; the code is new. The ribbon and
Browser glyphs are drawn in two colour roles, with the new `icon_line` token
taking its light and dark values from PCBcraft Next's
`assets/family/icon-palette.toml`. Symbol, Wire and the sheet glyph redraw PCBcraft
Next's original `assets/original/place-symbol.svg`, `wire.svg` and `sheet.svg`.
The folder and Fit glyphs redraw SolidCraft's
`assets/icons/ui/design/svg/flat/folder.svg` and `fit.svg` at
`60488a0837affcee79832bdb7bf6e860e84b10f2` (MIT OR Apache-2.0, copyright (c) 2026
SolidCraft contributors), as carried in PCBcraft Next's `assets/family/icons/`.

The floating Parts library in `crates/app/src/parts_browser.rs` and
`crates/chrome/src/workspace/catalog.rs` adapts PCBcraft Next's
`src/desktop/library.rs`, `panels.rs`, `controls.rs` and `theme.rs` at
`568cd495397a91563e563e7163efb226d84fa83a` (MIT OR Apache-2.0). It keeps the
260-point card, drag grip, edge snapping, position menu, compact list, view
controls and preview/name/value tile layout. The catalog preview extent and
stroke weight in `crates/app/src/schematic_canvas.rs` and chrome theme metrics
follow `src/desktop/canvas.rs` at the same revision, retaining upstream symbol
definitions and their shorter leads. Familiar starter parts lead in the
prototype’s order. Presentation lives in chrome;
search and revision-checked placement use the upstream library and commands.
Clicking a part retains upstream's placement command and leaves the card open;
the header's Close button and View toggle hide it without losing its state. Camera clearance
uses the actual visible cards, including a library moved into the canvas.
The Parts collapse control uses the chevron and accessible naming of the
collapsing card header in Next's `src/desktop/panels.rs` at the same revision.
Applying that control to the Parts card, preserving its search/view/position,
and expanding it on **A** are upstream additions.

## Box selection and group editing

The drag-selection behavior in `crates/app/src/schematic_canvas/selection.rs`,
its integration in `crates/app/src/lib.rs` and `project_browser.rs`, and the
marquee painter in `crates/chrome/src/selection.rs` adapt PCBcraft Next's
`src/desktop/interaction.rs`, `canvas.rs` and `docs/selection-and-text.md` at
`568cd495397a91563e563e7163efb226d84fa83a` (MIT OR Apache-2.0). Both mouse buttons,
window/crossing direction, modifier selection, solid/dashed outlines and hit counts
follow that reference. Hit testing uses upstream symbol bodies and actual wire
paths. `crates/engine/src/selection.rs` implements new atomic group commands over
upstream snapshots; it does not reuse Next's session or document model.

## PCB inspection workspace visual reference

Travis Lowe's FigCraft exports, supplied as visual references on November 6,
2026, guide the Browser-above-Layers arrangement, grouped layer list, compact
right-side Properties and copper palette in `crates/app/src/board_panels.rs`
and `crates/chrome/src/board.rs`. The locally supplied artifacts are
`designs/exports/workspaces-2026-10-30/03-layout-top.png` (SHA-256
`e86739bdbffbbff0ea5bd47995642fb4febc750e84fe7841615968f8221663a7`) and
`04-layout-bottom.png` (SHA-256
`9353adae0c1da3392752818e751e44e13c69258b9de8579fd29f35a260ec3e6b`).
These exports are not bundled assets. No depicted circuit geometry, library data
or external implementation was copied; the example is generated from original
starter definitions through shared engine commands. The current slice omits the
reference's routing tools, selection filters, Flip view and fabrication geometry.
It extends the existing chrome card primitives; their provenance is recorded above.

## Eframe native window lifecycle patch

`vendor/eframe/` contains the crates.io eframe 0.36.2 package from egui revision
`49682f8baa058bf49e011035cfbd6e825f88a5ef`, copyright (c) 2018-2021 Emil Ernerfeldt,
MIT OR Apache-2.0. Both license texts are retained alongside it. Local changes add
an opt-in WGPU root-window retirement request so a surviving editor keeps its
native window when the original closes. The patch boundary and upgrade procedure
are recorded in [vendor/eframe/PATCH.md](vendor/eframe/PATCH.md). Document/session
ownership and the close/save policy remain in PCBcraft.
