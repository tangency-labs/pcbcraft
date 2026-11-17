# Board materials and dielectric construction

The engine stores project-local material definitions and their use in a uniform
rigid stackup. `pcb.set_board_setup` validates and adopts the complete candidate
with the caller's `expected_revision`, so construction, material references and
dimensions change in one undo step. The command is shared by GUI and MCP.
**Layout → Board → Board setup** or **Layers → Board setup…** opens the Craft settings
form with Board editor layers, Physical stackup, Materials, Silkscreen and Profiles pages.

Copper count accepts every even number from 2 through 64, including initial setup
and later count changes. Surface and surviving inner-layer identities remain stable.
Populated-board changes require explicit front/all-copper binding consent; the GUI
supplies it when Apply is chosen after showing the changed pad-span behavior.
Occupied drawing layers cannot be removed. Reductions that would discard authored
thickness/construction data are refused until those fields are cleared deliberately.
Unused optional technical layers can be removed or added, while this editor retains
copper, Board Outline and courtyard layers. Per-view visibility remains separate.

The first setup draft proposes a two-layer, 1.6 mm FR-4 board with nominal
1 oz (35 µm) copper on both faces and a 1.53 mm dielectric core. These are editable
starting choices, adopted only on Apply. Existing configured boards retain their own
values. The nominal 1 oz/35 µm convention is also used in Rogers' published
[RO4003C material information](https://www.rogerscorp.com/advanced-electronics-solutions/ro4000-series-laminates/ro4003c-laminates);
this preset is original PCBcraft configuration, without a supplier qualification.
The two-layer chooser also offers 2 oz/70 µm FR-4 and a generic high-Tg FR-4 family.
Changing copper count retains existing dimensions; newly added gaps remain unknown.

The Physical stackup page shows an illustrative laminate cross-section aligned with
the table rows, finished copper/dielectric thicknesses,
and their laminate total. Blank values stay unknown. **Fit unlocked dielectrics**
distributes the exact residual thickness after copper and locked dielectric gaps;
all fixed thicknesses must be specified and the resulting gaps must remain positive.
Each dielectric supports tolerance, a descriptive label, core or ordered prepreg
construction, pinned material selection, and ply glass/resin/nominal stock fields.
Stock ply thickness is never counted again as finished laminate thickness.

Materials supports project-local specifications with sourced Tg and Dk/Df reports,
including Tg method, basis, frequency, test method and specimen conditions. Existing
pins are immutable: **Technical details → Add specification…** retains the old reports
and creates a distinct revision; choosing that revision for a dielectric is explicit.
Saving a material only stages it in the form. Apply commits the entire board draft
and Cancel discards it.
No electrical value, supplier property or manufacturing qualification is inferred.
Impedance calculations and manufacturing-profile/rule editors remain follow-up work.
Portable manufacturer/process profiles are a separate planned resource; see
[project settings and fabrication profiles](pcb-representation.md#project-settings-and-fabrication-profiles)
for their import, pinning and update contract.

## Material identity and reports

`setup.materials` contains up to 64 embedded definitions, addressed by UUID `id`
and positive `u32` `revision`. A name is required; family, manufacturer, product
and source are optional. Family is user text, not an FR-4 enum or an approved
material list. Custom materials and different materials within one board are
supported. The Materials page offers generic FR-4, high-Tg FR-4, polyimide laminate and PTFE
laminate templates alongside custom materials. Two-layer choices apply to the core; on multilayer
boards they add a definition for explicit per-gap selection. These templates
have stable built-in identities and immutable revisions and no manufacturer, Tg or Dk/Df reports.
Technical properties and measurement conditions expand separately from basic material
identity. No supplier product data is bundled, and a family label supplies no physical
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

### Thermal and insulation reports

Four further optional report lists sit beside `tg` and `dielectric`. Each report needs a
basis (`typical`, `minimum`, `maximum` or `measured`) and a source, and may name its method
(up to 80 characters). The unit is part of the value's field name; a report with a bare
value, a separate unit or no source is refused. Up to 16 reports of each kind are retained
separately and in canonical order, and are never averaged or substituted for one another.
A missing report or condition means unknown, never zero.

| List | Value field and unit | Conditions |
| --- | --- | --- |
| `thermal_conductivity` | `conductivity_mw_per_m_k`, mW/(m·K), 1–10,000,000 | optional `axis`, `temperature_milli_c` |
| `cte` | `cte_ppb_per_k`, ppb/K (10⁻⁹ per kelvin), −1,000,000–10,000,000 | optional `axis`, `region` (`below_tg` or `above_tg`), and the temperature range `from_milli_c`/`to_milli_c`, which come together and rise |
| `dielectric_strength` | `breakdown_v`, V, 1–10,000,000 | required `specimen_thickness_nm`, 1–100,000,000; optional `axis` |
| `cti` | `cti_v`, V, 1–1000, and/or `material_group` (`I`, `II`, `IIIa`, `IIIb`) | at least one of the two |

The numeric fields in these four lists require parsed JSON integer-kind tokens:
`1` is accepted within the field's range, while `1.0`, `1e0`, `-0`, strings and
fractional numbers are refused. This token-kind restriction is stricter than the
mathematical-integer meaning of JSON Schema's `integer`. The existing Tg and Dk/Df
field readers are unchanged.

For example, 1.0 W/(m·K) through the thickness is `conductivity_mw_per_m_k: 1000` with
`axis: z`, 50 ppm/K is `cte_ppb_per_k: 50000`, and 40 kV across a 1.0 mm specimen is
`breakdown_v: 40000` with `specimen_thickness_nm: 1000000`. Breakdown voltage is kept with
its specimen thickness instead of being reduced to a field strength, because the two do
not scale linearly.

The CTI material groups follow IEC 60664-1 clause 4.8.1.3: group I from 600 V, II from
400 V up to below 600 V, IIIa from 175 V up to below 400 V and IIIb from 100 V up to
below 175 V. A report that gives both a CTI value and a group must agree with these
bands; a group alone records a datasheet that states only the group. These are material
reports, not creepage or clearance requirements, which belong to the rules model.

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

## Copper strata and board thickness

A copper stratum's `thickness_nm` is the finished conductor thickness, including
any plating on outer layers. Two optional qualifiers sit beside it and both require
it: `thickness_tolerance_nm`, a symmetric tolerance with the same bounds as the
dielectric tolerance, and `thickness_basis`, which states whether the value is
`nominal` or a guaranteed `minimum`. Without a basis the value's basis is unknown.
Finished copper on inner layers can be thinner than the foil, so no relation
between the two is enforced.

Optional `role` is `signal`, `plane` or `mixed`. It describes how the layer is
used and does not assign nets. Optional `base_copper` describes the foil as ordered:
`weight_milli_oz`, the nominal weight in thousandths of an ounce per square foot
(1000 is 1 oz, accepted from 1 to 100 000), and/or the foil `thickness_nm`. At least
one is required. Neither is derived from the other, and neither changes the finished
thickness.

Base copper weight and foil thickness, copper tolerance, and the three board-thickness
fields require parsed integer-kind numbers: decimal/exponent spellings (including `-0`)
are refused even when JSON Schema considers their values integral, and the existing
JSON parser does not promise lossless decimal precision.

The stackup may declare the finished board thickness: `finished_thickness_nm`
(nominal), a symmetric `finished_thickness_tolerance_nm`, which requires the
nominal, and `declared_minimum_thickness_nm`, the lowest thickness the board is
stated to have.
Each thickness is a positive whole number of nanometers up to 100 mm, and a tolerance
keeps the lower bound positive and the upper bound within 100 mm. The minimum may not
exceed the nominal value. These are declarations: they cover plating, coatings and
fabrication variation and are never computed from, or checked against, the laminate
sum. Checks that need one board thickness take the declared
minimum when given, otherwise the nominal value, and report which one they used.

Every field is optional and omitted means unknown; zero is never a placeholder.
Engine code reads them through `BoardSetup::copper`, `copper_stack` and
`board_thickness_for_checks`; MCP clients read them from the inspected setup and
change them with `pcb.set_board_setup`, as one undo step. A copper-count reduction
refuses to drop a copper layer that carries any of them. The two-layer starting
stackups replace copper thickness, so they clear its tolerance, basis and base copper
and keep roles and the board thickness. The Board setup form keeps these fields
when it applies other edits but does not edit them yet.

## Persistence and limits

Material-aware fields use `pcbcraft-board/3`. A project using only the original
layer catalog, labels and thicknesses retains version 2 byte-for-byte on a no-op
round trip. Opening version 2 does not infer materials, reports or construction.
Version 3 embeds all selected definitions in `board.json`; it needs no source
library to reopen. Each material revision and each dielectric gap has a stable
record line, with ordered plies nested within their gap. Unknown versions and
fields, unresolved pins and duplicate JSON keys fail before project adoption.

Copper roles, base copper, copper thickness qualifiers and finished board
thickness need at least `pcbcraft-board/23` when one of them is present. Thermal
conductivity, thermal expansion, dielectric strength and CTI reports use
`pcbcraft-board/24`, chosen only when one of them is present; it may also carry
every format 23 field. Older headers refuse these reports, and a board without
them keeps its earlier format and bytes.

Version 3 does not introduce a reusable material library,
manufacturing qualification, rule requirements, flex regions or an impedance
solver. Extending those capabilities will use typed records and explicit format
versions rather than an unchecked property bag.

## Silkscreen and portable setup profiles

Silkscreen separates text/graphic dimensions from copper-pad clearance and previews
the actual PCBcraft Technical stroke lettering and conservative pad clipping. The
three current settings govern board-generated reference text and outlines; they are
not per-object text defaults or a general DRC rule set. The organization was compared
with the published [KiCad text/graphics defaults](https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html#configuring_text_and_graphics)
and Altium's [silkscreen preparation](https://www.altium.com/documentation/altium-designer/pcb/preparing-silkscreen)
and [manufacturing rules](https://www.altium.com/documentation/altium-designer/pcb/design-rule-types/manufacturing).
KiCad distinguishes layer defaults from object properties, and Altium distinguishes
clearance checks and preparation actions. PCBcraft currently clips against copper-pad
bounds; mask-opening, silk-to-silk and board-edge DRC remain outside this slice.

Profiles imports/exports a named `.pcsetup.json` file with format
`pcbcraft-setup-profile/1`, `/2` when the setup uses the copper and board
thickness fields above, or `/3` when a material carries a thermal conductivity,
thermal expansion, dielectric strength or CTI report, an independent UUID and positive revision, a complete
validated `setup`, and physical `silkscreen` settings. Lengths are integer nanometers;
material definitions, reports and references are embedded. The bounded, strict decoder
rejects unknown fields, unsupported versions, invalid pins and files over 1 MiB.
These setup snapshots carry no required electrical/fabrication constraints or project
paths and do not establish a live link to their source file. Every export creates a
new snapshot identity. Broader fabrication/process profiles remain planned.

Import stages a preview. Layer IDs are rebased to the target's physical stack and
unambiguous technical roles while extra target layers survive, preserving drawing
ownership. Existing immutable material pins cannot be replaced by contradictory data.
Apply publishes stackup/materials/silkscreen through the shared revision-checked command,
with one Undo; Cancel leaves the project unchanged. Export captures the current draft
without saving or dirtying the project. Desktop publication uses an atomic new file
and refuses existing destinations; browser export starts a download.

One application-owned file job remains bounded after a form closes. Tickets and
captured revision checks prevent its result from reaching another form or a replaced
project. Picker and filesystem work run away from the editor loop; disconnected
workers release their slot with an error, and shutdown does not join a stalled worker.
Atomic publication prevents a partially written visible profile. An export interrupted
by application exit is not reported as successful.
