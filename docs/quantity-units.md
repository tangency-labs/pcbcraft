# Shared quantities and units

`pcbcraft_units::quantity` owns standard quantity identities, canonical unit
metadata, input aliases, display symbols, engineering prefixes and exact decimal
normalization. Native categories refer to that shared `Quantity` type. The engine
owns category bindings, requirements and persisted typed `Value` records, adapting
shared `CanonicalQuantity` results through `Value::from`. The units crate has no
dependency on the engine, project state, libraries or UI.

The standard registry supplies the meaning used by parsing, formatting and
canonical validation. Resistance, for example, has canonical unit `Ohm`, accepts
input aliases including `R` and `Ω`, and displays with `Ω`. Aliases never become
stored identifiers. A field's declared quantity determines validation; part names,
reference designators and display labels do not determine units. Hosts can
enumerate `standard_quantities()` or look up an exact canonical identifier with
`definition_for_unit()`.

`Quantity::parse` returns a `CanonicalQuantity` whose private strings have passed
common numeric/unit validation. Hosts can inspect its value/unit, format it or
consume its exact strings with `into_parts()`. The engine's
`library.parse_quantity` command keeps its tagged quantity response.
`library::parts::Quantity`, `decimal` and `display_quantity` remain re-exports
for existing imports; direct Rust callers adapt shared parsed values to native
`Value` records when that host representation is needed.

The [quantity contract](part-schema.md#canonical-quantities) remains in force:
atto–exa engineering prefixes, case-sensitive aliases, exact decimals, 36 digits,
18 fractional places and no display rounding. Category/schema names, canonical
units, project/baseline formats and no-op save bytes remain unchanged. Engine
tests check registry/schema agreement and native command responses together;
the shared crate tests parsing, display, aliases and precision independently.

The geometry API (`parse_length`, `Unit`, `edit_label`) still converts lengths to
integer nanometres with its own bounds and rounding. It shares the crate without
adopting the physical-quantity decimal storage policy.

The registry contains the existing built-in quantities. A new standard quantity
needs a stable meaning and canonical unit here, published schema updates and
compatibility review. Library-provided definitions would additionally need
versioned, pinned records so projects retain those meanings offline. This
extraction establishes shared ownership without adding those saved records.
