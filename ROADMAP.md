# Roadmap and current status

The target is a complete Rust EDA application with MCP access to every feature.
This checklist records implementation, not an estimated parity percentage.

## M0 — MCP-first foundation (implemented)

- [x] Rust workspace and enforced dependency direction
- [x] Stable IDs, typed commands, discoverable schemas, revision guards
- [x] Transactional edits/batches, undo/redo, validated project persistence
- [x] MCP stdio and loopback bridge into the displayed session
- [x] Single-window tabbed editor, shared project and cross-tab component selection
- [x] Pinned craft-ui integration; dark/light/accent/grid preferences
- [x] Native and WebAssembly compilation gates

## M1 — First circuit (implemented subset)

- [x] Four original two-pin symbol templates and generic SMD footprints
- [x] Explicit pin nets and orthogonal schematic wire presentation
- [x] Schematic-to-board reconciliation with retained footprint placement
- [x] Basic straight and orthogonal tracks, same-layer copper connectivity, airwires
- [x] Initial ERC/DRC with affected object references
- [x] Rectangular outline dimensions and circular cutouts
- [x] KiCad PCB subset export and external KiCad CLI validation
- [x] Read/search/preview installed KiCad symbol and footprint libraries
- [x] Embed placed single-unit symbols and assigned front SMD footprints
- [x] Actual pad geometry in UI, previews, clearance checks and KiCad export
- [ ] Editable library symbols/footprints and multi-unit symbol placement
- [ ] Through-hole/back-side placement and broader library compatibility
- [ ] Explicit wire vertices, junctions, labels, buses and hierarchy
- [ ] General KiCad import with unknown-field preservation and round-trip fixtures
- [ ] Complete pin-type electrical rules and project/net-class rules
- [ ] Browser download/upload and project recovery
- [ ] Configurable shortcut profiles and complete KiCad default keymap

## M2 — Serious board layout

- [ ] Arbitrary pad shapes, through-hole pads, vias and multilayer stackups
- [ ] Interactive 45° routing, obstacle avoidance, shove and track dragging
- [ ] Differential pairs, length tuning, impedance-aware constraints
- [ ] Copper zones, thermal reliefs, islands, keepouts, robust polygon operations
- [ ] Comprehensive clearance and manufacturing checks
- [ ] Gerber, Excellon, placement files, BOM and fabrication packages

## M3 — Sketch-driven board geometry and mechanical coordination

- [ ] Line/arc outline sketch model inspired by SolidCraft's planar sketch workflow
- [ ] Dimensions, constraints, named parameters and expression-driven regeneration
- [ ] Slots, notches, mounting holes and dimensioned patterns
- [ ] PCB/enclosure coordination, shared mounting datums and STEP exchange
- [ ] Real 3D board tab with component bodies and inspection

## M4 — Complete workflows and distribution

- [ ] Simulation strategy compatible with the Rust-only constraint
- [ ] Full schematic/PCB/library editor workflows and reference-project parity suite
- [ ] Autosave/recovery, project migration, large-board responsiveness
- [ ] Native Windows/Linux/macOS packaging, browser distribution
- [ ] Craft Launcher integration where authorized by its maintainers

No feature is considered complete solely because a command exists. Workflows need
usable UI, working MCP access, reference fixtures and meaningful verification.
