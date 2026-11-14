# Native symbol schema

Symbols own schematic drawings, logical terminals, electrical types and units.
They do not own footprints, package mappings or manufacturer identities. A
[part definition](part-schema.md) references an exact symbol revision and its
package choices; a placed part instance owns the reference and value.

The machine-readable schemas describe canonical standalone library files:

| Format | Schema | Additional fields |
| --- | --- | --- |
| `pcbcraft-symbol/1` | [Version 1](schemas/symbol-v1.schema.json) | Existing drawings, terminals and one schematic unit |
| `pcbcraft-symbol/2` | [Version 2](schemas/symbol-v2.schema.json) | Optional explicit electrical behavior and instance defaults |
| `pcbcraft-symbol/3` | [Version 3](schemas/symbol-v3.schema.json) | An explicit default category binding |
| `pcbcraft-symbol/4` | [Version 4](schemas/symbol-v4.schema.json) | Up to 32 independently placed units sharing logical terminals; category remains optional |

[The dispatcher](schemas/symbol.schema.json) accepts these versions. All schemas
use JSON Schema Draft 2020-12 and registered URNs; validation needs no network
retrieval. Their common vocabulary is in
[library-common-v1.schema.json](schemas/library-common-v1.schema.json).
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
Changing either does not silently replace definitions in an existing project.
Version 1 refuses electrical behavior/default fields, and versions 1–2 refuse
`category`. A version-3 symbol requires a non-null category binding. Unchanged
older definitions retain their existing format and saved bytes.

The [symbol guide](symbol-library.md) defines the existing geometry, pin
appearance, defaults and power behavior. Formats 1–3 retain their one-unit limit. Format 4 supports multiple units
within the existing 128-terminal and per-unit drawing bounds; higher pin counts
remain separate work under #251. The schema and engine validate the declared supported
fields rather than accepting arbitrary extension properties.

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
The [original dual-amplifier example](../examples/native-parts/symbols/dual-amplifier.json)
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
`pcbcraft-library-item/2`; single-unit items keep format 1. Shared terminal
name/type edits propagate to the other appearances. Native export emits format 4
when several units exist. This does not add pin/gate swapping, alternative symbol
definitions for a part, repeated circuit occurrences, high-pin-count geometry,
or a new BOM/ERC subsystem. Existing single-unit files keep their original
symbol formats and no-op serialization.
