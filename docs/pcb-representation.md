# PCB representation and shared electrical rules

November 6, 2026. This records the agreed architectural direction and the proposed
contracts that still need implementation. It extends the [board workspace
plan](board-workspace-plan.md), [native storage contract](native-project-storage.md)
and [library proposal](library-sources.md) merged in #118. The implementation
status below distinguishes the first layer/stackup slice from later work; this
document does not claim a working board canvas, router or design-rule checker.

The model should support a small board soon while allowing richer constructions
without making names, array positions or today's UI controls into permanent
electrical identities. Start the foundations of [#76](https://github.com/tangency-labs/pcbcraft/issues/76)
and [#77](https://github.com/tangency-labs/pcbcraft/issues/77) before finishing
footprint placement in #54. Coordinate every numeric input with
[#75](https://github.com/tangency-labs/pcbcraft/issues/75).

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
and commands; the existing persistence module owns format versions and encoding.
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

Adding, removing or changing a layer is an atomic document edit. Before applying
it, the command must account for all objects using that layer, including pads,
via spans and later zones or graphics. An occupied layer requires explicit
reassignment or rejection; disabling its visibility is never removal. Changing
a layer's purpose under existing geometry requires the same validation.

## Physical stackup

Represent the stackup as an ordered construction of copper and dielectric
strata. Copper strata reference layer IDs. Dielectric strata have their own IDs,
optional material information and thickness. A stackup profile has a stable ID;
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
Board geometry must leave room for arbitrary footprint angles rather than
inheriting the schematic's quarter-turn-only command restriction.

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
material or thickness specifications. There is no setup dialog or board renderer
yet. General footprint layer bindings are also absent; the first slice must
conservatively reject topology changes on populated boards rather than infer a
reassignment from the hard-coded two-pad footprint.

Follow with shared unit parsing/numeric setup UI (#75/#76), the net-identity and
rule-resolution contract (#77 with #42), pinned footprints/assignments (#92),
then the board canvas and mixed workspace/cross-probing acceptance (#54/#64/#65).
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
finished-thickness semantics and the scope of per-layer pad stacks. These are
explicit follow-up decisions rather than promises made by the initial format.
