# Native part and category schemas

A **part definition** is reusable library data for a resistor, IC, connector or
another electrical part. It references one symbol and zero or more package
variants. That symbol may contain several schematic units, all belonging to
the same physical part and complete terminal-to-pad mapping. A **part instance** is one use in a project, such as R1 or U1, with its
own stable identity, reference, value and requirements. A **manufacturer part**
identifies a particular manufacturer's MPN and its specifications for one part
definition and package variant. Several instances or manufacturer parts can
share one reusable definition.

These are the record boundaries in [the library model](library-sources.md).
The implementation uses `PartDefinition` for reusable records and `PartInstance`
for the engine type previously called `Component`. Existing project JSON
`components`, component-addressed command parameters and stable `component-…`
IDs retain their compatibility meaning: they address placed part instances.
The accepted board snapshot type is `PartInstanceState` (formerly
`ComponentState`); this Rust rename does not change persisted keys or IDs.
They never refer to a library part definition. New library APIs use an explicit
library/item/revision address. **Rename part** changes an instance reference,
while **Edit copy** authors a separate library definition.

## Formats and implementation

| Record | Format | Machine-readable schema |
| --- | --- | --- |
| Part definition | `pcbcraft-part/1` | [part-v1.schema.json](schemas/part-v1.schema.json) |
| Manufacturer part | `pcbcraft-manufacturer-part/1` | [manufacturer-part-v1.schema.json](schemas/manufacturer-part-v1.schema.json) |
| Project part selection | Project format 15 | [part-selection-v15.schema.json](schemas/part-selection-v15.schema.json) |
| Category definition | `pcbcraft-category/1` | [category-v1.schema.json](schemas/category-v1.schema.json) |
| Shared references and value vocabulary | Schema vocabulary version 1 | [library-common-v1.schema.json](schemas/library-common-v1.schema.json) |

The engine reads, validates and deterministically encodes these record kinds.
Native folder loading, portable bundles, source inspection and search accept
`parts/*.json`, `categories/*.json` and `manufacturer-parts/*.json`. `library.resolve_part` validates exact
dependencies, typed fixed values and every package map without mutating a
project. The same source/resolution operations are available through MCP.
Library sources displays the record kinds and reports unresolved dependencies.

The Parts Catalog places resolved native parts, with an optional manufacturer
selection and package, in one revision-checked transaction. The project pins the
part, category, symbol, all supported package footprints and selected manufacturer
record so it remains editable after source removal. Symbol-only placement remains
available for libraries without central part definitions. The native assignment
chooser also lists external native footprints.

This slice does not include KiCad import, vendor HTTP connectors, supplier stock
or prices, parametric catalog filtering or the complete manager. Native part authoring
uses session documents in Parts Library, as described in the
[source guide](native-library-sources.md#generic-and-orderable-part-documents). Manufacturer records use the same library ingestion and provenance
contracts that a later vendor adapter can produce. No provider is required to
open or edit a saved project. Footprint geometry and general pad stacks retain
their existing contracts.

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

Physical assignments preserve a single-pad string or a multi-pad array per
terminal, plus optional `unconnected_pads`. Multi-pad/NC assignments require
board format 17 and accepted baseline format 2, or project format 15 for a
schematic-only choice. The [placement schema](schemas/footprint-placement-v17.schema.json)
covers the new physical record; `check-footprint-schemas.py --placement-version
17 --placement <record.json>` validates it offline. The engine additionally checks
complete mapping.
Every mapped land participates in net membership and the ratsnest, including
multiple lands on an otherwise unwired terminal. An NC pad is inspectable but
has no logical terminal or inherited net.

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

The [shared quantity model](quantity-units.md) in `pcbcraft-units` owns standard
quantity identities, canonical units, aliases, prefixes and exact normalization.
Category fields reference those definitions; native part records retain the
typed value shape below.

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

The editor shows stored quantities with an SI prefix and unit symbol, for
example `0.0000001 F` as `100 nF`, `4700 Ohm` as `4.7 kΩ` and `32768 Hz` as
`32.768 kHz`. It picks the engineering prefix from atto (`a`, 10⁻¹⁸) through
exa (`E`, 10¹⁸) that leaves one to three digits before the decimal point.
Examples include `1 fF`, `1 aF`, `1 PHz` and `1 EHz`; larger values keep exa
(`1000 EHz`). Values below 10⁻¹⁸ base units exceed the stored decimal precision
and are rejected without rounding. Zero has no prefix and ratios show the plain decimal.
Every significant digit is kept and nothing is rounded, so the shown text parses
back to the same stored value: submitting it unchanged is a no-op. Display never
changes saved data, MCP output or the component's stored Value text.

`library.parse_quantity` accepts a declared quantity and up to 128 input bytes.
It recognizes decimal/scientific input, compatible base units and case-sensitive
SI prefixes `a`, `f`, `p`, `n`, `u`/`µ`/`μ`, `m`, `k`, `M`, `G`, `T`, `P`, `E`; resistance also accepts
`Ohm`, `ohm`, `ohms`, `Ω`, `R` and the `milli` prefix. The Ohm sign (U+2126) and
Kelvin sign (U+212A), which look like `Ω` and `K`, are read as those letters. A bare prefix uses the
field's unit. An explicit base unit takes precedence, so `100 m` in a length
field means 100 metres and `100 mm` means 0.1 metres.
Scientific exponent notation is distinct from exa: `1E3 Hz` means 1000 Hz,
while `1EHz` or `1E` in a frequency field means 10¹⁸ Hz.
Percentage input normalizes to a ratio: `1%` becomes `"0.01"` in unit `"1"`. Unsupported units, including
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

## Manufacturer parts and design requirements

A manufacturer record has its own exact identity, provenance and license, plus
`manufacturer`, `mpn`, an exact `part` reference, a declared `package`, optional
HTTP(S) `datasheet` URL and typed `specifications`. Loading a record never fetches
the URL. Specifications must use fields from the referenced part's category and
canonical base units. Manufacturer names and MPNs are metadata, not native
identity keys; the synthetic example is explicitly illustrative.

An instance selection stores its exact part reference, optional package and
manufacturer references, and `requirements`. Each field supports `exact`,
`at_least`, `at_most`, `range` or `one_of`; ordered comparisons require numeric
values of the declared type and unit. Decimal comparisons retain the full
canonical precision, without floating-point conversion. Generic fixed values
remain exact requirements and cannot be overridden. Missing manufacturer
specifications produce unknown suitability rather than zero or a guessed value.

`schematic.place_part` validates the part dependency digest and optional
manufacturer digest before publishing its symbol, package assignment and
selection together. Choosing an incompatible MPN is refused. `part.set_selection`
edits package, requirements and optional MPN through the same transaction path
from either editor. Later requirement or package edits retain an existing MPN
and report incompatibility; they never silently pick an alternate. The part
properties form exposes both requirements and manufacturer specifications.
Editing the ordinary Value of a categorized native instance normalizes its
explicit primary quantity, while symbol-only instances retain free text.

## Source organization and versioning

`crates/engine/src/library/parts/` owns reusable definitions, categories, values,
package maps, manufacturer metadata and requirement comparisons in
separate modules. `library/sources.rs` owns catalog resolution and dependency
digests. `part_instances/` owns project selections, shared transactions and pinned
storage; the app's `parts_browser/native_parts.rs` and
`native_part_properties.rs` own presentation. UI controls delegate parsing and
compatibility rules to the engine.

`crates/units/src/quantity/` owns standard quantity/unit metadata and canonical
quantity values; `crates/units/src/quantity.rs` owns exact parsing and display.
The engine re-exports shared quantity APIs and adapts parsed results to native
typed values. Category and manufacturer semantics remain engine-owned.

Record `revision` identifies a new immutable item revision. A `format` or schema
version changes when a parser needs a different structural contract. The app
release version advances independently and remains 0.1.0. Project format 15
adds part selections to the storage vocabulary. It supports format 14's board
folders and retains a historical flat layout until ordinary first-save migration.
Projects without the new data retain their low-level encoding and no-op saves;
the existing first-save storage migration still applies.
Accepted board snapshots retain an optional canonical primary quantity under
baseline/3, so a deleted typed part awaiting reviewed removal keeps its readable
SI value in PCB Properties and the project browser. Capture uses the resolved
primary requirement or fixed parameter, with no type inferred from text. Old
baselines remain text-only until a real update accepts new history; see the
[storage contract](native-project-storage.md) for compatibility and validation.

Older apps cannot read project/15, board/17 or baseline/2 or /3; saving these features
therefore requires the newer app. This introduces no board-list migration.
