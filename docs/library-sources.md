# Library sources and the Parts Catalog

November 6, 2026, revised November 11, 2026. This document defines the library
model behind [#28](https://github.com/tangency-labs/pcbcraft/issues/28), incorporating
the discussion in [#217](https://github.com/tangency-labs/pcbcraft/issues/217).
The [symbol](symbol-schema.md) and [part/category](part-schema.md) schemas and
native codecs implement the definition contracts. Native part placement, typed
instance requirements, manufacturer records and offline pinning are implemented
as described in [part-schema.md](part-schema.md). The remaining manager, provider
and import workflows below describe intended behavior.

The Parts Catalog is meant to become the one place to find symbols, footprints
and parts, from bundled, local and remote sources, including KiCad libraries and
vendor downloads. This document defines the library model behind it, so later
slices build on the same terms. It also defines what the library manager does
and how projects store 3D models. Supplier data, model rendering and the
manager's implementation details each get their own design (see
[Related designs](#related-designs)).

## What exists today

- **Bundled native libraries.** Basic components (`libraries/basic-components`) and Power
  (`libraries/power`) are embedded at build time (see
  [symbol-library.md](symbol-library.md)). Basic components also holds a small set of
  original [SMD footprint definitions](footprint-library.md): 0402/0603
  resistors and capacitors, an 0402 LED, SOT-23 and a plated two-pin header.
- **Exact copies pinned into projects.** A project keeps a full copy of every
  symbol and footprint it uses, so it opens without the library (see
  [native-project-storage.md](native-project-storage.md)). Opening does not adopt
  newer source content. It can normalize recognized, unchanged historical
  `starter` pins to `basic-components` in memory; ordinary Save publishes those
  addresses. Customized pins and conflicting destinations keep their original
  identities. See the [bundled address migration](native-project-storage.md#bundled-library-address-migration).
- **Explicit package assignment.** A shared schematic/PCB chooser assigns an
  exact footprint and a validated terminal-to-pad mapping to a part instance, then
  places it on the board as a separate step.
- **Personal native sources.** The Library workspace, folder registration and bundle installation are
  described in [native-library-sources.md](native-library-sources.md). Conflicting
  content claiming the same identity and revision is reported, not chosen.
- **Parts Catalog.** Search over bundled symbols and loaded native parts/symbols, in an
  expanded chooser and a compact dockable view that share one placement path.

## Goals

- **Two workflows.** Support part-first design: search, compare and place an exact
  manufacturer part with its symbol, footprint and pin map already resolved.
  Also support generic-first design: place a generic symbol now, choose the
  package before layout, and leave the MPN out where the project allows it.
- **Parametric search.** Make parts searchable and comparable by typed category
  parameters, so finding an alternate is a filter rather than a hunt.
- **Unit-aware automation.** Validate physical values against their declared
  quantities, normalize equivalent input forms, and export typed requirements
  that inventory systems can use even when a part instance has no chosen MPN.
- **Project rules.** Let each project decide what must be filled in at each stage,
  without weakening the structural checks every project needs.
- **Existing libraries.** Read KiCad libraries and vendor downloads without a
  separate plugin (VISION.md names this as the most common request).
- **Offline and traceable.** Keep every project openable and editable offline from
  its pinned copies, with the source, revision and license of every definition.
- **One command set.** Library actions go through shared commands, so MCP can
  search, place and pin exactly as the GUI does.
- **Git-friendly files.** Stable identities, no timestamps or caches in saved
  files.

## The records

The model separates five library record kinds from the placed instance:

| Record | Holds | Never holds |
| --- | --- | --- |
| **Symbol** | Schematic drawing, terminals with stable IDs, units, and an optional explicit default category/primary-value binding | Footprints, pads, MPNs |
| **Footprint** | Pads, outline, courtyard, silkscreen, paste and mask | Symbols, terminals, MPNs |
| **Part definition** (generic) | An optional symbol, an optional category with typed parameters and a primary-value field, and zero or more package variants. Each variant is an exact footprint revision; terminal-to-pad mapping applies when a symbol is present. Symbol-only, footprint-only and metadata-only definitions are valid library records. | Manufacturer identity, drawings or pads of its own |
| **Manufacturer part** | Manufacturer, exact MPN, the one part and package variant it is, its specified parameters and a datasheet link | Drawings, pads, supplier prices or stock |
| **Category schema** | Field IDs, value types and units for a kind of part, such as a capacitor | Values |
| **Part instance** | One use in a project, such as R1: its reference, its chosen part, package variant and manufacturer part (each possibly unset), and its own parameter values | Library definitions; it points at pinned copies |

Symbols and footprints are versioned independently. One footprint can serve many
parts, and a team can share a footprint-only library. The part is the only place
a symbol and a footprint meet, so the pin map and its validation live there.
Many manufacturer parts can point at one part. That is how "one generic 0603
resistor, many MPNs" works without copying drawings, and without part
inheritance.

The engine type is `PartInstance`; reusable records use `PartDefinition`.
Manufacturer parts identify orderable products and remain separate from both.
Existing saved `components` collections and component-addressed command keys
retain their compatibility spelling and refer only to placed part instances;
new library records use exact library/item/revision references.

### Pin maps

When a symbol is present, a package variant maps symbol terminals to footprint pads. Without a symbol, its mapping is empty and it asserts no electrical connections. Real packages need
more than one pad per terminal: a QFN's exposed pad tied to GND, several GND pins
on one terminal, or shield and mounting pads. So the mapping is:

- **Terminal to pads:** each terminal maps to one or more pads.
- **Pads:** each pad belongs to at most one terminal, or is explicitly
  not connected.
- **Completeness:** every terminal is mapped. A pad that is missing from the
  mapping is an error, never silently left out.

Native part definitions and project assignments support multiple pads per
terminal, including explicitly unconnected pads. The board assignment codec
and pinned part-instance storage validate complete coverage and exclusive pad
ownership; positions and numbering never establish a mapping implicitly.
Choosing a package validates the selected variant.

### Values on the instance

In the generic-first workflow you place a resistor and type 10k. That value
belongs to the instance. Native part instances store typed requirements, while
symbol-only instances retain a free-text value and metadata fields.

Instance values and manufacturer values mean different things:

- **Requirements, on the instance.** What the design needs. Each is typed by its
  category field and has a form: exact (10 kΩ), at least (rated voltage ≥ 25 V),
  at most (tolerance ≤ 1 %), a range, or a set of allowed choices (C0G or X7R).
  Typing "10k" sets an exact resistance requirement.
- **Specifications, on the manufacturer part.** What the product is rated for:
  10 kΩ, 50 V, ±1 %, X7R.

The part instance's effective requirements include the values set on the instance
and any values fixed by its generic part. A fixed value is an exact requirement
even when the instance leaves that field unset, and the instance cannot override
it. A field with neither kind of value leaves the choice open.

A manufacturer part **meets** R1 when its specifications satisfy all those
requirements after unit conversion. A 50 V rating meets "at least 25 V". An
unknown specification is reported as unknown and does not satisfy a requirement.

These rules drive the rest of the generic workflow:

- **MPN search:** R1's requirements filter manufacturer-part candidates.
- **Choosing an MPN:** a candidate that doesn't meet R1 is refused, and the
  refusal names the failing fields.
- **Changing a value, part or package after an MPN is chosen:** the edit is
  allowed, and R1's manufacturer part is re-checked in the same command. A
  manufacturer part names one part and one package variant, so choosing a
  different part or package variant for R1 also re-checks it, even when the new
  pin map is valid. If it no longer meets R1's requirements or no longer matches
  R1's part and package variant, it stays recorded but is marked **does not meet
  requirements** in inspection and BOM review. It is never silently kept as if
  it were still valid.
- **Fixing a mismatch:** you either choose another MPN, clear it, or relax the
  requirement.
- **Assembly BOM:** refuses a part instance whose MPN doesn't meet its requirements
  or doesn't match its part and package variant, whatever the project's
  completeness profile says.
- **BOM review:** shows which part instances still need a package or an MPN, and
  which chosen MPNs no longer meet their requirements.

### What stays out of v1

These ideas came up in review. They are deferred because nothing in v1 needs
them, and the records above leave room to add them later.

- **A separate logical record.** This would split terminals and units out of the
  symbol. Stable terminal IDs already give pin maps and connectivity what they
  need. A separate record becomes useful once one symbol needs several
  drawings, or pin and unit swapping is supported. Stable terminal IDs make a
  later split possible but don't make it automatic. Adopting it needs its own
  compatibility review of the codec, existing pinned symbols, connections and
  no-op saves. Until then, simple parts never show the user a "logical" layer.
- **Part inheritance.** This deferral is a proposal for the #217 participants to
  review, since bherbruck asked for "one base passive, many MPN variants".
  Many manufacturer parts sharing one part covers that case for manufacturer
  variants. What it doesn't cover is a family of generic parts that differ only
  in fixed values or package variants. Inheriting one part from another needs
  cycle checks, typed overrides and pinned bases, so the proposal is to wait for
  a concrete family that needs it. If participants want it in v1, it would be
  single inheritance from one exact, pinned base.
- **Library item status.** Draft, approved and obsolete states would let a project
  require approved parts only. They fit the completeness rules below, but need
  their own discussion of who approves.

## The two workflows

- **Part first (the main path).** Search, compare, pick a manufacturer part and
  place it. Its part, package variant, symbol, footprint and pin map are pinned
  together in one undoable command. A supplier result with no symbol or
  footprint yet can be searched and compared but not placed. Resolving it either
  produces a complete part or reports what's missing, without assigning anything
  partway.
- **Generic first (the fallback).** Place a part's symbol, set values on the
  instance, then choose a package variant before the part instance goes to the
  board. Choosing a manufacturer part is optional unless the project requires it.
- **From a symbol-only library.** Placing a plain symbol needs no library
  authoring first. The placement pins the symbol and creates a project-local
  generic part that references it, with no category and no package variants yet.
  - **Category:** stays unset unless the source explicitly declares one, together
    with any primary-value field binding. Nothing is guessed from the symbol's
    name or drawing.
  - **Package:** assigning a footprint later validates a pin map and adds that
    package variant to the project-local part, as one undoable command. This is
    today's assignment flow, recorded as a part.
  - **Reuse:** you can save the project-local part to a library afterwards. Until
    then it lives only in the project, and other placements of the same symbol in
    that project can reuse it.

## Categories and parameters

The bundled Basic components library defines standard categories with stable field IDs, value
types and units: capacitance, tolerance and rated voltage for capacitors, for
example. Search and comparison use field IDs and convert compatible units. They
never match on display labels.

Other libraries can reuse the standard categories, add namespaced fields, or add
categories of their own. An extension can't redefine a standard field's type or
meaning. Two fields with the same label but different IDs stay distinct unless an
explicit mapping says otherwise.

A value can be known, unknown or not supported by its source, and those are three
different states. Unknown is never zero or a guessed default. Projects pin the
category schemas they use, so a newer bundled library can't reinterpret saved
values.

### Unit-aware input and stored values

The [shared quantity/unit model](quantity-units.md) owns standard unit meanings,
aliases and exact normalization. Category fields reference its quantity identities;
the part model adapts canonical results to its existing typed value records.

A categorized part declares which field its ordinary Value control edits, such
as resistance for a resistor. That field's schema defines its physical quantity
and canonical unit. A symbol may declare an explicit default category and
primary-value field for the project-local part created on placement. An explicit
part binding determines validation; display names never determine units.

For a resistance field, the supported input grammar should accept `100m` and
`0.1R` as equivalent forms of 0.1 Ω, with the implied unit supplied by the field.
Prefixes are case-sensitive. A resistance entry containing a voltage or an
arbitrary string such as an MPN is rejected with a field-specific error, without
overwriting the previous valid value. Unset remains distinct from invalid input
and from zero. GUI and MCP use the same parsing and validation rules.

Store and compare the normalized quantity, not its display spelling, using a
representation that preserves the accepted precision. Formatting may display
`100 mΩ` or `0.1 Ω` without changing the meaning. Generic part instances can carry
additional typed requirements, such as tolerance or minimum power rating, without
selecting an MPN or putting the requirement in a comment. The implemented parser syntax, canonical decimal representation and numeric
bounds are specified in [part-schema.md](part-schema.md#canonical-quantities),
with shared engine normalization and regression cases. The part properties form
edits typed instance requirements and their operators through
`part.set_selection`. Symbol-only Value editing retains its existing free text.

An uncategorized imported symbol may retain its existing free-text value during
editing. It is explicitly untyped, so automation cannot treat it as a validated
resistance or infer requirements from it. Assigning a category requires a
reviewed field binding and value validation. A project profile can require typed
values before export while a looser project remains editable without them.

## Project completeness rules

Each project picks which information is required at each stage. A team can share
a versioned rules profile, for example "MPN required for every fitted part",
while another project allows generic parts. Projects pin the profile they adopt,
and changing it is a deliberate edit. No profile stops you from opening, editing
or saving unfinished work.

| Stage | Always required | Required if the project says so |
| --- | --- | --- |
| Schematic editing | Valid symbols | Category values |
| Sending a part instance to the board | An exact footprint and a valid pin map | — |
| Assembly BOM | Every fitted part instance has a package variant with an exact footprint and a valid pin map. Every chosen MPN meets its part instance's requirements and matches its part and package variant. | MPNs and category values for fitted parts |
| Final 3D render or mechanical export | — | An exact model for every fitted part |

Unresolved requirements name the part instance, the field and the stage, in the same
inspection results for GUI and MCP. Checking never fetches from a supplier,
picks a replacement or assigns a package by itself.

## Machine-readable BOM and inventory automation

The unit system must also cross the application boundary. This addresses the
inventory workflow described in
[griush's follow-up on #217](https://github.com/tangency-labs/pcbcraft/issues/217#issuecomment-6762481616):
an external inventory system needs enough structured information to find owned
parts or suggest alternatives, including for a generic part instance without an MPN.

Provide a versioned machine-readable BOM/export contract through shared GUI/MCP
operations. Its library-data fields include stable part instance references and
quantities, category/schema identities and revisions, typed requirements with
their operators, canonical numeric values and explicit unit identifiers, selected
package references, and the manufacturer/MPN when present. Optional tolerance, voltage
and power requirements remain typed fields. Consumers must not need to parse
formatted Value text, comments or an MPN hidden in that text to recover them.
The export includes fixed part values as exact requirements even when the
instance leaves those fields unset. Equivalent resistance inputs therefore
export equivalent requirements.

The export carries known, unknown and untyped states and any validation failures
explicitly. A direct assembly BOM export checks the same completeness rules,
including package and pin-map validation, even if no board transfer ran. Draft
and diagnostic exports identify unresolved rows. Exact encodings, assembly
selection and row grouping belong to the export design, while the category and
unit semantics are shared with search/comparison.

Inventory connectors may use this same contract to compare a part instance's
requirements with owned-part specifications. Unknown specifications do not count
as a match, and a candidate still needs compatible package/mapping information.
Accepting a replacement is an explicit validated edit. Supplier-cart export and
purchasing are separate integrations; this proposal supplies the typed data they
need without requiring either workflow to be built into the library manager.

## Sources and identity

A **source** is somewhere items come from: bundled data, a native folder, a KiCad
library, a vendor zip, a generator, a Git repository or a remote provider. It
contains one or more libraries. A library can hold any mix of the record kinds
above, so symbol-only and footprint-only libraries are normal.

| Source | Status |
| --- | --- |
| Bundled Basic components and Power | Implemented, through the shared native loader |
| Native folder | Personal sources implemented; the rest under [#92](https://github.com/tangency-labs/pcbcraft/issues/92) |
| KiCad library (`.kicad_sym`, `.pretty`) | Proposed below |
| Vendor download (zip of KiCad files) | Proposed below |
| Generator (for example a connector family) | Proposed: produces native records from explicit inputs |
| Git repository | [#93](https://github.com/tangency-labs/pcbcraft/issues/93) |
| Remote provider or team catalog | [#94](https://github.com/tangency-labs/pcbcraft/issues/94) |

Every source answers the same questions:

- Which libraries and items does it have?
- What is the full definition of an item?
- Why is something unavailable or invalid?

One bad item never hides the rest of its library.

**Identity and revisions.**

- **Address:** an item is addressed by library ID, kind, item ID and revision. A
  path or URL only says where to find a library, so moving a folder changes
  nothing a project refers to.
- **Conflicts:** two copies claiming the same identity and revision with
  different content are a conflict, reported and never chosen silently.
- **KiCad revisions:** KiCad files carry no revision, so the adapter uses a hash
  of each item's normalized content. Any change then reads as a new revision.

**Generators.** A generated item records the generator's identity and version,
its inputs and a digest of its output. It emits an MPN only where the family
rules establish one exactly. The output is pinned, so reopening never reruns the
generator.

**Registering sources.** There are two registration lists:

- **Per person:** libraries you use in every project, kept with your
  preferences.
- **Per project:** libraries the project's team shares, in a small sorted file
  in the project folder, without machine-specific paths.

A project opens even when none of its sources is available. Removing a
registration never removes anything a project has pinned.

## Basic components library

Package the standard categories and the original Basic components data in a versioned
Rust crate, pinned in the application build. Desktop and browser then get the
same defaults with no download. The crate version names a distribution. Each item
and schema in it keeps its own ID and revision, so changing an item means a new
revision even inside an app update. Basic components uses the `basic-components` identity. Historical `starter`
references remain readable through a compatibility registration hidden from browsing;
customized project pins and conflicting target addresses retain their original
identities. Recognized unchanged bundled pins normalize to `basic-components` on
native open, with files changed only by ordinary Save. Code uses the shared library
contract rather than assuming particular bundled items exist. Bundling third-party libraries is not part of
this proposal.

## Pinning into a project

Pinning copies the exact definition into the project with its provenance: source
kind, library ID, item ID, revision, license and the locator it was read from.

```text
library/
  symbols/<library>/<symbol>.r<revision>.json        implemented
  footprints/<library>/<footprint>.r<revision>.json  implemented
  parts/<library>/<part>.r<revision>.json            proposed
  mpns/<library>/<manufacturer-part>.r<revision>.json  proposed
  categories/<library>/<category>.r<revision>.json   proposed
```

The proposed paths are illustrative and need a storage review. Updating a pinned
definition is a deliberate step: preview the newer revision against the
project's use, then apply it as one undoable command. A project-local variant is
a pinned copy the project edits, leaving the shared definition untouched.

## KiCad libraries and vendor downloads

The KiCad adapter reads symbol libraries (`.kicad_sym`) and footprint libraries
(`.pretty` folders of `.kicad_mod` files). It converts selected items into native
definitions and leaves the original files untouched.

- **Clean room.** The adapter is written only from KiCad's published format
  documentation ([S-expression format](https://dev-docs.kicad.org/en/file-formats/sexpr-intro/),
  [symbol libraries](https://dev-docs.kicad.org/en/file-formats/sexpr-symbol-lib/),
  [footprints](https://dev-docs.kicad.org/en/file-formats/sexpr-footprint/)), the
  [user manual](https://docs.kicad.org/9.0/en/eeschema/eeschema.html) and black-box
  tests against KiCad. It never uses KiCad source, headers, patches or
  source-derived fixtures (AGENTS.md). PCBcraft Next's KiCad code stays out: its
  source-exposure record and pause still apply.
- **Refuse rather than approximate.** A KiCad feature the native model can't
  represent yet is reported on that item, which can't be pinned.
- **Licenses travel with the data.** KiCad's official libraries have
  [their own terms](https://www.kicad.org/libraries/license/), as do vendor
  parts. Every pinned item records its license, and none is bundled by default.

A vendor zip of KiCad symbol, footprint and 3D files becomes a library, either
personal or stored with the project. Its symbols and footprints go through the
KiCad adapter, and its 3D files are handled under #86. The provider's terms are
recorded with each pinned item.

## The Parts Catalog

- **Sources** with their status: available, or unavailable and why, with item
  counts and problems.
- **Search across sources.** Every source returns the same listing shape, so
  searching the project, an installed library, a generator or a supplier looks
  the same. The Schematic workspace offers parts and symbols; PCB layout offers
  footprints and parts.
- **Filters** by kind, source, category, package, manufacturer or MPN, and typed
  category parameters with ranges in compatible units. A source says which filters
  it supports, and a partial search is never presented as a complete one.
- **Compare** selected candidates side by side, highlighting the values that
  differ and the ones that are missing. Matching footprints or ratings never
  makes two parts approved substitutes.
- **Details:** symbol and footprint previews, terminals, pads and pin map, license,
  provenance and validation errors.
- **Actions:** Place, Add to project (pin without placing) and Compare.

Every action that changes a project is an engine command. Searching and listing
are commands too, so MCP can drive the same workflow. Creating and editing items
belongs to the Symbol editor and Footprint editor workspaces
([#78](https://github.com/tangency-labs/pcbcraft/issues/78),
[#79](https://github.com/tangency-labs/pcbcraft/issues/79)).

## Library manager

The library manager acquires, organizes and maintains libraries. The Parts
Catalog searches and places from installed libraries and registered sources,
including unresolved supplier candidates. The manager
extends the source manager in [native-library-sources.md](native-library-sources.md).

- **Connectors.** A connector adapts one kind of external collection to the
  manager's common search, fetch, convert and refresh operations. Examples are a
  native folder, a Git repository, a team database, a manufacturer catalog or a
  distributor service.
  - Each connector declares what it can supply, such as MPNs and prices, or
    symbols and footprints, and which operations it supports.
  - New connectors can be added over time, built in or later as extensions under
    #100, without changing native definitions or the project format.
  - Once items are converted and collected, ordinary use no longer needs the
    connector installed. Only refreshes and fetching linked models do.
- **Installed packages.** A library package is a versioned, data-only
  distribution of native items with a small manifest. Installing one never runs
  code. Several releases can stay installed side by side, and installing a newer
  one never removes an older one or changes existing libraries or projects.
- **Mixing old and new items.** You can collect items from different releases and
  libraries into an editable local library. For example, keep a footprint a newer
  release dropped alongside a newer symbol. Collection brings the selected items
  with the definitions they depend on, such as category schemas. A new
  combination still needs a valid pin map, whichever releases supplied its parts.
- **Local variants.** *Create local variant* makes an editable copy with its own
  identity and a record of the original item and revision. Editing it never
  changes the installed original or projects that use it. A later upstream change
  can be compared with the variant without forcing a merge.
- **Compatibility.** Package releases, item revisions and data-format versions
  are separate:
  - A release names a collection, and an item revision names one definition.
  - A format version says whether PCBcraft can read the data.
  - Old and new releases don't need to be compatible as wholes. Each selected
    item must be readable, and its relationships (pin maps, parameter types,
    model alignment) must validate.
  - A conversion that changes stored content makes a new revision and keeps the
    original with its provenance.
  - A format that can't be read is reported on the affected items. Nothing is
    partly applied.

The manifest format and package directory layout need a storage review before
implementation.

## 3D model storage

Models are post-MVP (#86), but this proposes how a project holds them. A
project chooses one of two modes as its default, and individual models can
override it:

| Mode | Saved in the project | Where the geometry comes from |
| --- | --- | --- |
| **Link** (proposed default) | The pinned model record and binding, including the geometry's content hash | An installed model library or a local cache |
| **Embed** | The record, the binding and the geometry itself | The project. Each unique asset is stored once by hash. |

- **Link** keeps ordinary projects small. If the geometry isn't available locally,
  only 3D coverage is affected.
- **Embed** makes a project portable for offline rendering. **Embed used models**
  shows the added size and any redistribution limits, then copies only the models
  the project actually uses.
- **Switching modes** keeps the model's identity and alignment. Geometry whose
  content doesn't match the pinned hash is rejected, never substituted.
- **Caching** a linked model never changes the project's mode or marks it dirty.

## Related designs

These need their own documents and must respect the boundaries above:

- **Suppliers and pricing (#94).** Supplier offers (SKUs, packaging, MOQ),
  dated price and stock snapshots, and lifecycle and compliance reports.
  - These are provider records. They are never part of a library definition, and
    a refresh never changes a chosen MPN or makes a new revision.
  - They are never saved to a project by default. Keeping a dated quote is a
    separate, deliberate action.
  - Credentials are per person, in the system keychain on desktop. Offline mode
    is checked before any connection.
- **3D models (#86).** The model record (geometry by content hash, units,
  source, license), the binding that places it on a package variant, supported
  formats, rendering and STEP export.
  - Different MPNs can share a footprint but need their own body model.
  - Storage follows the embed/link choice above.
  - A missing model never blocks opening or electrical editing.
- **Library manager implementation (#92, #93).** Manifest format, package
  directory, connector interface signatures and the browser's storage limits, for
  the manager behavior above.

## Proposed order

1. **Agree the records** in #217: part, manufacturer part, category schema,
   instance values and the terminal-to-pads mapping.
2. **Native parts and categories** (#92): the format and codec review, a
   versioned default-library crate with a few standard categories and generic
   parts. Both workflows should work end to end offline.
3. **Shared listing, filters and compare:** across project, local and generator
   sources, including incomplete and conflicting results.
4. **Project completeness rules:** profiles and per-stage checks.
5. **Library manager:** installed packages, collecting old and new items, local
   variants, then acquisition connectors (#93, #94, KiCad and vendor adapters).
6. **3D models** (#86), using the embed/link storage above.

## Open questions

- **Starter content.** Which categories and generic parts should the default
  library ship first? Should schemas and part data share one release version?
- **Completeness defaults.** Which rules should a new project start with?
- **Pin map edge cases.** Is "one or more pads per terminal, each pad used at most
  once or marked not connected" enough? Or do some packages need one pad shared
  by two terminals?
- **Instance values vs fixed part values.** Which category fields should a part
  be able to fix, and how is a conflict with an instance value shown?
- **Inheritance.** Is deferring part inheritance acceptable to the #217
  participants, or is there a generic family that needs it in v1?
- **Model storage default.** Is linking the right default for new projects, with
  embedding chosen per project or per model?
- **Registrations.** Per person, per project or both as proposed? Should PCBcraft
  offer to read a person's existing KiCad library tables?
- **KiCad revisions.** Is a content hash the right revision, and how should it
  read in the interface?

## Acceptance for the first slices

- **Part first:** place a manufacturer part in one undoable command, with its
  part, package variant, symbol, footprint and pin map pinned. Reopen offline
  with all sources unavailable.
- **Generic first:**
  - Place a generic resistor, set 10 kΩ on R1, then choose an 0603 variant
    before sending it to the board.
  - Filter MPN candidates by R1's values. A candidate whose specified
    resistance differs is reported, not applied.
- **Requirements vs specifications:**
  - Leave R1's resistance unset on a part fixed at 0 Ω. A 10 kΩ MPN fails
    matching, and inventory export includes the exact 0 Ω requirement.
  - Require "at least 25 V" on C1. A 50 V part meets it, a 16 V part doesn't,
    and a part with unknown voltage is reported as unknown.
  - Change C1 to "at least 63 V" after choosing the 50 V part. It is marked as
    not meeting requirements in the same command, and the assembly BOM refuses
    it until it is replaced, cleared or the requirement relaxed.
  - Switch C1 from its 0603 variant to an 0805 variant with a valid pin map
    while its 0603 MPN is chosen. The MPN is marked in the same command, and
    the assembly BOM refuses it the same way.
- **Unit validation:** enter `100m` and `0.1R` in a resistance field through GUI
  and MCP. Both normalize to 0.1 Ω and produce equivalent machine-readable
  requirements. Reject a voltage or a non-quantity part-number string in that field
  without changing its prior valid value; leave uncategorized free text explicitly
  untyped.
- **Inventory export:** export a generic resistor with resistance, tolerance
  and minimum power requirements but no MPN. An external consumer can compare
  typed owned-part specifications without parsing the displayed value or a
  comment. Include unresolved/unknown states explicitly and keep an invalid
  selected MPN out of assembly-ready output.
- **Symbol-only library:** place a symbol from a library with no parts. It gets a
  project-local generic part, then a package variant when a footprint is
  assigned. No library authoring is needed, and the project reopens offline.
- **Multi-pad mapping:** map a QFN's exposed pad and several GND pins to one GND
  terminal. Reject a mapping that leaves out a pad without marking it not
  connected, or uses a pad twice.
- **Category search:** compare capacitors from two libraries with compatible units
  converted. Keep a namespaced custom field separate. Report two schemas claiming
  one identity and revision with different content.
- **Bundled library upgrade:** save with one default-library version and reopen
  with a newer one. New source revisions do not replace pinned meanings without
  an explicit update. The qualified `starter`-to-`basic-components` address
  migration preserves those meanings and can change bytes on the first Save;
  subsequent no-op saves retain their bytes.
- **Completeness rules:** check board transfer and assembly-BOM requirements
  under a permissive profile and an MPN-required one, through GUI and MCP.
  Direct assembly BOM export under either profile reports a fitted part instance
  with a missing package or invalid pin map even before board transfer. The
  permissive profile allows a fitted part instance with a valid package and pin map
  to have no MPN. Unfinished work still opens and saves.
