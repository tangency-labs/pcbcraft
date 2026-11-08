# Board materials and dielectric construction

The engine stores project-local material definitions and their use in a uniform
rigid stackup. `pcb.set_board_setup` validates and adopts the complete candidate
with the caller's `expected_revision`, so construction, material references and
dimensions change in one undo step. The command is shared by GUI and MCP. In PCB layout, **Layout → Board → Setup**
or **Layers → Board setup…** opens the graphical form. It creates an initial
two- or four-copper-layer setup, edits names and finished thicknesses, and adds
technical layers before footprints are placed. Initial setup also supports populated
boards: its explicit `resolve_existing_footprints` command flag resolves component-side
pads to front copper and all-copper pads across the stack, retaining marker-only pads,
placements and assignments. Blank thicknesses remain unknown.
The Stackup page edits descriptive dielectric material labels while retaining
existing material pins, reports, construction and tolerances. Material/report and
construction editors, copper-topology migration and impedance calculation remain
follow-up work in the [representation plan](pcb-representation.md#stackup-and-impedance-ui).
Portable manufacturer/process profiles are a separate planned resource; see
[project settings and fabrication profiles](pcb-representation.md#project-settings-and-fabrication-profiles)
for their import, pinning and update contract.

## Material identity and reports

`setup.materials` contains up to 64 embedded definitions, addressed by UUID `id`
and positive `u32` `revision`. A name is required; family, manufacturer, product
and source are optional. Family is user text, not an FR-4 enum or an approved
material list. Custom materials and different materials within one board are
supported. No product data is bundled, and a family label supplies no physical
properties or manufacturing qualification. Naming a material metal-core or flex
does not add the physical constructions those boards need.

Several revisions of a material can coexist. An edit cannot change the content
of a pin retained from the current setup: add a new revision and explicitly
update the desired core/ply references. References to other revisions stay put.
Unused definitions may be removed; a referenced definition cannot. This is
project-local snapshot validation, not a global historical material registry.
Names do not resolve references, even when they match.

Each source has a required `document` description, optional source `revision`
and optional `locator`. These fields survive offline saves; locators are never
fetched. A source documents the author's provenance and does not certify a
measurement. Names/classification text are at most 80 characters; source
documents allow 200 and locators 1024. Text is nonblank, unpadded and control-free.

Optional `tg` reports retain integer `temperature_milli_c`, method (`dsc`, `tma`
or `dma`), basis (`typical`, `minimum`, `maximum` or `measured`), source and optional
`test_standard`. Up to eight reports are retained separately; they are not
averaged or substituted for one another. Temperatures must be above absolute
zero and at most 1000 °C. Missing reports mean unknown, whereas zero is a value.
A design requirement such as minimum Tg will belong to the shared rules model;
it is not a material report or an operating-temperature rating.

Optional `dielectric` reports retain Dk and/or Df, a required basis and source,
and any known frequency, method and specimen conditions. Up to 32 reports per
material can coexist. The supported fields are:

| Field | Meaning and supported range |
| --- | --- |
| `dk_millionths` | Dimensionless relative permittivity, 1,000,000–1,000,000,000 (Dk 1–1000) |
| `df_millionths` | Dimensionless loss tangent, 0–1,000,000 (Df 0–1) |
| `frequency_hz` | Integer frequency, 1–1,000,000,000,000,000 Hz |
| `method` | Authored test/calculation method, up to 80 characters |
| `value_kind` | `test` or `design`, when the source makes that distinction |
| `temperature_milli_c` | Specimen temperature, using the same range as Tg |
| `axis` | Specimen `x`, `y` or `z`; omission does not mean isotropic |
| `glass_construction` | Specimen construction description, up to 80 characters |
| `resin_content_ppm` | Specimen resin fraction, 1–999,999 millionths |
| `specimen_thickness_nm` | Specimen thickness, 1–100,000,000 nm |

At least one of Dk or Df must be present. Other conditions may remain unknown;
omission never makes a report universally applicable. For example, `dk_millionths:
4200000` means 4.2 and `df_millionths: 15000` means 0.015. These ranges are current
representation limits, not a statement that every accepted value describes a
manufacturable PCB. Unknown fields, explicit nulls and duplicate reports fail
validation. Catalog and report order are canonicalized for deterministic saves.

Isola's published [I-Speed datasheet](https://www.isola-group.com/wp-content/uploads/data-sheets/i-speed-laminate-and-prepreg.pdf)
reports Dk/Df at several frequencies and identifies measurement methods. This
supports retaining the conditions alongside each value instead of storing one
unqualified constant. The tests use original synthetic data, not that product's
values. The model does not select reports, interpolate frequencies, combine
different plies into an effective Dk or calculate impedance. Those operations
need an explicit model and applicability checks.

## Construction and dimensions

Each dielectric gap retains its existing stable UUID and optional descriptive
`material` label. Its optional `construction` is either `core`, with optional
exact material reference and `glass_construction`, or `prepreg`, with 1–32 ordered
plies. Each ply has a stable UUID, optional material reference, `glass_style`,
`resin_content_ppm` and `nominal_thickness_nm`. Construction can be known before
the product is selected, and different plies may reference different materials.
Ply IDs are unique across all layers, gaps, plies and the stackup profile. The
whole setup permits at most 256 plies. Core glass construction remains a
description rather than an internal glass-ply recipe.

The gap's `thickness_nm` remains the specified finished dielectric separation.
An optional `thickness_tolerance_nm` is a nonnegative symmetric tolerance that
requires a specified thickness; its lower bound must remain positive and its
upper bound at most 100 mm. Copper thickness remains in copper strata, so a
core's foil is not counted twice. Nominal stock thickness belongs to each ply
and does not enter the laminate-thickness sum. Isola's [prepreg selection note](https://www.isola-group.com/wp-content/uploads/Prepreg-Selection-Individulal-Dielectric-Space.pdf)
describes how copper fill and resin flow affect the finished gap; adding stock
thicknesses is not a finished-thickness calculation.

Lengths use exact integer nanometers under #75. Resin fractions, dimensionless
dielectric values and temperatures have their own explicit units; changing the
display between mm and mil never rescales them. Construction edits advance the
shared document revision but preserve net identity. A future derived-result
cache must invalidate on changed material, construction or geometry inputs.

## Persistence and limits

Material-aware fields use `pcbcraft-board/3`. A project using only the original
layer catalog, labels and thicknesses retains version 2 byte-for-byte on a no-op
round trip. Opening version 2 does not infer materials, reports or construction.
Version 3 embeds all selected definitions in `board.json`; it needs no source
library to reopen. Each material revision and each dielectric gap has a stable
record line, with ordered plies nested within their gap. Unknown versions and
fields, unresolved pins and duplicate JSON keys fail before project adoption.

Version 3 does not introduce a reusable material library, thermal expansion,
manufacturing qualification, rule requirements, flex regions or an impedance
solver. Extending those capabilities will use typed records and explicit format
versions rather than an unchecked property bag.
