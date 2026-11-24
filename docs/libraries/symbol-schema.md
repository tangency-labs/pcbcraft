# Native symbol schema

Symbols own schematic drawings, logical terminals, electrical types and units.
They do not own footprints, package mappings or manufacturer identities. A
[part definition](part-schema.md) references an exact symbol revision and its
package choices; a placed part instance owns the reference and value.

The machine-readable schemas describe canonical standalone library files:

| Format | Schema | Additional fields |
| --- | --- | --- |
| `pcbcraft-symbol/1` | [Version 1](../schemas/symbol-v1.schema.json) | Existing drawings, terminals and one schematic unit |
| `pcbcraft-symbol/2` | [Version 2](../schemas/symbol-v2.schema.json) | Optional explicit electrical behavior and instance defaults |
| `pcbcraft-symbol/3` | [Version 3](../schemas/symbol-v3.schema.json) | An explicit default category binding |
| `pcbcraft-symbol/4` | [Version 4](../schemas/symbol-v4.schema.json) | Up to 32 independently placed units sharing logical terminals; category remains optional |
| `pcbcraft-symbol/5` | [Version 5](../schemas/symbol-v5.schema.json) | Optional alternate functions on logical terminals |

[The dispatcher](../schemas/symbol.schema.json) accepts these versions. All schemas
use JSON Schema Draft 2020-12 and registered URNs; validation needs no network
retrieval. Their common vocabulary is in
[library-common-v1.schema.json](../schemas/library-common-v1.schema.json).
Run `python3 scripts/check-library-schemas.py` from the repository root to check
the schemas, bundled definitions and original examples. Additional arguments
accept native record files. Python 3.10+ and `jsonschema` 4.18+ are required.

## Identity and ownership

A standalone symbol has `format`, `id`, positive 32-bit `revision`, `name`,
`reference_prefix`, `default_value`, `terminals` and `units`. Its library identity
and license come from `library.json`. A project's pinned copy additionally
carries its `library` and optional portable provenance; it is not the standalone
schema envelope. File paths, source registration IDs and the latest installed
library revision do not determine the pinned identity.

`format` versions the readable contract. `revision` versions immutable content.
The symbol editor's native export and publication write `pcbcraft-symbol/5`
for every new definition, with either one or several units. The schema supports
optional category, electrical behavior, instance defaults and alternate pin
functions. Removing optional records leaves the output at version 5. Published
files stay byte-for-byte intact, and project pins retain the original schema and
revision address.

Version 1 refuses electrical behavior/default fields, and versions 1–2 refuse
`category`. A version-3 symbol requires a non-null category binding.

The [symbol guide](symbol-library.md) defines the existing geometry, pin
appearance, defaults and power behavior. A unit can contain at most 441 pins on
the 1.27 mm grid. Formats 1–3 have one unit and therefore accept at most 441
terminals; formats 4 and 5 allow up to 1,024 logical terminals across multiple
units. The schema and engine validate the declared supported
fields rather than accepting arbitrary extension properties.

A placed part instance also owns `designator_locked`, independently of its
symbol definition and unit drawings. All placed units share that flag; unit
letters have no separate lock. Inspection includes false explicitly, and
`schematic.set_symbol_fields` edits it through an existing placed symbol.
[Project/51](../formats/native-project-storage.md#designator-locks) stores active locks
without changing any library symbol schema. Manual reference edits remain
available; automatic annotation itself is not implemented.

## Alternate pin functions

A logical terminal may declare `"alternates": [{"name":"ADC_IN","electrical_type":"input"}]`
alongside its primary `name` and `electrical_type`. Each present list contains one
through sixteen alternate functions. Names are case-sensitive, trimmed, single-line,
1–32 characters, unique within the terminal and distinct from its primary name.
Types use the existing terminal vocabulary. Every appearance of that terminal must
have an integrated pin lead; the choice does not override the lead's name visibility.
Alternates and their order belong to the shared terminal, not its individual unit drawings.
Formats 1–4 refuse the field. Adding an alternate adopts the
`pcbcraft-library-item/5` draft envelope, which that editor item retains after
the last alternate is removed. Native export and publication use
`pcbcraft-symbol/5` throughout.

A placed part can select one alternate per terminal. Selection does not renumber,
swap or move terminals or change connectivity. See [placed-part storage](../formats/native-project-storage.md#alternate-pin-function-records).
The current project format stores alternate choices and designator locks
independently. Project/56 remains readable for older projects that combine them.

## Explicit category defaults

The version-3 field has this shape:

```json
"category": {
  "definition": {"library": "parts-demo", "id": "resistor", "revision": 1},
  "primary_value": "pcbcraft:resistance"
}
```

`definition` names one exact category revision. The optional `primary_value`
names a quantity field in that category. A name such as Resistor, a reference
prefix R or a resistor-shaped drawing never supplies this binding implicitly.
An explicit part definition owns its own binding. The symbol's binding is the
default for a symbol-only workflow, not an override of a selected part definition.

The native parser, source diagnostics, project pinned-symbol codec and authoring
copy/export path retain this binding. Source inspection reports a missing or
conflicting category; it never fetches or substitutes one. Resolving a part
checks that part's exact category and field binding. Native part placement uses its own explicit part/category binding and typed
requirements, including canonical primary Value editing. Symbol-only placement
retains free text; synthesizing a project-local part definition from a bare
symbol remains separate authoring work. See [part-schema.md](part-schema.md).

## Validation beyond JSON Schema

The engine also checks relationships: terminal IDs are unique; every terminal
appears in at least one unit and every pin names a terminal; terminal appearances
and pin positions are distinct within each unit; unit IDs are unique;
integrated leads and parameterized drawing extents remain within bounds; power
behavior names an existing terminal; and default field appearances reference
valid metadata. Filled polygons must be convex and nonintersecting. Category
resolution checks the exact referenced identity, field existence and quantity
type. Schema validation alone cannot establish these cross-record relationships.

The canonical schemas omit absent optional fields. Some retained Rust decoders
also accept null or empty optional compatibility inputs and canonicalize them on
export; those inputs are not additional canonical spellings. No project format
or existing footprint/pad geometry is changed by publishing these symbol schemas.

## Multiple schematic units

A part can optionally reference one symbol definition, which can contain several
units. Each unit has a stable `id`, its own drawing and a subset of the global
`terminals` table. A terminal may appear once in each of several units, as with
shared supplies. Those appearances refer to one logical terminal; they cannot
have different names or electrical types. Every declared terminal must appear
somewhere in the definition, though a project may place only some units.
The [original dual-amplifier example](../../examples/native-parts/symbols/dual-amplifier.json)
shows two units with shared supplies and one complete six-terminal package map.

`schematic.place_symbol` creates a new part and places its first unit, or the
explicit optional `unit`. `schematic.place_unit` places another unit of an
existing `component-…` instance from its pinned definition. It requires `unit`,
position, `expected_revision` and an explicit `sheet` in a multi-sheet project.
A unit can be placed only once per part, across all sheets. Inspection reports
its unit, part ID, base reference, display reference and remaining available units.
The inspector's **Place unit** actions and the Parts Catalog's **Existing part
units** menu use that same command; the catalog menu can place units on another
sheet without requiring a unit to be present there already.

U1 A and U1 B have independent symbol IDs and positions but share U1's reference,
Value, metadata, package assignment and physical footprint. The space separating
the base reference and unit cannot occur in either identity, so `U1 A` cannot
alias another part's base reference `U1A`. Renaming through
either unit updates that part. Repeated appearances of a terminal join the
project-wide electrical graph by part/terminal identity, including across sheets;
board updates collapse those appearances into one physical terminal. Deleting a
unit, cutting it, or deleting its sheet preserves the part while another unit
remains. Removing the last unit follows the existing reviewed board-removal rule.
Ordinary copy allocates a new part; copying several units together, including in
a hierarchy subtree, allocates that part only once.

The symbol editor can add and select units through the shared authoring actions.
It retains source unit order, inactive drawings and globally unique primitive IDs in
the editable library item. Shared terminal name, type and alternate-list edits
propagate to other appearances, and native export uses format 5 for any unit
count. Explicit [reviewed pin renumbering](properties-and-library-editors.md#review-pin-renumbering)
changes only the authored definition, including its shared terminal appearances;
it adds no saved fields. It does not swap terminals/gates in placed instances or
update their native package mappings. Alternative symbol definitions for a part,
repeated circuit occurrences, high-pin-count geometry and a new BOM/ERC subsystem
remain separate work. Existing single-unit files keep their original
symbol formats and no-op serialization.

### Complete loose packages

Package completion is an explicit command, not a placement or annotation default.
With sheet scope, it only uses owners whose placed units are all on that sheet.
Project scope can combine placements across sheets. It first fills equivalent
slots in existing multi-unit packages, then combines remaining loose units in
stable part order. It never moves a part that already holds several units.

The pinned symbol, Value, reusable part/manufacturer selection, package
assignment, authored fields and part groups must match. A spare slot needs
the same drawing and an unambiguous match for each pin's role, position and
alternate functions. Shared pins keep their terminal numbers. Completion refuses the whole edit if
reassignment changes which pins are connected or joins or splits a net.
If a transfer would change net anchors, every meaningful net in the project
must already have an active identity record with a saved net number. This
includes nets on other sheets, even when completion is scoped to one sheet.
Preview and Apply both refuse the entire operation if any such record or number
is missing; neither changes the project or its Undo history.
Completion skips designator-locked owners, owners with authored terminal
no-connect intent or selected alternate functions, and placed packages without
an accepted board counterpart.

The command keeps placement IDs, positions, orientations and wire geometry.
The surviving owner keeps its ID and reference. Removed owners are retired;
accepted board footprints and copper are unchanged until reviewed through
[Update PCB](../pcb/single-board-update.md#preview-and-apply). The operation adds no
saved format and Undo restores the previous grouping in one step. See the
[MCP parameters and report](../automation/part-commands.md#native-part-definitions-and-quantities).
