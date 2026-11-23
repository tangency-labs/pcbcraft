# Reading a saved native project

Use this guide to locate parts, pins and connections in a saved snapshot. The
[native storage contract](native-project-storage.md#document-ownership) lists the
supported document versions and governs validation. If you only need a netlist,
prefer [the netlist export](netlist-format.md): it accounts for versions, labels,
power symbols and hierarchy without requiring you to implement those rules.

## Follow the document and identity boundaries

| To read | Start with | Follow |
| --- | --- | --- |
| Project and sheets | `project.json`: `format`, `id`, `name`, `sheets` | Each sheet’s `file` names its document; the document’s `id` must match the reference. See [layout and manifest rules](native-project-storage.md#packed-projects-and-manifests) for a `.pc` archive. |
| Logical parts | `components`: `id`, `reference`, `value`, `library`, `definition`, `definition_revision` | Resolve the pinned `library/symbols/<library>/<definition>.r<definition_revision>.json`, not an installed library’s latest revision. For selected parts and packages, see [part definitions](part-schema.md) and [schematic-only package choices](native-project-storage.md#schematic-only-package-choices). |
| Placed units and pins | Sheet `symbols`: `id`, `component`, `unit`; pinned symbol `terminals`, `units` and each unit’s `pins` | Resolve `component` to the owning part and `unit` to the definition’s unit. A placed pin’s `terminal` refers to a logical terminal ID, not its drawing order or label. See the [symbol schema](symbol-schema.md). |
| Wires and junctions | Sheet `wires`: `from`, `to`, optional `bends`; sheet `junctions`: `id` | A pin endpoint names a sheet symbol and pin; a junction endpoint names a junction. In the flat example below, follow endpoint references, not geometry: a crossing without a shared endpoint does not join wires. See [hierarchical sheet records](native-project-storage.md#hierarchical-sheet-records) and [named net records](native-project-storage.md#named-net-records) for other endpoint and name rules. |
| Net identity and instances | [Net registry](net-registry-storage.md) and [sheet occurrence records](sheet-occurrence-storage.md) | The sparse registry associates identities and names with resolved connectivity; it is not a complete wire graph. A sheet definition ID is not its occurrence path. |
| Physical placements and copper | Board registration and its `board.json` | Use the registered layout and board folder; see [single-board storage](single-board-storage.md), [multi-board storage](multiboard-storage.md) and [footprint and placement schemas](footprint-schema.md). The board’s accepted connection baseline and its copper are not the current schematic netlist. |

Read each document’s `format` before applying its field rules. Keep the ID
namespaces separate: a component counter, a symbol counter and a pin number
are not interchangeable. See the [object identity inventory](object-identity.md)
for which records carry persistent UUIDs and which use counters; references that
use counters still need to be resolved as specified by their document. A designator, display name,
file order or coordinate is not an identity.

## Worked example: one flat sheet

The [Python example](../scripts/read-flat-project-netlist.py) reads a trusted,
unpacked `pcbcraft-project/6` with one `pcbcraft-sheet/1` and ordinary
single-unit `pcbcraft-symbol/1` definitions. It emits numeric part IDs,
designators, values and logical pin fields, groups terminals through explicit
wire and junction endpoints, and lists unwired terminals. It makes no edits.
Duplicate terminal, unit-pin or junction IDs are rejected rather than collapsed
into an apparently valid result.
Run it against the directory containing `project.json`:

```sh
python3 -B scripts/read-flat-project-netlist.py /path/to/unpacked-project
```

This is not a general importer or file validator. It does not read an archive
itself or interpret newer layouts, net names, hierarchy, power behavior, board
connectivity or historical standalone formats. Use the production reader or
implement the corresponding version contracts for those cases.

The [frozen pre-release editing demo](../crates/storage/tests/fixtures/native-open/README.md)
is the exact test input. A [Python test](../scripts/test_flat_project_netlist.py)
reads its archived documents using only the standard library. A separate
[storage test](../crates/storage/tests/native_netlist_reference.rs) opens those same
bytes and runs `project.export_netlist`. Both compare parts, pin names and types,
net membership and unwired terminals to the same checked-in expected result.
The comparison is independent of export ordering, display net names and net UUIDs.

### Compare counts with a source project

Record the source project's counts separately, as a JSON object with exactly
three nonnegative integers: `parts`, `pins`, and `nets`. Use the same logical
scope on both sides: count each schematic part once, count all its logical pins
(including unwired pins), and count connected groups with at least one terminal
(not copper, physical pads, or isolated unwired terminals). The source counts
must come from the source project, not the reader's output.

```sh
python3 -B scripts/read-flat-project-netlist.py /path/to/unpacked-project --expected-counts /path/to/source-counts.json
```

The command prints the three counts only if they all match. A mismatch or an
unsupported input exits with status 1 and reports the error on stderr; neither
input is written. The same flat-format limits apply as for the listing mode.
Count agreement alone does not establish that pins or nets correspond one to
one. For automated application integrations, use `inspect_project` or netlist
export rather than this example.
