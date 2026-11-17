# Object identity inventory

Checks, comparisons, waivers and change records need to name a design object in a
way that survives edits, saves and reopening. This page records how each kind of
object is identified in the current native format and which kinds still lack a
persistent UUID. It describes the current engine; it does not change the format.

The inventory is enforced by `crates/engine/src/identity_tests.rs`. The test builds
a project through the shared commands, encodes it with the production document
encoder and classifies each record it saves by its `id` field. A record that the
test cannot classify, a declared record that the fixture no longer saves, or a
kind whose identity no longer matches the gap list below fails the test.
When a kind moves to UUIDs, remove it from `EXPECTED_GAPS` in the same change.

The test registers the places in a saved document where an object of some kind
can be stored:
- every JSON object, however deeply nested and whether or not it has an `id`;
- every element of a list, including plain values such as the wire ends in a
  sheet's no-connect list;
- every value under a key that is an id;
- the element path of every empty list.

Each registered path must be declared in the test's `RECORDS` table, including
plain owned content such as positions, rule values and page settings. A list that
is empty in the fixture is declared with no forms, so the first element saved
there has to be classified. Only a named scalar field is left out: it holds one
value of its owner, not a collection of objects. Library definitions are
versioned as a whole, so only their document root is registered.

The test lists owned content path by path rather than recognising it by shape,
for example "any object with only coordinates is a point". A shape rule would let
a new kind with that shape in, unseen. Declaring every path costs one line per
new field, and that line is where the field's identity is decided. A separate
test adds a nested object without an id, lists of objects, plain values and
nothing, values under id keys and a new document, and checks that each is
reported.

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
| Part group | `project.json` `part_groups.groups[]` | UUID |
| Footprint instance | `board.json` `footprints[]` | Counter |
| Pad | footprint plus pad number, `{"footprint":"footprint-N","pad":"1"}` | Owner token on a counter |
| Track segment | `board.json` `tracks[]` | UUID |
| Via | `board.json` `vias[]` | UUID |
| Zone outline | `board.json` `zones[]` | UUID; vertices and fill settings are owned content |
| Board drawing item | `board.json` `graphics.items[]` | UUID |
| Layer | `board.json` `setup.layers[]` | UUID |
| Rule set | `board.json` `rules` (one per board; its values have no id of their own) | UUID |
| Library item | library definition documents | Name |
| Arc track, pour, keepout, text, dimension | none yet | Not stored |

The stackup, its dielectric strata, circuit uses and the project itself also carry
UUIDs. Copper strata are addressed by their layer. Wire bends, drawing vertices,
zone vertices and fill settings, no-connect intent on pins and wire ends, package choices, accepted board
connections, net anchors, class bindings and part-group memberships belong to their owner
and have no id of their own. A part-group membership names its part instance's counter
and its group's UUID; the display name and designator are not identity.

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
added. The audit fails on such a record, unclassified, once the fixture saves
it. The fixture does not produce every optional record the encoder can write,
such as net lineage after a split or generated named-contact legs. Those records
are not checked by the audit until a fixture saves them; the empty lineage list
is declared, so its first entry will be.
