# PCB representation and shared electrical rules

November 6, 2026. This records the agreed architectural direction and the proposed
contracts that still need implementation. It extends the [board workspace
plan](board-workspace-plan.md), [native storage contract](native-project-storage.md)
and [library proposal](library-sources.md) merged in #118. The implementation
status below distinguishes the layer/stackup and material slices from later work; this
document distinguishes the current physical SMD/legacy marker canvas and single-footprint
transforms from general footprint editing, routing and design-rule checking.

The model should support a small board soon while allowing richer constructions
without making names, array positions or today's UI controls into permanent
electrical identities. Start the foundations of [#76](https://github.com/tangency-labs/pcbcraft/issues/76)
and [#77](https://github.com/tangency-labs/pcbcraft/issues/77) before finishing
footprint placement in #54. Coordinate every numeric input with
[#75](https://github.com/tangency-labs/pcbcraft/issues/75).

## Board addresses and multiboard migration

[#122](https://github.com/tangency-labs/pcbcraft/issues/122) requires physical
boards to have identities before additional board features depend on them. The
inspection canvas therefore resolves a `BoardTarget { project, board }` through
an immutable `BoardView`. Placements and cross-probe pad results include their
board address; project inspection lists available boards even when empty. The
existing `pcb.*` commands accept an optional `board` UUID, defaulting only under
the current single-board compatibility model. Unknown IDs fail before mutation.
New renderers consume the resolved board, not a global project placement list.

This is an address bridge, not the full #122 storage migration. The board remains
in `board.json`; layer-addressed Board Edge graphics use board document version 9 without adding
a persisted board list. The board UUID is derived deterministically from
SHA-256 of the UTF-8 bytes `pcbcraft:legacy-board:1` followed by a zero byte and
the saved project UUID's 16 bytes. The first 16 digest bytes use UUID v8 version
and standard variant bits. This keeps the address stable through rename, edits,
reopening and no-op saves. It is only the legacy-board identity rule; additional
boards need independently allocated, persisted IDs. The native format must retain
this compatibility ID when that board becomes an explicitly stored record.

The proposed next storage change, for public review before adoption, is an
ordered `boards` list alongside `sheets` in a new project-format version, with
stable ID, display name and document reference per board. Whether the first board
retains `board.json` or moves to `boards/<uuid>.json` remains a format decision.
The migration must preserve every placement, pin mapping, layer, material report
and stackup, and become byte-stable after the first migrated save. Readers that
cannot handle multiple boards must refuse them instead of truncating the list.

Component-to-board assignment must become an explicit project-model choice,
including assigned but unplaced components. The current legacy model still has
one implicit owner, and this slice does not change it. Each physical board owns
its outline, stackup/materials, placements, routing, local rule/output settings
and results. Shared electrical intent remains project-owned; connectors/cables,
a system netlist and assembly transforms are later contracts. Board duplication
for panelization and assembly variants remain separate concepts. More than one
board must make an omitted command target ambiguous rather than select the first
board silently. Board creation, rename/reorder/delete, ownership assignment and
per-board outputs remain open under #122 and its linked issues.

## Ownership

| Owner | Authored state | Derived or presentation state |
| --- | --- | --- |
| Shared project electrical model | Components, logical terminals, net identities and names, classifications and assignments | Connectivity membership and its revision |
| Project rule set | Stable rule IDs, typed constraints/preferences, selectors and explicit precedence | Resolved values, contributing rules and conflicts |
| Board | Layer identities, physical stackup, outline, placed footprints and later tracks/vias/zones; accepted schematic-update baseline | Physical connectivity, unrouted connections and rule results |
| Pinned library items | Independent symbol and footprint definitions; part/assignment definition linking them | Search indexes, thumbnails and external source availability |
| Editor view | No electrical document ownership | Camera, viewed side, active layer, visibility, theme colors, filters, edit selection and tool draft |

The schematic and board share logical electrical intent. Copper connectivity is
the physical implementation of that intent, so it must remain separately
inspectable for shorts and unrouted connections. The last accepted board-update
baseline also differs from current schematic intent while changes await review.
All views borrow the same session, revision, history and save controller.

These boundaries fit the existing engine, app, chrome and storage crates. They
do not require a new process or a document copy per workspace. A future library
editor's independently editable definition follows #118's separate document
save/history contract; merely viewing a pinned definition does not create one.

Keep code organized around these responsibilities as features arrive. The first
implementation's `engine::board::setup` module owns layer/stackup types, validation
and commands; `engine::board::materials` owns pinned material reports and
construction, with electrical reports in a separate submodule. The existing
persistence module owns format versions and encoding.
GUI and MCP callers use the command catalog instead of implementing their own
mutation rules. Shared net identity and rule resolution should become separate
engine modules, while footprint definitions remain separate from board instances.
This allows a new view or numeric control to reuse model behavior without moving
electrical state into a window or growing one general-purpose board controller.

## Layer identity, purpose and physical order

A layer has an opaque stable ID, an editable display name and a typed purpose.
Side is explicit where it applies. Board objects reference IDs; names and list
positions are never foreign keys. Purpose controls permitted uses, and a display
name such as `Ground` does not connect copper to the GND net. An assigned zone or
other conductive object establishes that electrical relationship.

The layer catalog includes copper and technical purposes such as mask, paste,
silkscreen, fabrication, courtyard, adhesive, outline and user documentation.
Copper order comes from the physical stackup. Catalog serialization uses stable
ID order, so renaming or inserting a layer does not renumber unrelated records.
Display order can differ by view without changing physical order.

The engine's immutable `CopperLayers` projection now resolves physical pad
membership into these copper UUIDs. Component-side surface lands use the current
front-side placement compatibility rule; plated lands resolve every copper stratum
in stack order. Without setup, the projection retains an explicit `unconfigured`
binding, and center-only legacy markers have no physical membership. GUI and MCP
consume this contract; renderers no longer infer surface membership from the
active layer. The personal palette is stored separately in local editor settings,
keyed by board/layer UUID, shared across views, and used for both swatches and
metal. Selecting a layer changes display priority for spanning pads without
moving or rebinding any metal. This slice adds no native-format migration, board
side flipping, tracks, vias or zones.

Adding, removing or changing a layer is an atomic document edit. Before applying
it, the command must account for all objects using that layer, including pads,
via spans and later zones or graphics. An occupied layer requires explicit
reassignment or rejection; disabling its visibility is never removal. Changing
a layer's purpose under existing geometry requires the same validation.

## Physical stackup

Represent the stackup as an ordered construction of copper and dielectric
strata. Copper strata reference layer IDs. Dielectric strata have their own IDs,
optional material information and thickness. Structured material reports and
core/prepreg construction now accompany the legacy descriptive material label.
A stackup profile has a stable ID;
the first board has one uniform rigid profile. Region assignments and additional
profiles are a later extension for flex/rigid-flex, not an inferred capability
of the initial model.

Thickness is an exact integer length when specified. An unspecified value stays
unknown rather than becoming zero or an invented material property. The sum of
known copper and dielectric thicknesses is the laminate thickness only when all
its constituents are known. Surface coatings, finished-board tolerances and
manufacturer specifications need explicit future fields; do not silently include
them in that sum or label the result a guaranteed finished thickness.

The first presets are two and four copper layers. Their thicknesses and materials
start unspecified. The representation is a bounded collection, not a fixed
two-layer enum or numeric layer bitmask. Supporting more layers must not require
changing the meaning of existing IDs. A through-via spans the first and last
copper layers; later blind/buried spans reference explicit copper endpoints.
Geometric validity of a span is separate from whether a selected manufacturing
process supports it.

Examples below use readable aliases for IDs, not literal storage identifiers.

| Two-layer stack, top to bottom | Four-layer stack, top to bottom |
| --- | --- |
| Copper `cu-top`, display name Top | Copper `cu-top`, display name Top |
| Dielectric `d-core` | Dielectric `d-outer-a` |
| Copper `cu-bottom`, display name Bottom | Copper `cu-inner-a`, display name Ground |
| | Dielectric `d-core` |
| | Copper `cu-inner-b`, display name Power |
| | Dielectric `d-outer-b` |
| | Copper `cu-bottom`, display name Bottom |

When extending a board, preserve its outer layer IDs and any surviving dielectric
or copper identities. Rename Ground to Reference without changing references.
Inserting a copper layer is an explicit topology change, with consequences for
pad stacks and vias reviewed before applying it to a populated board.

### Materials, core and prepreg

Core/prepreg construction and substrate properties belong in the foundation
before a material-aware setup UI or impedance calculation relies on it. The
legacy `material` string remains a human label with no inferred electrical or
thermal properties. The implemented [material contract](board-materials.md)
adds embedded definitions, sourced Tg and Dk/Df reports, core construction,
ordered prepreg plies and finished-gap tolerance through shared setup commands.
It supports user-defined families and products without a fixed FR-4 catalog.

The contract separates three responsibilities:

| Record | Responsibility |
| --- | --- |
| Pinned material definition | Stable identity/revision, material family or classification such as FR-4, optional manufacturer/product, source revision and reported properties |
| Dielectric construction | Core or prepreg role, referenced material definition, glass construction and resin content when specified, and ordered plies where needed |
| Board stackup instance | Stable gap identity and location between copper layers, chosen construction and specified finished dielectric thickness/tolerance |

FR-4 does not select a single product or assign a Tg. Material properties need
their own typed records: Tg with a temperature unit, measurement method and
source; Dk/Df with frequency and relevant test conditions; later thermal
expansion with direction and temperature interval. A reported typical value
must remain distinct from a guaranteed minimum or a design requirement. Isola's
[370HR datasheet, revision E](https://www.isola-group.com/wp-content/uploads/data-sheets/370hr-laminate-prepreg.pdf)
illustrates these distinctions by reporting a typical Tg of 180 °C by DSC,
frequency-specific Dk/Df values and separate thermal properties. These are
documentation examples, not bundled material defaults or a product qualification.

Keep properties optional and retain their provenance when projects reopen
offline. A family label must never manufacture missing values. A board-wide
requirement such as a minimum Tg belongs to the rule/requirements model and is
checked against applicable material records with compatible measurement methods;
it is not substituted for each selected material's reported value. Tg alone
must not be treated as a permitted operating temperature or a reflow approval.

The physical representation must accommodate a core construction and a bond
region with multiple prepreg plies without inserting fictitious copper layers.
For example, a proposed four-copper-layer construction could read
`copper / prepreg group / copper / core / copper / prepreg group / copper`, with
each prepreg group retaining its ordered plies. This is an example, not a required
four-layer recipe or an inferred manufacturing capability. Glass style and resin
content belong to the specified construction; Isola's [laminate and prepreg
manufacturing overview](https://www.isola-group.com/wp-content/uploads/Understanding-Laminate-Prepreg-Manufacturing.pdf)
shows why ply construction must remain distinguishable even at a given thickness.

Use finished dielectric separation for board geometry, while retaining raw or
nominal ply specifications separately. Do not add unpressed prepreg thicknesses
and claim that the sum predicts the finished gap. Keep copper thickness in its
copper records, so selecting a copper-clad core cannot count the foil twice.
Unknown construction or finished thickness stays unknown until specified; any
later estimate needs an explicit calculation and provenance. Lengths still obey
#75's canonical-nanometer contract, while temperature and dimensionless material
properties use their own units and validation.

The dedicated material module is consumed by stackup validation and setup
commands, and will supply the future property resolver. Board format 3 stores
these records, while unchanged version-2 projects preserve their IDs, labels
and thicknesses without guessing a product, Tg or core/prepreg role. Tests cover
mixed custom materials, unresolved properties, distinct report methods and
frequencies, invalid references/dimensions, explicit revision updates, offline
save/reopen and undo. Material requirements and derived-result invalidation
remain work for the future rule resolver and calculation layer.

### Stackup and impedance UI

The next user-facing workflow is **Board Setup → Stackup & Impedance**, accessible
from the project even before the layout canvas or router is complete. It should
combine an editable stack table with material selection, custom material entry
and impedance profiles. Altium's published [stack manager workflow](https://www.altium.com/documentation/altium-designer/pcb/defining-layer-stack)
is a behavior reference for keeping layer construction and impedance profiles
together; this does not adopt its implementation or promise solver parity.

A user chooses a starting stackup or enters the fabricator's construction, then
selects signal/reference layers, calculation geometry and frequency. A profile
can estimate impedance from a supplied width, or estimate width for a target
impedance. Differential profiles additionally need pair spacing: one of width
or spacing must be fixed before solving for the other. Store targets separately
from derived dimensions so later stackup changes can identify stale results.
No calculation should silently use a layer merely because it is named Ground.

Offer editable starting presets with explicit assumptions, including material
reports and dimensions; keep the current topology-only presets available when
those values are unknown. Preset values must have qualified provenance or be
clearly identified as user-editable engineering assumptions. A calculator needs
a supported model, relevant Dk and dielectric/copper geometry; Df is retained for
models that include loss. Mixed plies, anisotropy, mask, copper shape and roughness
must be handled explicitly where the chosen model requires them. Missing or
incompatible inputs produce a useful unresolved state, never a silent FR-4
constant or a nominal-ply thickness substituted for the finished gap.

The first calculator should expose only independently validated geometries and
show its model, assumptions and estimate status. Reference examples and tests
must cover valid ranges, dimensional units, inversion to width and rejection of
unsupported geometry. More capable solvers can then replace or supplement it
behind the same calculation interface. Altium [distinguishes its field solver
from closed-form calculators](https://www.altium.com/documentation/knowledge-base/altium-designer/validate-transmission-impedance-computed-in-layer-stack);
an initial estimate here must not claim equivalent accuracy.

After review, accepted width and pair spacing feed the shared net/class routing
rules. Minimum clearance and fabrication limits remain separate constraints:
an impedance calculation cannot override them. Material, thickness or profile
changes invalidate affected estimates and expose a reviewable rule update;
they do not silently resize existing tracks. The calculation service and the
apply-to-rules action belong in engine commands for GUI/MCP parity, with #75's
shared unit parser at the input boundary. These UI, calculation and rule steps
remain follow-up implementation work; the current slice supplies their inputs.

## Footprints, parts and pad stacks

Follow #118's proposed separation: a symbol describes terminals and schematic
graphics; a footprint describes pads and physical graphics; a part links pinned
definitions and supplies the terminal-to-pad mapping. A generic-symbol-first
workflow needs an equivalent project-local assignment, including mapping
validation, without forcing publication of a reusable library part.

Footprints should use relative layer purposes such as component-side copper and
opposite-side mask, or a supported copper span. Placement resolves those purposes
against the board and the footprint's side. A library footprint must not contain
another project's layer IDs. Pad geometry, holes and later per-layer pad shapes
belong to pad definitions; the board instance owns placement and orientation.
Board placement now stores counterclockwise whole degrees from 0 through 359,
independently of the schematic's quarter-turn restriction. The shared transform
command accepts those angles while GUI rotation steps by 90°. Quarter turns
resolve exactly and other angles round pad offsets to the nearest nanometer;
subdegree precision remains future work.

Looking at the underside changes a view transform. Flipping a footprint changes
the document and resolves its layer purposes again. Neither operation reverses
the authored physical stackup. General pad stacks and via manufacturing processes
remain later work, but their references must use the same layer catalog.

## Shared nets and cross-probing

Current connectivity consists of derived terminal groups without persistent net
IDs. A durable net identity contract must precede saved class/rule assignments.
The initial fixture can remain one sheet, but logical terminal addresses must
resolve through component identity and permit future unit/occurrence addressing.
Reference text, local label text and a net's array index cannot establish identity.

The net reconciliation design must specify these cases before its schema lands:

- A rename, placement move or geometry-only wire change preserves net identity.
- A split records which membership became each resulting net and what happens to
  explicit assignments. It cannot silently transfer an old rule to an arbitrary
  branch selected by traversal order.
- A merge reports conflicting identities, names and assignments. Any retained
  identity or alias/lineage is explicit and deterministic.
- Deleting a terminal or an entire net leaves the reviewed board baseline and
  pending removals intelligible until their consequences are accepted.
- Undo/redo and reopening restore authored identities and assignments, while a
  new opening invalidates old runtime requests even if saved IDs match.

The choice of anchors, survivor policy and lineage representation is still open.
Ordinary wire edits should not require a modal question at every split; a
reviewable unresolved assignment can be represented, but downstream operations
must not silently treat it as resolved. A separate implementation brief should
exercise a conflicting power/signal merge and a classified net split.

Cross-probing resolves schematic placement to component/terminal, then to the
assigned footprint/pad, in both directions. A received highlight never becomes
edit selection or emits a second selection event. Missing assignment, unplaced
footprint, incompatible mapping, pending removal and stale target are distinct
results. Net highlights carry a connectivity revision and expire after topology
changes. Selection and highlighting never apply the pending board update.

## Rules and net classes

Rules belong to the project and are available from either workspace. Board-only
constraints have explicit board scope rather than a second unrelated rule set.
Classes have stable IDs and editable names. Permit multiple classifications,
such as Power and High voltage, with explicit precedence; avoid baking a single
class field into every future object. Named net groups and differential pairs
should eventually reference net IDs rather than derive identity from suffixes.

Represent rules as versioned, typed records with an ID, scope/selector and
constraint or preferred value. Start with project/board defaults, classes,
individual nets and layers; leave room for regions and object-pair conditions.
The first resolver need not be a general scripting language. New selector and
constraint kinds need explicit supported versions and validation.

Separate manufacturing limits from design constraints and preferred routing
values. A preferred 0.3 mm width is not the same statement as a 0.3 mm minimum.
Define combination per property: minimum bounds can tighten, maximum bounds can
tighten, allowed sets can intersect, and contradictory bounds are a conflict.
Where an override is supported, its precedence and effect are explicit; file
ordering must not accidentally decide it. A routing preference cannot weaken
a hard manufacturing limit. Clearance between objects needs a symmetric
two-object resolution contract, including conflicts between their classes.

The resolver returns the effective values, contributing rule IDs, revisions and
any conflicts or unsupported requirements. The router, zone generator, DRC,
Properties inspector and MCP call this same resolver. An unresolved requirement
must not produce a claim that checking passed. Store authored intent; derived
results are revision-keyed caches outside tracked documents. Selecting a width
does not establish controlled impedance; that needs a qualified material/stackup
model and a separately implemented calculation.

### Project settings and fabrication profiles

Provide a project settings editor available from schematic and layout, with
Manufacturing, Stackup & Impedance and Electrical Rules pages. Manufacturing
settings describe a selected fabrication process; electrical rules describe the
design's requirements. Both are project data with shared engine validation,
undo and persistence, rather than application preferences or per-window state.

An editable fabrication profile should cover these initial groups:

| Group | Constraints |
| --- | --- |
| Copper | Minimum trace width and copper spacing, plus copper-to-board-edge clearance |
| Holes and vias | Minimum drill/finished-hole sizes with their meanings explicit, via land diameter, annular ring and hole-to-hole clearance |
| Via capabilities | Supported through, blind/buried and microvia processes, permitted layer spans, aspect-ratio limits and restrictions on stacked/staggered constructions when specified |
| Silkscreen | Minimum text height and stroke width, and clearance from exposed pads/mask openings |
| Stackup | Offered constructions, thickness ranges/tolerances and material selections where supplied |

Do not reduce microvia support to a single unchecked boolean: process capability
and valid span/dimensions must agree. Unknown capability remains unspecified,
not permission to route that feature. The first implementation may support a
subset of the table, but must reject unsupported required profile constraints.

Profiles should be shareable as a separate, portable, versioned file. The planned
contract uses a stable profile ID and revision, a name, optional fabricator and
process/service identification, provenance and typed constraints in canonical
units. Optional offered stackups reference embedded pinned material definitions;
the profile must be self-contained for those selections, without machine-local
paths, credentials, executable rules or required online lookups. The file
extension and exact schema will be fixed in that implementation slice, before
publishing examples. Users can author custom profiles; third-party profiles
require qualified provenance and redistribution terms before bundling.

Import validates the complete profile and presents its constraints before an
explicit apply. A project retains a pinned copy, selected offered stackup and
its own explicit refinements, so moving the portable file or going offline does
not change the design. A source path is a hint, not identity. Re-exporting the
profile should not include unrelated project circuitry. Taking a newer revision
is deliberate: compare changed limits, report conflicts with current rules and
geometry, and apply the accepted update in one undoable transaction. Never
silently mutate the shared profile file when editing a project's settings.

The shared resolver combines process limits with electrical requirements and
routing preferences. An impedance-derived width or pair gap must satisfy the
selected process limits; pair spacing is not a substitute for general clearance.
Keep unresolved/conflicting settings visible to users and MCP, with contributing
profile/rule revisions, and invalidate calculations on input changes. Project
settings, portable profile import/export, the resolver and DRC are planned work,
not capabilities implemented by the current material model.

## Units and numeric input — #75

Lengths remain signed integer nanometers internally, with field-specific bounds.
Positive thicknesses, nonnegative clearances and signed coordinates use the same
conversion contract but different validators. Display units never become the
stored unit of a layer, object or rule. Preserve 1 mil = 25,400 nm and
100 mil = 2.54 mm exactly; changing display preferences does not resnap geometry,
change connectivity, dirty the project or add a document undo step.

The existing unit-aware parser lives in chrome. Before exposing new setup/rule
numeric fields to both GUI and MCP, move or extract that parser into an
appropriate shared, UI-independent module. Do not copy its implementation into
the engine or make the engine depend on chrome. A typed suffix overrides the
display unit; a bare user-entered number needs an explicit default unit. Keep
rounding, representable precision and overflow errors identical across callers.
Canonical command payloads and persisted records can continue to use `_nm`
integers, which do not require a display preference.

Acceptance includes mixed-unit entry, negative coordinates, invalid suffixes,
values near precision/range limits and repeated unit switching. Compare stackup,
rule values, connectivity, revision, history and saved bytes, including no-op
saves. Parsing is not validation of manufacturing suitability.

## Storage evolution and commands

Extend the existing split documents with explicit versions. A board without setup
can retain its existing encoding; configuring one must use a format that older
readers reject rather than silently dropping layers. Keep each layer/stratum
independently diffable, stable IDs and deterministic serialization. Definitions,
rules and stackup must reopen with their source libraries unavailable.

Required unknown electrical features fail validation; they cannot be ignored on
save. A future optional metadata namespace needs its own preservation and size
contract. It is not a way to hide unsupported electrical semantics. Migration
must preserve legacy geometry and mappings without inventing physical values.

All edits use revision-checked engine commands and commit atomically after full
validation. No-op edits preserve revision and history. Inspection is available
to MCP through the same session. Headless command availability and the live
editor's explicit allowlist are separate; adding a catalog command alone does
not implement #61. No network requests belong in these model operations.

## First implementation and delivery order

The first slice implements a validated layer catalog and one rigid stackup,
initialization and replacement commands, project inspection, native persistence,
undo/redo and headless MCP discovery. Presets provide topology without claiming
material or thickness specifications. The board canvas now renders pinned
rounded SMD geometry alongside legacy pad-center markers and supports
single-footprint transforms and selection-based placement deletion, retaining
schematic components and physical package assignments. The Board setup dialog
now exposes layer names/optional membership, every even copper count from 2 through
64, finished thicknesses/tolerances, locked-gap fitting, core/prepreg construction,
and sourced Tg/Dk/Df material revisions. Initial setup on a populated board requires explicit
`resolve_existing_footprints: true`: component-side pads resolve to front copper,
all-copper pads span the stack, and marker-only pads remain markers. General
footprint bindings beyond front/all-copper remain deferred. Compatible populated
count changes require the same explicit consent, retain surface IDs, and reject
occupied drawing-layer removals; they are never inferred from footprint names.

The structured material/core/prepreg model and sourced electrical reports are
now implemented (#76). Follow with shared unit parsing and the stackup UI
(#75/#76), project manufacturing settings and portable fabrication profiles,
a bounded impedance estimator, and the net-identity/rule-resolution contract
(#77 with #42) before applying estimates to routing rules. The bounded
[pinned-footprint slice](footprint-library.md) now covers starter physical
definitions and mappings; general assignment UI (#92) and the remaining board
canvas/mixed-workspace acceptance (#54/#64/#65) are still open.
Review the accepted-board baseline and pending removals under #55 before exposing
schematic deletion in that board workflow. Live editor command parity remains
#61. These are bounded slices; none closes the broader issue by itself.

Tests for the first slice cover stable identities across renaming, a two-to-four
layer transition on an empty board, unsupported/removal cases on populated
boards, unknown thickness, invalid references/dimensions, stale commands,
atomic failure, undo/redo, legacy compatibility and deterministic offline
save/reopen. The later rules slice adds class conflicts, net splits/merges and
resolver consistency between consumers. UI and native-window acceptance belongs
to the slices that expose these features visually.

## References and remaining decisions

Behavior references are the published KiCad 9.0 [PCB Editor manual](https://docs.kicad.org/9.0/en/pcbnew/pcbnew.html)
for editor layers, physical stackup and design constraints, and the [Schematic
Editor manual](https://docs.kicad.org/9.0/en/eeschema/eeschema.html) for shared and
multiple net classes. This is a new proposal over approved upstream code; no
KiCad implementation, prototype implementation or third-party library assets
are inputs to this slice.

Before their implementation, settle net split/merge identity, rule precedence
and exceptions, accepted-board baseline storage, material-property provenance,
core/prepreg construction and ply schema, finished-thickness/tolerance semantics
and the scope of per-layer pad stacks. These are
explicit follow-up decisions rather than promises made by the initial format.
