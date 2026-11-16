# Object identity inventory

Checks, comparisons, waivers and change records need to name a design object in a
way that survives edits, saves and reopening. This page records how each kind of
object is identified in the current native format and which kinds still lack a
persistent UUID. It describes the current engine; it does not change the format.

The inventory is enforced by `crates/engine/src/identity_tests.rs`. The test builds
a project through the shared commands, encodes it with the production document
encoder and classifies every saved record by its `id` field. A record that the
test cannot classify, a declared record that the fixture no longer saves, or a
kind whose identity no longer matches the gap list below fails the test.
When a kind moves to UUIDs, remove it from `EXPECTED_GAPS` in the same change.

## Identity forms

- **UUID**: a canonical lower-case UUID in the record's `id` field, allocated once
  and kept through edits, undo, save and reopen.
- **Counter**: a positive project counter, saved as a number and shown in commands
  as `symbol-N`, `wire-N`, `footprint-N` and similar. New counters are allocated
  above the saved [numeric watermark](multiboard-storage.md#numeric-id-allocation),
  so a deleted counter is not reused within the project's own history. Counters
  from separately edited copies of a project can still collide.
- **Owner token**: an owner's id plus a name, for objects that cannot exist without
  their owner, such as pins and pads.
- **Name**: an authored text key, such as a library definition's library, id and
  revision.

## Current inventory

| Kind | Saved record | Identity today |
| --- | --- | --- |
| Board | `project.json` `boards[]` | UUID |
| Sheet | `project.json` `sheets[]` and the sheet document | UUID |
| Sheet instance | sheet `subsheets[]` (sheet symbol); `sheet_occurrences[]` | Counter for the sheet symbol; UUID for the occurrence |
| Component | `project.json` `components[]` | Counter |
| Symbol unit, power port | sheet `symbols[]` | Counter |
| Wire | sheet `wires[]` | Counter |
| Junction | sheet `junctions[]` | Counter |
| Net label | sheet `net_labels[]` | Counter |
| Port | sheet `labels[]` (hierarchical labels and their sheet pins) | Counter |
| Pin | placed symbol plus pin number, `{"symbol":"symbol-N","terminal":"1"}` | Owner token on a counter |
| Net | `net_registry.records[]` | UUID |
| Net class | `project.json` `net_classes.definitions[]` | UUID |
| Footprint instance | `board.json` `footprints[]` | Counter |
| Pad | footprint plus pad number, `{"footprint":"footprint-N","pad":"1"}` | Owner token on a counter |
| Track segment | `board.json` `tracks[]` | UUID |
| Via | `board.json` `vias[]` | UUID |
| Board drawing item | `board.json` `graphics.items[]` | UUID |
| Layer | `board.json` `setup.layers[]` | UUID |
| Rule | `board.json` `rules` | UUID |
| Library item | library definition documents | Name |
| Arc track, pour, keepout, text, dimension | none yet | Not stored |

The stackup, its dielectric strata, circuit uses and the project itself also carry
UUIDs. Copper strata are addressed by their layer. Wire bends, drawing vertices,
package choices, accepted board connections, net anchors and class bindings belong
to their owner and have no id of their own.

Pins are addressed differently by different consumers. Commands and inspection
name a pin by its placed symbol unit, while net anchors and the accepted board
baseline name it by component and pin number. Pad numbers are distinct within a
footprint definition, so a pad token names one pad.

## Gaps and limits

The kinds without a persistent UUID are component, symbol unit, power port,
wire, junction, net label, sheet instance (the sheet symbol), port, footprint
instance and library item, plus pin and pad, whose tokens rest on counter owners.
Moving the schematic and footprint counters to UUIDs changes the saved format and
every command that names those objects, so it needs a project format step with a
migration that keeps older files loading.

Kinds that are not stored yet must be classified when their first record is
added; the audit fails on the new record until it is. The fixture does not
produce every optional record the encoder can write, such as net lineage after a
split or generated named-contact legs, so those records are not checked by the
audit until a fixture saves them.
