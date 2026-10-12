# Architecture and decisions

PCBCraft follows the Craft application structure, with a headless design engine
that is the sole mutation boundary. GPL-3.0-or-later permits future attributable
KiCad implementation ports. Rust-only applies to the delivered application.

| Layer | Crates | Responsibility |
|---|---|---|
| L0 | geom | Bounded millimetre geometry and distance operations |
| L1 | doc | Stable-ID project model, component definitions, pins, nets, pads, tracks |
| L2 | rules, io | Connectivity and initial checks; native project, KiCad board and SVG export |
| L3 | engine | Typed command registry, transactions, revisions, undo/redo |
| L4 | mcp, ui-egui | MCP over stdio / live bridge; thin tabbed Rust UI using craft-ui |
| Apps | pcbcraft, pcbcraft-cli, pcbcraft-web | Desktop, headless and browser entry points |

## MCP-first

The command registry publishes JSON schemas and descriptions. `execute` and
`batch` mutate a candidate document, validate it, and commit once. Failures preserve
the document, revision and history. Stable IDs survive edits, save/open and undo.
Optional expected revisions reject stale agent mutations. Undo branches never reuse
an allocated ID. Read tools expose actual project state and current limitations.

Headless MCP owns a session. Live MCP forwards calls over a loopback-only JSON-lines
bridge into the exact session displayed by the desktop. The bridge is local trusted
automation, not a remotely exposed service; do not bind to all interfaces.

## Single-window flow

Schematic, PCB, Libraries and Checks are workspace tabs in one window. Browser,
inspector and command entry are inline panes. 3D is reserved for a future real renderer.
Switching tabs carries the selected component into its corresponding board footprint.
Properties apply through commands, and checks can select the affected item.

## Initial model and limits

Pins use explicit logical nets. Wires are schematic presentations between those pins.
Built-in original component templates have simple generic SMD footprints; no third-party
library artwork is bundled. PCB sync reconciles component footprints and pin nets.
Copper connectivity joins touching same-net tracks and pads, separated by layer.
Initial clearance checks use capsule tracks and circular pad bounds. Arbitrary pad
shapes, vias, zone fills, hierarchy and comprehensive electrical rules remain future work.
The initial KiCad writer targets a small documented subset and must be checked with
KiCad CLI; there is no general KiCad importer yet.
