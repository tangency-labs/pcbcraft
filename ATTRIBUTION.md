# Attribution

PCBCraft's own code is MIT OR Apache-2.0. This file lists what came from elsewhere. Add to it in the same PR whenever you bring something in: source, revision, license, and what changed.

## Selection filters

The right-hand Selection filters card follows Travis Lowe's original FigCraft
exports `01-schematic-overview.png` (SHA-256
`ae02dbd7b0d9f60c535a4755f7930ef318b4deead1e6c30f463a86dee18ec4ba`)
and `03-layout-top.png` (SHA-256
`e86739bdbffbbff0ea5bd47995642fb4febc750e84fe7841615968f8221663a7`)
in `designs/exports/workspaces-2026-10-30/`, reviewed November 7, 2026.
The implementation is original MIT OR Apache-2.0 work using existing Craft
cards, theme roles and egui input/accessibility primitives. No design files or external assets are
imported. Unsupported selection categories remain disabled; the live controls
and local arrangement state are independent of electrical documents.

The contextual-card docking primitive is original MIT OR Apache-2.0 work,
following the existing Parts library's qualified drag-to-float, edge snapping
and position-menu interaction. It reuses the Craft frame, palette, typography
and popup controls. Panel positions are presentation data; no project format,
external dependency or artwork is added.

## Board Edge drawings

The layer-addressed drawing model, commands, native board version 9 codec, canvas
tools and ribbon glyphs are original PCBcraft work under MIT OR Apache-2.0.
The earlier local rectangle codec remains for read compatibility. Layer-based
board-boundary terminology was checked against the published [KiCad 10 PCB Editor
manual](https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html#board_outlines_edge_cuts),
consulted November 7, 2026. No KiCad implementation, fixtures, icons or artwork
were used.

PCBcraft Next's original workspace documentation and the existing FigCraft exports
`03-layout-top.png` and `pcb-workspace.png` were reviewed as visual references
for grouped Layers and canvas drawing. No board geometry, project fixtures or
implementation code was imported from those references. The hosted Next demo
could not be opened in this environment; that review is limited to the local
documentation and exported visuals. This slice implements straight paths and
rectangles without closure, edge-clearance or manufacturing validation.

## Copper layer membership and personal colors

The engine's resolved pad-layer contract, local board/layer color settings and
GUI/live-control integration are original PCBcraft work under MIT OR Apache-2.0.
The form uses the existing Craft modal, typography, fields and action buttons,
with egui 0.36.2's color picker through its public API. No new dependency or
external artwork is imported. Front/back colors retain the shared board palette;
the alternate internal-layer green is an original palette extension. Color
choices are presentation settings and make no material/manufacturing claim.

## Layer controls

The active-layer selector, full-row activation, active arrow and independent
visibility behavior were checked against the published [KiCad 10 PCB Editor
manual](https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html#layer_controls) and
[Altium Designer's layer-view documentation](https://www.altium.com/documentation/altium-designer/pcb/your-view-of-the-board),
consulted November 7, 2026. These are behavioral references only; no implementation,
icons or artwork were copied. The controls reuse PCBcraft's existing themed
choice field, Browser tree rows, cards, typography and palette.

The layer-key patch in the ribbon and its popup uses the existing themed combo
control with reserved label space and shared palette keys. Technical-layer
color overrides extend the same local board/layer settings as copper. The keyboard
bindings were checked only against the published [KiCad 10 PCB Editor manual](https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html#actions_reference),
consulted November 7, 2026: +/− for next/previous and PgUp/PgDn for front/back copper.
Picker-order traversal and shared view dispatch are original PCBcraft work. An
incidental excluded search result and the user's subsequent manual-only approval
are recorded in [the source-boundary note](docs/source-boundaries/2026-11-07-layer-shortcut-search.md);
that result was not used as an implementation reference.

The per-layer eye at the right follows Travis Lowe's original FigCraft rendering
in `designs/pcb-workspace.png` and the eye construction in
`designs/grouped_pcb_layers.py` (SHA-256
`d213a6c0eb8ef4995225715c5af3f458339f4696bd7e04a9498751c716e7bfe2`),
reviewed November 7, 2026. The outline stays visible, a pupil indicates shown and
a slash indicates hidden. Its original vector geometry is adapted to the app's
existing theme roles and metrics; the design files remain unchanged.

## Named nets and power markers

The local/global/power/ground model, engine commands, sheet/3 records and chrome
vector drawings are original PCBcraft work. Scope, value-based supply names and
L/Ctrl+L/P controls were checked against the published
[KiCad 10 Schematic Editor user manual](https://docs.kicad.org/10.0/en/eeschema/eeschema.html)
(Labels, Power symbols and Actions reference, consulted November 7, 2026).
No KiCad implementation, symbol library data, icons or artwork was read or copied.
This slice adds no third-party dependencies or bundled assets.

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

`crates/craft-agents/` recreates SolidCraft's agent chat core from
`crates/ui-egui/src/agent_chat/` at `c553365a2197a689e749389a6fbf707a38df9b61`
(MIT OR Apache-2.0, copyright (c) 2026 SolidCraft contributors) until SolidCraft
extracts it as a shared library:

| PCBCraft file | SolidCraft source | What changed |
|---|---|---|
| `acp.rs`, `acp_client.rs`, `api_client.rs`, `backend.rs`, `discover.rs`, `markdown.rs`, `process.rs` | same names in `agent_chat/` | Client, bridge and tool names, token variable and system note are PCBCraft's. The MCP bridge runs `pcbcraft-cli mcp --connect PORT --token-file FILE --wait SECONDS`, with the live endpoint's credential in a private file as SolidCraft's token file keeps it, not in its environment. |
| `agents.rs` | `agent_chat/agents.rs` | Agent settings are an `AgentPrefs` value instead of SolidCraft's window state and preferences; `short_command` moved here from `panel.rs`. |
| `i18n.rs`, `lib.rs` | `i18n.rs`; `agent_chat/mod.rs` | English messages for the keys these modules use; the model id limits and a small `ControlRequest`. |

The Assistant in `crates/app/src/assistant.rs` and `assistant/` is a lighter rewrite of
SolidCraft's chat panel, state and gate at the same revision: `chat.rs` follows
`agent_chat/mod.rs` and `gate.rs` (conversation items, permission questions and holding tool calls
for Allow, Always allow or Deny under the Ask, Edits and All modes), and `settings.rs` follows
`agent_chat/settings.rs` (agents found, added and not installed, Test, default and per-agent
modes). The gate's rules are PCBCraft's own command namespaces, and the UI uses PCBCraft's chrome.
`assistant/host.rs` adapts SolidCraft's `agent_http` and `Keychain` from `apps/solidcraft/src/main.rs`
at the same revision: HTTPS through `ureq` and `rustls` with the platform's certificates, no
redirects, a bounded response, and keys in the system keychain through `keyring`.
The chat's token file (`write_token_file` and `restrict_to_user` in `assistant/chat.rs`) adapts
`write_token_file_with` and `restrict_to_user` from SolidCraft's `crates/engine/src/control_auth.rs`
at the same revision: an owner-only file on Unix, and on Windows `icacls` replacing the empty file's
inherited permissions before the secret goes in. PCBCraft calls `icacls` by its full path, keeps the
file in its own temporary folder, and writes no token when `icacls` fails or times out.

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
the window's own view and window handlers.


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
| `crates/app/src/project_browser.rs`, `crates/chrome/src/workspace.rs`, `crates/app/src/lib.rs` | `docs/sheet-workflows.md`, `docs/desktop-guide.md`, `docs/ui-audit.md` (behavior only) | Followed the prototype's Browser tree (Project › Sheets › sheet › Symbols / Nets), with fold arrows separate from row clicks, sheet-local item lists that select without opening a drawer, and Capture/View ribbon tabs where tab changes never edit the project. The tree, tab strip, row glyphs and pick-to-center are new code; no prototype implementation was copied. Sheet creation and hierarchy now have independent upstream implementations. |
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
samples, recovery and export, stacks the rail above the
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

The recent-project cards in `crates/chrome/src/home.rs` and their preview
and inset tokens in `crates/chrome/src/theme.rs` adapt SolidCraft's
`crates/ui-egui/src/ui_kit/file_view.rs::{sample_grid, sample_card_with_height, thumbnail}`
at revision `c553365a2197a689e749389a6fbf707a38df9b61` (MIT OR Apache-2.0,
copyright (c) 2026 SolidCraft contributors): a responsive card grid, image
region, inset, title/description placement and hover/focus treatment. PCBCraft replaces
the original 84-point preview height with a bounded aspect ratio based on the
card's actual width, caps the card width, and adds
a separate remove control, path-qualified accessibility labels and a clipped
app-supplied drawing callback. Middle elision of long paths adapts
`crates/ui-egui/src/ui_kit/text.rs::ellipsize_middle` at that revision.
The 12-entry bound and promote/forget behavior follow `home.rs::HomeState`.
`crates/chrome/src/preview.rs` adapts the bounded scheduling, retained worker slots
and stale-result rejection pattern in `crates/ui-egui/src/render_workspace_thumbnails.rs::Cache`:
it accepts generic keys and callbacks, permits two workers, and retains no
SolidCraft document, scene, cancellation or renderer types. The cache and card
interfaces have no PCBcraft engine dependency and can move to shared Craft UI.

`crates/app/src/recent_projects.rs`, `home.rs` and `project_files.rs` implement
application-storage history and reopen through upstream's file worker and
unsaved-changes controller. `schematic_canvas/thumbnail.rs` prepares saved geometry
and reuses upstream's symbol painter and engine wire paths, rendered by egui;
no new rasterizer, SolidCraft mechanical session or project model is included.
`crates/storage` adds bounded, read-only preview reads that neither recover
interrupted saves nor create locks or write project files. Preview pictures and
cache state remain outside native project documents.

## Native library source loading

The native source snapshot/catalog in `crates/engine/src/library/sources.rs`,
folder adapter in `crates/storage/src/library.rs`, source management UI and
bundle packager are original PCBcraft code under MIT OR Apache-2.0. They reuse
the existing upstream symbol/footprint validators and command path, without
external EDA implementation or prototype source. The Workshop example in
`examples/native-library` adapts this repository's original starter resistor
symbol and 0603 footprint from `57ee1e76f047f542c52d16bf3c0db7c119c47a8a`
retaining MIT OR Apache-2.0 and
the footprint's existing dimensional provenance. Its library identity and
display names are changed to demonstrate native source loading. No third-party
asset or new dependency is introduced.

## Bundled fonts

| File | Source | License |
|---|---|---|
| `crates/chrome/assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf` | IBM Plex Sans, IBM Corp., unmodified, by way of SolidCraft at the revision above | SIL OFL 1.1, text alongside |
| `crates/chrome/assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf` | IBM Plex Mono, IBM Corp., unmodified, by way of SolidCraft at the revision above | SIL OFL 1.1, text alongside |

## Dependencies

`Cargo.lock` lists them, and `deny.toml` is the list of licenses allowed. `epaint_default_fonts`, which egui bundles as its fallback, is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`. A notices file for release binaries is still to do, and is needed before the first release. `webpki-root-certs`, the Mozilla root-certificate list that the desktop Assistant's HTTPS uses, is CDLA-Permissive-2.0 data. It allows use, changes and redistribution, places no conditions on PCBCraft's own code, and asks only that its license text ship with the data, so that notices file must include it.

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
The original group renderer remains upstream code; the qualified toolbar port below now includes Next's planned tools as disabled controls.

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

## Existing-footprint transform slice

Movement and quarter-turn orientation in `crates/engine/src/board/transform.rs`,
`board.rs`, `board/view.rs` and the board canvas are original upstream code over
the existing session, view identity, camera and persistence contracts. Control
behavior was checked against the published [KiCad 9 PCB Editor manual](https://docs.kicad.org/9.0/en/pcbnew/pcbnew.html)
(Move and Rotate operations); no KiCad implementation source or library assets
were used. The Move and Rotate ribbon glyphs are original vector drawings using
the existing shared chrome palette. Legacy pad-center markers remain inspection
proxies; the later SMD geometry and its dimensional references are recorded below.

## Eframe native window lifecycle patch

`vendor/eframe/` contains the crates.io eframe 0.36.2 package from egui revision
`49682f8baa058bf49e011035cfbd6e825f88a5ef`, copyright (c) 2018-2021 Emil Ernerfeldt,
MIT OR Apache-2.0. Both license texts are retained alongside it. Local changes add
an opt-in WGPU root-window retirement request so a surviving editor keeps its
native window when the original closes. The patch boundary and upgrade procedure
are recorded in [vendor/eframe/PATCH.md](vendor/eframe/PATCH.md). Document/session
ownership and the close/save policy remain in PCBcraft.

## Wire endpoint attachment

Endpoint drops in `crates/app/src/schematic_canvas/wires.rs` and
`crates/app/src/schematic_canvas.rs` follow the interaction described in PCBcraft
Next's `docs/wire-editing.md` and implemented by the junction-drop path and
`attachment_target` in `src/desktop/canvas.rs` at
`568cd495397a91563e563e7163efb226d84fa83a` (MIT OR Apache-2.0). The new upstream
implementation reuses its drawing tool's pin/junction hit test, rejects ambiguous
targets and previews the connection before release. `crates/engine/src/wire/heal.rs`
and `wire.rs` adapt the bounded split/merge, positive-overlap detection, path
simplification and degree-two collapse workflow from Next's `src/wiring.rs`
(`resolve`, `heal_overlaps`, `cut_pair`, `merge_endpoints`, `deduplicate_paths`,
`collapse_runs`) at the same revision. Upstream retains its own snapshots, command
catalog, numeric identity allocation, format limits and revision checks; straight
junction collapse preserves intentional corners, and automatic routes are only
materialized when normalization changes them. Next's document/session model and
net-name machinery are not imported.

Pin contact on part/group moves, two-pin insertion into straight wires, the explicit
Heal command and the Capture → Wire edits group are new upstream implementations.
Shortening U-shaped returns by sliding a shared junction along its existing straight
run in `crates/engine/src/wire/heal.rs` is also original upstream logic; it preserves
endpoint identities and does not connect unrelated crossings.
Self-intersection erasure and removal of branch-free alternative routes in
`crates/engine/src/wire/heal/loops.rs`, plus the remembered Auto-heal preference,
are original upstream implementations as well.
The active drawing preview now shares the same self-route normalization, with
reversible corner checkpoints; this is new upstream code over those helpers.
Folded-transit tail cleanup in `crates/engine/src/wire/heal/tails.rs` is also new
upstream logic. It distinguishes obsolete split points from original free ends,
pins and branches, retaining that protection through endpoint merges.
Active-route preference during loop cleanup, direct node starts and the larger
magnetic hit targets are new upstream behavior. The plain crosshair and target ring
in `crates/chrome/src/cursor.rs` are original vector drawings using the shared
theme; no KiCad cursor artwork or implementation was used.
The per-view connection-target cache and edited-wire overlap scan are new upstream
optimizations. Target identities remain revision-bound, projected geometry is rebuilt
after navigation or viewport changes, and automatic starts exclude occupied nodes;
W explicitly begins at the pointer without changing the model until completion.
The Slice, Break, Heal and Delete glyphs in `crates/chrome/src/workspace.rs` are
original drawings using the existing shared icon palette and ribbon metrics.
The horizontal ribbon scrollbar uses Next's five-point `scroll_gutter` from
`src/desktop/theme.rs` at the same revision; upstream places it below the captions
so the additional wire actions remain usable at compact window widths.

The schematic **M** shortcut in `crates/app/src/lib.rs` dispatches the existing
Disconnect and Move preview and engine command. Its selected-symbol priority and
leave-wiring-in-place behavior follow the published [KiCad 9 Schematic Editor manual](https://docs.kicad.org/9.0/en/eeschema/eeschema.html#moving-symbols).
The shortcut routing and tests are original upstream code; no KiCad
implementation source or artwork was used.

## Original starter SMD footprints and NPN symbol

`libraries/starter/footprints/*.json`, `libraries/starter/symbols/npn.json`,
their engine definition/validation/persistence code and physical pad rendering
are original upstream work under MIT OR Apache-2.0. The regenerated Dual LED
example pins these definitions through shared engine commands.

Dimensional references are Vishay's [D/CRCW e3](https://www.vishay.com/docs/20035/dcrcwe3.pdf)
(14-Apr-2026, p. 11, 0402/0603 reflow lands),
[Kyocera's MLCC mounting guide](https://ele.kyocera.com/en/product/capacitor/ceramic/mlcc-handling/mounting-01/)
(original capacitor lands selected within its nominal ranges, consulted 7-Nov-2026),
[Kingbright APG1005CGC-T](https://www.kingbrightusa.com/images/catalog/spec/apg1005cgc-t.pdf)
(DSAM9856 V11B, 12-Mar-2025, p. 1, 0402 LED lands and polarity), and
[Nexperia SOT23](https://assets.nexperia.com/documents/outline-drawing/SOT23.pdf)
(12-Oct-2022, body and same-side pin pitch). SOT-23 copper lands, courtyards and
silkscreen are original demo choices, not vendor-qualified manufacturing data.
NPN terminal naming follows [Nexperia PMBT3904](https://assets.nexperia.com/documents/data-sheet/PMBT3904.pdf)
(16-Feb-2024); the symbol's vector drawing is original. The definition metadata
and [footprint contract](docs/footprint-library.md) record exact dimensions.
No manufacturer artwork, PDF, third-party footprint asset or GPL implementation
source was incorporated. The manufacturers' documents retain their own rights;
only dimensional facts informed these original definitions.

The follow-up rendering revision uses the user's supplied PCB screenshots as a
visual reference for copper contrast, rounded lands and pad labels. It imports
no screenshot assets or external implementation. Starter SMD revision 2 adds
original rounded corners (25% of the shorter land dimension) and removes the
misleading closed silkscreen strips between passive pads. The original
`header-1x02-254` definition uses chosen demo dimensions: 2.54 mm pitch, 1.70 mm
lands and 1.00 mm plated drills. Its drawing and geometry are MIT OR Apache-2.0,
without a third-party library asset or a vendor qualification claim.

The initial physical silkscreen slice adds original `pcbcraft-stroke-1` monoline
lettering in `crates/engine/src/board/silkscreen.rs`, original centerline outlines
and polarity marks, and original conservative pad-clearance clipping. No font,
KiCad drawing data or external implementation was copied. The starting print
style follows dimensional guidance in
[JLCPCB ordering instructions](https://jlcpcb.com/help/article/instructions-for-ordering)
and [character design specifications](https://jlcpcb.com/blog/character-design-specifications),
consulted 7-Nov-2026: 1 mm text, 0.15 mm lines and 0.25 mm chosen pad clearance.
The board's editable settings and original font/path data are MIT OR Apache-2.0;
the linked manufacturer guidance retains its own rights and is not bundled.


## PCBcraft Technical stroke font

`crates/engine/assets/fonts/pcbcraft-technical/strokes.json` is a converted,
renamed font-data derivative of [Relief SingleLine SVG Regular](https://github.com/isdat-type/Relief-SingleLine),
revision `01dfc5779ec1e9e4b288d96c6c96c23bfccbaf9d`, copyright 2022 The Relief
SingleLine Project Authors (François Chastanet, Noëlie Dayma and Élisa Garzelli,
with contributors retained in the asset notices). This font data remains
**OFL-1.1**, independently of the project's code license. The asset directory
retains OFL.txt, AUTHORS.txt and CONTRIBUTORS.txt, plus the exact source hash
and [conversion recipe](crates/engine/assets/fonts/pcbcraft-technical/README.md).
The original converter flattens Bézier centerlines and preserves proportional
advances; runtime spacing and reference placement are original upstream code.
No external implementation, KiCad font data or artwork was incorporated. The
prior original font remains a distinct compatibility identity.

The hierarchical sheet model, commands, UI, native sheet/2 records and Sheet/Label
ribbon glyphs are original PCBcraft work. Control names and connection scope were
checked against the published [KiCad schematic editor manual](https://docs.kicad.org/10.0/en/eeschema/eeschema.html),
without consulting implementation source or copying icons. PCBcraft synchronizes
parent pins directly from child label identities; repeated sheet instances are
not included in this feature.


## PCBcraft Next toolbar port

The Capture, Analyze, Libraries, Manage and View tab composition in
`crates/app/src/ribbon.rs` adapts the UI tables and button/menu dispatch in
PCBcraft Next's `src/desktop/ribbon.rs` at
`568cd495397a91563e563e7163efb226d84fa83a` (MIT OR Apache-2.0, copyright
2026 Travis Lowe and contributors). This port retains the Place, Sheets, Connect,
Modify, Inspect and Design groups, routes implemented actions through upstream's
existing session/view handlers, and leaves unavailable tools disabled. Compact
layouts retain those unavailable controls in caption menus. Upstream's existing
Fit, Net Inspector, split/window controls and saved View-tab index are preserved.
No prototype document/session backend, importer or dependency lockfile is ported.

`crates/chrome/src/ribbon_icons.rs` adapts the embedded SVG consumer and two-stage
palette substitution in Next's `src/desktop/icons.rs` at the same revision. It
uses upstream chrome tokens and a palette-keyed egui image cache; only the SVG
loader is enabled, with no file or HTTP loader. The original SVG masters are
unchanged. `crates/chrome/assets/ribbon/provenance.json` records every source path,
revision and SHA-256, including the separate SolidCraft provenance of the five
family icons. Those family masters were carried in Next's `assets/family/icons/`
from SolidCraft revisions `60488a0837affcee79832bdb7bf6e860e84b10f2` (`spin.svg`)
and `c553365a2197a689e749389a6fbf707a38df9b61` (`move.svg`, `mirror.svg`,
`list.svg`, `configuration_table.svg`), MIT OR Apache-2.0, copyright 2026 SolidCraft
contributors. Both projects' license texts and SolidCraft's NOTICE are retained
beside the assets, included in About's Toolbar licenses, and copied into the web
build's `ribbon-notices/` directory. Save, Undo and Redo remain in upstream's app bar with their existing menu glyphs. This is a bounded UI/artwork adaptation; the chrome module remains
replaceable by the shared Craft UI layer.

SVG rendering adds `egui_extras` 0.36.2 with only its `svg` feature, using
`resvg`/`usvg` 0.45.1 and `tiny-skia` 0.11.4. The exact dependency closure is pinned
in `Cargo.lock` and checked by cargo-deny. These packages and their added
transitive dependencies use MIT, Apache-2.0, BSD-2-Clause or BSD-3-Clause licenses;
no GPL-family, file-fetching or HTTP dependency is introduced by this feature.


The subsequent toolbar cleanup gives each schematic action one tab owner:
Capture contains drawing/editing and hierarchical interfaces; Analyze contains
checks/reports; Libraries contains source management; Manage contains independent
sheet creation and project operations; View retains panels and layout. The
hierarchical-label command is named **Hierarchical label**, replacing the
prototype's broader **Port** label. The imported SVG master retains its original
`port.svg` filename and recorded hash. Move now dispatches upstream's existing
M/Disconnect and Move handler, and redundant Save/Undo/Redo ribbon actions were
removed in favor of the existing app bar. These layout/label changes are
original upstream adaptation code and do not import further prototype sources.


## Compact and expanded selection properties

The compact inspector and expanded component/placement form follow PCBcraft
Next's `src/desktop/inspector.rs` at
`568cd495397a91563e563e7163efb226d84fa83a` (MIT OR Apache-2.0, copyright
2026 Travis Lowe and contributors). Travis's FigCraft export
`designs/exports/workspaces-2026-10-30/02-schematic-properties.png`
(2026-10-30; SHA-256
`641c84e6d2569dc27d62459c34f1aec7b8d6d29ff9c130a2e88d5d3c0983c497`)
was a design reference for the compact card and expanded form composition only;
no artwork was copied from that export. The upstream implementation retains its own
view identities, revision checks, command transactions, field validation and
physical board painter. No prototype model, session or file format is imported.
The new package preview projects native definitions through existing chrome
primitives. Published manual observations and deliberate differences are recorded
in [the properties/editor notes](docs/properties-and-library-editors.md); no EDA
implementation source, external drawings, icons or example assets were used.


## Native library authoring workspaces

`crates/engine/src/authoring.rs`, `crates/app/src/library_editors*` and
`crates/chrome/src/authoring.rs` are original upstream code. Their workspace
composition uses the already qualified Craft chrome and properties primitives
above. The feature comparisons in `docs/properties-and-library-editors.md`
reference only published KiCad/Altium manuals; no implementation code, artwork,
external component definitions or examples were imported. Built-in definition
fork tests use this repository's original starter library data. The native
Close regression adds a direct development dependency on the already locked
winit 0.30.13 crate (Apache-2.0), already used by the desktop lifecycle tests;
no new dependency version or asset is introduced.

The shared picker integration, authoring ribbon groups and Select/Pin/Pad/Line/Rectangle/Circle
vector glyphs are original upstream code and artwork using the existing Craft
palette. Workspace identity and focus remain separate from project view targets;
no additional external source or asset was introduced.

Definition views now use the existing upstream pane/window layout and native
close controller. Per-pane authoring presentation, shared-session routing and
closed-pane field recovery are original upstream code; no external source,
asset or dependency was added for the layout integration.

The authoring usability expansion adds original integrated pin geometry, editable
technical artwork, oval pads, group transforms, duplication/arrays and clipboard
transactions. Published KiCad 10 and Altium user manuals were consulted for the
behavior comparison recorded in the properties/editor notes; no vendor code,
icons, library data or fixtures were used. Parameterized text and pin labels reuse
the existing qualified PCBcraft Technical stroke font, with its existing notices.
No new dependency or third-party asset was added.


## Parts Catalog and net inspection forms

The large Parts Catalog in `crates/app/src/parts_browser.rs` and
`crates/chrome/src/catalog.rs` is original upstream code using the existing
Craft source-manager form composition, palette and typography. It replaces the
floating Parts card described above while retaining the qualified catalog tiles,
starter ordering and native placement path. Library identity, revision checks,
source management and custom power names use the upstream session interfaces.
The Net Inspector now reuses the Browser's card/tree primitives, and net-label
properties reuse the shared aligned fields. The published [KiCad 10 Schematic Editor manual](https://docs.kicad.org/10.0/en/eeschema/eeschema.html)
informed the library-selection, preview and confirmation workflow;
no KiCad implementation source, artwork, library data or fixtures were used.
No dependency or third-party asset was added for this UI revision.


The optional compact Parts Catalog restores the already qualified floating-card,
edge docking and collapse implementation in `crates/chrome/src/workspace/catalog.rs`.
Its new Expand glyph, large-form Dock control and shared presentation state are
original upstream code. Both forms use the same native source catalog and typed
placement paths, with original regression fixtures and no new dependencies or
external assets. Compact layout persistence is user-local presentation data;
project document encodings are unchanged by this UI revision.


Direct named-anchor dragging, physical-contact pin feedback and the shared catalog
Repeat placement control are original upstream code. They reuse existing native
label/part commands, geometry queries and Craft controls; no outside implementation,
artwork, fixture or dependency was introduced. Regression circuits are constructed
through the existing engine commands.
