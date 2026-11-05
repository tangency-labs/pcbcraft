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

## Adapted from PCBcraft Next

PCBcraft Next, revision `568cd495397a91563e563e7163efb226d84fa83a`, MIT OR Apache-2.0. This is the independent prototype.

| PCBCraft file | PCBcraft Next source | What changed |
|---|---|---|
| `crates/engine/src/schematic/wiring.rs`, `crates/engine/src/lib.rs` | `src/document.rs`, `src/netlist.rs` | Adapted explicit terminal references and net traversal to the existing command session. This slice supports direct pin links on one schematic. |
| `crates/engine/src/schematic.rs`, `crates/engine/src/lib.rs` | `src/document.rs`, `src/definitions.rs`, `src/session.rs` | Adapted nanometer coordinates, distinct component and symbol identities, and the procedural resistor definition to the existing session and command history. |
| `crates/app/src/schematic_canvas.rs` | `src/desktop/canvas.rs` | Adapted the schematic grid and resistor zigzag for the shared desktop and browser canvas. Pin endpoints come from the engine and drawing metrics live in `crates/chrome`. |

## Bundled fonts

| File | Source | License |
|---|---|---|
| `crates/chrome/assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf` | IBM Plex Sans, IBM Corp., unmodified, by way of SolidCraft at the revision above | SIL OFL 1.1, text alongside |
| `crates/chrome/assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf` | IBM Plex Mono, IBM Corp., unmodified, by way of SolidCraft at the revision above | SIL OFL 1.1, text alongside |

## Dependencies

`Cargo.lock` lists them, and `deny.toml` is the list of licenses allowed. `epaint_default_fonts`, which egui bundles as its fallback, is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`. A notices file for release binaries is still to do, and is needed before the first release.
