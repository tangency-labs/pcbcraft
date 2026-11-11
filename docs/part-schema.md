# Native part and category schemas

A **part definition** is reusable library data for a resistor, IC, connector or
another electrical part. It references one symbol and zero or more package
variants. A **part instance** is one use in a project, such as R1 or U1, with its
own stable identity, reference, value and requirements. A **manufacturer part**
identifies a particular manufacturer's MPN and its specifications for one part
definition and package variant. Several instances or manufacturer parts can
share one reusable definition.

These are the record boundaries in [the library model](library-sources.md).
The implementation uses `PartDefinition` for reusable records and `PartInstance`
for the engine type previously called `Component`. Existing project JSON
`components`, component-addressed command parameters and stable `component-…`
IDs retain their compatibility meaning: they address placed part instances.
They never refer to a library part definition. New library APIs use an explicit
library/item/revision address. **Rename part** changes an instance reference,
while **Edit copy** authors a separate library definition.

## Formats and implementation

| Record | Format | Machine-readable schema |
| --- | --- | --- |
| Part definition | `pcbcraft-part/1` | [part-v1.schema.json](schemas/part-v1.schema.json) |
| Category definition | `pcbcraft-category/1` | [category-v1.schema.json](schemas/category-v1.schema.json) |
| Shared references and value vocabulary | Schema vocabulary version 1 | [library-common-v1.schema.json](schemas/library-common-v1.schema.json) |

The engine reads, validates and deterministically encodes both record kinds.
Native folder loading, portable bundles, source inspection and search accept
`parts/*.json` and `categories/*.json`. `library.resolve_part` validates exact
dependencies, typed fixed values and every package map without mutating a
project. The same source/resolution operations are available through MCP.
Library sources displays the record kinds and reports unresolved dependencies.

This implements the definition contract and library ingestion/resolution, not
the entire manager or placement workflow. Ordinary Parts Catalog placement still
selects symbols; selecting/pinning a reusable part and category into project
instances, typed per-instance requirements, manufacturer-part records and
assembly-BOM matching remain integration work under #92/#28. In particular, a
resolved library record is not a claim that it has been placed or manufacturing
qualified. The existing footprint and pad-stack formats remain unchanged.

## Exact references

All records have a fixed `format`, portable `library` and `id`, positive unsigned
32-bit `revision`, display `name` and `license`. Identity segments contain 1–64
lowercase ASCII letters, digits and internal hyphens. Names are trimmed,
control-free text of at most 64 characters; licenses have the same rules with a
128-character limit. File names are locators. Moving or removing a source never
changes a record's identity or a project's already pinned definitions.

A reference has exactly `library`, `id` and `revision`; its containing field
defines the target kind. Resolution never chooses the newest version. Conflicting
content claiming the same identity/revision is quarantined. Dependencies may
come from different loaded libraries. Missing, conflicting or unavailable
dependencies remain explicit source diagnostics and unresolved search results.
Adding/removing/refreshing a source invalidates dependency-resolution caches.

## Part fields

| Field | Meaning |
| --- | --- |
| Common identity fields | `format`, `library`, `id`, `revision`, `name`, `license` |
| `source` | Required provenance `kind`, `reference`, `locator`; descriptive metadata, never fetched on load |
| `symbol` | Exact symbol reference |
| `category` | Optional exact category `definition` and optional `primary_value` field binding |
| `fixed_parameters` | Optional nonempty map of namespaced field IDs to known typed fixed values; requires `category` |
| `packages` | Required map of stable variant IDs to package records; an empty map is valid during schematic work |

No manufacturer, MPN, stock, price, drawing, terminal geometry or pad geometry
belongs in this record. The part links the independently versioned definitions.
An uncategorized part with no packages is valid. A fixed value is an exact
requirement of every instance using the definition; it is not an editable
instance default. An omitted fixed value imposes no fixed requirement. Unknown
or unsupported manufacturer specifications are not represented as fixed values,
zero or guessed defaults.

A package has `name`, an exact `footprint`, a `mapping` and optional
`unconnected_pads`. The mapping's keys are symbol terminal IDs; each value is
one or more footprint pad identifiers. Every terminal must be mapped, and every
pad must occur exactly once either in one mapping entry or as explicitly
unconnected. Multiple lands can belong to one terminal, but one pad cannot
belong to two terminals. Identifier spelling is preserved, including `01`, `EP`
and alphanumeric names. Names, positions and package size codes never establish
a map. Parts support at most 64 packages; this first native contract retains
the current 128-terminal/pad limits.

The existing board assignment command still uses its older one-to-one map.
Loading or resolving a multi-pad package does not silently flatten it into that
command. Project assignment integration needs its own versioned transaction and
saved mapping representation.

## Category fields and electrical characteristics

A category's required `fields` map contains 1–64 fields keyed by stable
`namespace:field` IDs. Both segments follow the portable ID rules. Each field
has a display `name` and `value_type`. Display labels never establish identity
or type. Libraries can add namespaced fields without redefining another field's
type, physical quantity or canonical unit. Category-specific integer bounds
and permitted enumeration choices do not change the underlying field type.

| `value_type.type` | Required type data | Fixed value record |
| --- | --- | --- |
| `quantity` | `quantity`, `unit` | `type`, canonical decimal-string `value`, canonical `unit` |
| `integer` | Ordered inclusive `minimum`, `maximum` | `type`, integer `value` |
| `choice` | 1–64 distinct `choices` | `type`, a permitted string `value` |
| `text` | None | `type`, trimmed control-free `value` of at most 128 characters |
| `boolean` | None | `type`, boolean `value` |

A resistor category can declare resistance, tolerance and power rating; a
connector can declare contact count, pitch and rated current; an IC can declare
supply-voltage limits and functional choices. These are fields of category
definitions, not new top-level part kinds. A part's optional `primary_value`
must name a quantity field in its exact category. Fixed values must name existing
fields and satisfy their types, units, bounds and permitted choices.

## Canonical quantities

Records store base units, independently of display-unit preferences:

| Quantity | Stored unit |
| --- | --- |
| resistance | `Ohm` |
| capacitance | `F` |
| inductance | `H` |
| voltage | `V` |
| current | `A` |
| power | `W` |
| frequency | `Hz` |
| time | `s` |
| length | `m` |
| temperature | `K` |
| ratio, including tolerance | `1` |

For example, `100m`, `100 mΩ`, `100 milli ohms`, `0.1R` and `0.1 Ohm` in a
resistance field all normalize to:

```json
{"type": "quantity", "value": "0.1", "unit": "Ohm"}
```

Decimal values are strings with no exponent, leading plus, redundant leading
zeros or trailing fractional zeros. Zero is exactly `"0"`. There are at most
36 total digits and 18 fractional places. Normalization uses decimal digit
shifts, never binary floating-point conversion or rounding. Excess magnitude
or precision is rejected. Integer fields stay within ±9,007,199,254,740,991 so
JSON consumers can represent them exactly.

`library.parse_quantity` accepts a declared quantity and up to 128 input bytes.
It recognizes decimal/scientific input, compatible base units and case-sensitive
SI prefixes `p`, `n`, `u`/`µ`/`μ`, `m`, `k`, `M`, `G`, `T`; resistance also accepts
`Ohm`, `ohm`, `ohms`, `Ω`, `R` and the `milli` prefix. A bare prefix uses the
field's unit. An explicit base unit takes precedence, so `100 m` in a length
field means 100 metres and `100 mm` means 0.1 metres. Percentage input normalizes
to a ratio: `1%` becomes `"0.01"` in unit `"1"`. Unsupported units, including
affine temperature conversions, are rejected. Celsius is not silently treated
as Kelvin. Display formatting can choose a compatible scale without changing
these stored values; this change does not add a new unit-preferences UI.

Canonical record readers reject noncanonical decimal/unit spellings such as
`"0.10"`, `"1e-1"` or unit `"mOhm"`. Input normalization is a separate shared
operation. This prevents two saved records from expressing 0.1 Ω as different
quantities merely because they were entered or displayed at different scales.

## Validation and original example

JSON Schema enforces record structure, field ownership, scalar bounds and
canonical encodings. The engine additionally rejects duplicate map identities,
invalid trimming, unordered integer bounds, duplicate pad use across entries,
unknown referenced terminals/pads, category type/field conflicts and unresolved
exact dependencies. A structurally valid record can therefore still be
unresolved. Definition-local validity and complete dependency resolution are
separate inspection states, not a partial project assignment.

[`examples/native-parts`](../examples/native-parts/README.md) contains an original
resistor definition, category, symbol and footprint. It is usable through both
the native folder loader and browser-uploadable bundle path. Schema checks and
regressions run with:

```sh
python3 scripts/check-library-schemas.py
python3 -B -m unittest discover -s scripts -p 'test_library_schemas.py'
cargo test --locked -p pcbcraft-engine library::parts
```

Formats and content revisions remain independent. Future authored fields or
changed meanings require an explicit schema version and compatibility tests;
unchanged project/symbol/footprint records retain their existing encodings.
