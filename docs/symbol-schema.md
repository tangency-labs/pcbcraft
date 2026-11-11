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
appearance, defaults and power behavior. These schemas retain the current
one-unit, 128-terminal and drawing bounds; they do not introduce multi-unit or
high-pin-count support. The schema and engine validate the declared supported
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
checks that part's exact category and field binding. Creating project-local
part definitions during ordinary symbol placement and replacing the existing
free-text instance Value editor with typed requirements remain separate
integration work under #92. Preserving this field does not claim that today's
Value editor already enforces it.

## Validation beyond JSON Schema

The engine also checks relationships: terminal IDs are unique; every terminal
has exactly one pin and every pin names a terminal; pin positions are distinct;
integrated leads and parameterized drawing extents remain within bounds; power
behavior names an existing terminal; and default field appearances reference
valid metadata. Filled polygons must be convex and nonintersecting. Category
resolution checks the exact referenced identity, field existence and quantity
type. Schema validation alone cannot establish these cross-record relationships.

The canonical schemas omit absent optional fields. Some retained Rust decoders
also accept null or empty optional compatibility inputs and canonicalize them on
export; those inputs are not additional canonical spellings. No project format
or existing footprint/pad geometry is changed by publishing these symbol schemas.
