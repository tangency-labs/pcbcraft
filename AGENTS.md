# PCBCraft

A complete EDA application being built in Rust, GPL-3.0-or-later, MCP-first.

Read README.md, ROADMAP.md and docs/architecture.md at the start of work.

- One main window. Document/workspace tabs and docked panes. Never spawn separate
  schematic, board, library, checks or 3D windows. Keep project and selection shared.
- Everything that edits the project is a typed engine command. UI, CLI and MCP
  call the same engine. Programmatic commands never open dialogs.
- Validate all input; finite bounded coordinates and dimensions, bounded counts,
  stable IDs, reference integrity. Invalid commands and batches leave state untouched.
- No unsafe or production unwrap/expect/panic/todo/unimplemented/unreachable.
- Lower crates never depend on egui, eframe, wgpu, or MCP. UI is replaceable.
- KiCad source may be inspected and GPL-compatible code adapted. Record exact
  source/revision/license in ATTRIBUTION.md for adaptations. Translation to Rust
  does not remove attribution or copyleft requirements. Check individual licenses.
- Shared Craft dependencies retain their licenses and notices. Do not copy ArtCraft
  trademarks or proprietary assets. Pin craft-libs revisions.
- Keep real feature limitations visible. Do not represent initial checks as complete
  ERC/DRC or a basic route as a shove router. Track parity by workflows and fixtures.
- Before committing: cargo xtask ci. Inspect rendered UI for UI changes.
- Do not publish releases or push to third-party repositories without authorization.
