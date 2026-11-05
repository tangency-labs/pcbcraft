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

## Adapted from PCBcraft Next

PCBcraft Next, revision `568cd495397a91563e563e7163efb226d84fa83a`, MIT OR Apache-2.0. This is the independent prototype.

| PCBCraft file | PCBcraft Next source | What changed |
|---|---|---|
| `crates/engine/src/schematic.rs`, `crates/app/src/schematic_canvas.rs` | `src/document.rs`, `src/definitions.rs`, `src/desktop/canvas.rs` | Adapted the original generic LED definition and drawing to the shared placement command and canvas. Preserved numbered terminals and identified their polarity in inspection. |
| `crates/engine/src/schematic/wiring.rs`, `crates/engine/src/lib.rs` | `src/document.rs`, `src/netlist.rs` | Adapted explicit terminal references and net traversal to the existing command session. This slice supports direct pin links on one schematic. |
| `crates/engine/src/schematic.rs`, `crates/engine/src/lib.rs` | `src/document.rs`, `src/definitions.rs`, `src/session.rs` | Adapted nanometer coordinates, distinct component and symbol identities, and the procedural resistor definition to the existing session and command history. |
| `crates/app/src/schematic_canvas.rs` | `src/desktop/canvas.rs` | Adapted the schematic grid and resistor zigzag for the shared desktop and browser canvas. Pin endpoints come from the engine and drawing metrics live in `crates/chrome`. |
| `crates/app/src/schematic_canvas/camera.rs`, `crates/app/src/schematic_canvas.rs` | `src/desktop/canvas.rs`, `src/desktop/page.rs` | Adapted the viewport transform, pointer-anchored zoom, middle-button pan, and fit behavior to Tangency’s canvas and chrome. |
| `crates/app/src/schematic_canvas/wires.rs`, `crates/app/src/lib.rs` | `src/desktop/canvas.rs`, `src/desktop/mod.rs`, `src/desktop/wire_geometry.rs` | Adapted pin hit testing, the Wire tool shortcut, the cancelable preview, and a symmetric subset of the orthogonal display routes. Placement dispatches through Tangency’s existing pin-connection command. |
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

## Bundled fonts

| File | Source | License |
|---|---|---|
| `crates/chrome/assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf` | IBM Plex Sans, IBM Corp., unmodified, by way of SolidCraft at the revision above | SIL OFL 1.1, text alongside |
| `crates/chrome/assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf` | IBM Plex Mono, IBM Corp., unmodified, by way of SolidCraft at the revision above | SIL OFL 1.1, text alongside |

## Dependencies

`Cargo.lock` lists them, and `deny.toml` is the list of licenses allowed. `epaint_default_fonts`, which egui bundles as its fallback, is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`. A notices file for release binaries is still to do, and is needed before the first release.
