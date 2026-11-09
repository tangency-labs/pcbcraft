# Single board update foundation

The engine command `pcb.initialize_update_baseline` explicitly adopts the current
board's package choices and schematic connectivity as an accepted baseline. This
is the storage and retention foundation for [reviewed board updates](https://github.com/tangency-labs/pcbcraft/issues/55).
It does not yet provide an Update PCB dialog or a command that accepts later
schematic differences.

## Initializing accepted state

The command takes the addressed `board` and `expected_revision`, through the same
engine command path as other board edits. Every current component must have a
supported pinned physical assignment or an existing compatible legacy marker.
Missing choices, incompatible mappings and stale targets fail without changing
the project. Initialization is one undo step; a repeated invocation is a no-op
and does not accept subsequent schematic changes.

Without an enabled baseline, existing deletion and file-encoding behavior is
preserved. Once enabled, deleting a component through single-symbol, group or
sheet deletion removes its live schematic representation but retains accepted
footprints. Board views render those footprints using historical component
fields when the live source is absent. Inspection reports `pending_removals`,
and each retained footprint exposes its `accepted_component` and
`pending_removal` state. No missing live symbol is fabricated for cross-probing.

Reference/value fields follow the live component while it exists. At deletion,
their last visible values become descriptive display metadata for the retained
footprint. This overlay changes neither accepted terminal/package mappings nor
connectivity or source identity, and is not an authority for reconstructing the
captured source schematic. Other physical edits share the immutable baseline
across undo snapshots; the display overlay copies it only when values differ.

Unchanged accepted assignments can still be placed, moved and rotated. New or
changed package choices in baseline mode are explicitly refused until the reviewed
Apply workflow exists. Pending footprint removal is not performed by calling
initialization again. Ordinary explicit PCB footprint deletion remains available
and undoable; accepted history and its package assets remain until reconciled.

## Persisted records

Board format `pcbcraft-board/10` requires a `baseline` object. Earlier board
formats reject that field and retain their previous no-op encoding until baseline
initialization. Setup, silkscreen, graphics, legacy outline, assignments and
placements retain their existing records alongside it.

| Baseline field | Meaning |
| --- | --- |
| `format` | `pcbcraft-board-baseline/1` |
| `project`, `board` | Explicit owners; both must match the loaded design |
| `source` | Frozen source `capture_id` UUID and lowercase SHA-256 `content_digest` |
| `components` | Canonical positive numeric ID keys, with historical symbol identity, reference/value, sorted terminal tokens and accepted package |
| `nets` | Canonically ordered accepted component/terminal groups and authored names; these are historical membership, not persistent net IDs |

Each physical package keeps its pinned library/item/revision and exact
terminal-to-pad mapping. A legacy marker has a null library and its explicit
two-pad mapping; it cannot become an unplaced catalog package by implication.
Historical component records need no live schematic placement or installed symbol
library to validate their accepted terminal set. Physical assets remain pinned
while any placement, assignment or accepted record references them.

Proposed inputs may have unresolved package choices so a later preview can report
blockers; persisted accepted board records require a package for every component.

The baseline has at most 4096 component records with at most 128 terminal tokens
per component, within a 16 MiB serialized bound and the existing project/document
budgets. Validation rejects duplicate/noncanonical numeric keys, foreign owners,
unknown or multiply assigned terminals, invalid capture addresses and incompatible
package mappings. Named single-terminal groups can retain their display names;
unnamed isolated terminals are not stored as connections.

Source capture occurs before the baseline is added, avoiding a self-referential
digest. Hash the canonical document bundle in sorted path order, beginning with
`pcbcraft-board-source-bundle/1` followed by a NUL byte. Each document contributes
its UTF-8 path byte length as unsigned 64-bit big-endian, path bytes, document
byte length with the same encoding, and exact document bytes. A fresh capture
UUID distinguishes byte-identical sources captured from different allocation
histories. The source address does not promise that a full historical schematic
snapshot remains available; the retained baseline supplies the information needed
to inspect the accepted physical state.

## Following slices

The engine's pure update helpers normalize all-sheet connectivity to stable
component/terminal addresses, compare accepted and proposed inputs, report missing
package choices, and construct an atomic physical candidate. They preserve
unchanged placements and explicit mappings. Runtime preview authority, persisted
Apply, GUI/MCP workflows and ratsnest rendering remain later work. The future
command must publish the complete candidate, which already contains its matching
baseline, after checking the project opening, board, revision and accepted baseline.

Broader design boundaries are recorded in the [circuit instance contract](circuit-instances.md#package-intent-and-physical-acceptance)
and [storage design](multiboard-storage.md). Repeated circuits, multiple boards,
partial-use assignment and general net rules are not implemented by this foundation.
