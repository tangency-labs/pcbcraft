# Multiboard projects and sheet instances

Design recorded November 8, 2026 for
[#122](https://github.com/tangency-labs/pcbcraft/issues/122).

A project should hold several physical boards, their schematic hierarchy and
their associated documents without becoming a collection of loosely connected
projects. Keep schematic definitions in one shared location and give each board
a folder for its physical design and associated files. A reusable circuit can
appear in distinct sheet instances with separate component identities, before
or after board assignment, with unambiguous cross-probing context.

A project may contain zero or more boards. Schematic-only circuit instances are
valid without a PCB association; when an instance is assigned for PCB work, its
PCB parts belong to exactly one board, whether the schematic is flat or
hierarchical. An optional system overview can organize several board branches, but a single
component-bearing instance should not mix owners. Explicit component
ownership remains necessary in the model to validate assignments and physical
correspondence; mixed sheets are deferred as a separate product and validation
decision.

These contracts describe intended behavior and distinguish it from current
capabilities below. Each implementation slice needs versioned schemas, migration
cases and validation. A later redesign must migrate or explicitly reject older
saved encodings rather than silently reinterpret them. Source and dependency
rules remain unchanged.

## Implementation decisions

| Concern | Decision |
| --- | --- |
| Circuit structure | Wiring, symbol units and child sheet structure are shared by every use of a circuit definition |
| Instance properties | Values and compatible part/package choices may have explicit, visible instance overrides with reset to the shared default |
| Property editing scope | A single-use circuit edits defaults unless the field is already overridden; a shared circuit edits instance properties by default |
| Reference annotation | References are unique with existing case-sensitive matching within a board; system views and combined outputs qualify them with the board |
| Electrical globals | Global names join within their board or project schematic scope; inter-board connectivity requires explicit connector mappings |
| Shared edits | One transaction updates all logical schematic uses; physical updates are reviewed independently for each board |
| Instance identities | Persist occurrence and component mappings; one common circuit-use scope resolves multi-unit packages across sheets |
| Instance storage | Schematic-only instances live once in `project.pc`; board-assigned instances live in that board's `instances.json`, separate from physical `board.json` |
| Schematic-only projects | Start without a board; normal schematic editing and future simulation require no layout, footprint or stackup |
| Flat and hierarchical sheets | Board assignment is explicit in both arrangements; a parent sheet or system overview is optional |
| Net identity | Follow the separate net-identity design, preserving ordinary pin-disconnect continuations and reviewing only genuinely ambiguous assignments |
| Assembly variants | Board-local population/value/compatible BOM alternatives share one PCB; named system builds explicitly select a variant for each board and population for off-board parts |
| Transfer default | Initially transfer only unplaced components with no affected physical implementation; populated/routed transfers are deferred |
| Packaging default | Design packages and Save As include registered authored documents; generated output history requires explicit inclusion |
| Structural editing default | Before editing a definition with several uses, choose Edit shared or Make independent; direct instance-property edits remain available |

The restricted initial transfer, package-inclusion and structural-editing defaults
are selected for the first implementation. They can evolve with later needs.
Intended behavior is distinguished from current capabilities below.

## Current implementation and planned changes

| Concern | Current implementation | Direction for multiboard work |
| --- | --- | --- |
| Boards | Zero or one explicitly addressed board; legacy files retain their compatibility ID; see the [single-board codec](single-board-storage.md) | Persist an ordered board registry and independent board records |
| Sheets | Shared sheet documents with stable IDs; each child has at most one parent; repeated instances and cycles are refused | Distinguish a sheet definition from each occurrence in the hierarchy |
| Components | Project-owned logical components; the optional board owns the whole supported capture scope | Stable logical instances with optional board association; distinguish schematic-only and assigned/unplaced states |
| Board setup | One setup on the sole board; the dialog already captures a board target | Independent layers, stackup, materials and output settings for each board |
| Named connections | Global, power and ground names resolve across the current project | Resolve globals within the board; resolve ordinary local names within the sheet occurrence |
| Cross-probing | Identity-based correspondence with board addresses; reveal across sheets is still planned | Include sheet occurrence and circuit-use context and navigate to the exact physical instance |
| Storage | `project.pc`, `sheets/`, optional root `board.json` and pinned libraries | Shared sheet definitions plus a folder for each board; explicit associated-file policies |

The [representation plan](pcb-representation.md#board-addresses-and-multiboard-migration),
[storage contract](native-project-storage.md),
[hierarchy guide](hierarchical-sheets.md) and [named-net guide](net-labels.md)
describe the existing behavior. Issue #122's original description predates the
implemented board-address bridge; its statement that boards have no identity
must not be used as the current implementation baseline.

## Design documents

| Contract | Contents |
| --- | --- |
| [Circuit instances](circuit-instances.md) | Definition/use/occurrence identities, multi-unit parts, extraction, references, overrides and package intent |
| [Multiboard storage](multiboard-storage.md) | Board folders, record examples, allocation watermark, Git recovery, snapshot references and migration |
| [Assembly and outputs](multiboard-assembly-and-outputs.md) | System connections, off-board parts, variants/builds, associated files and export publication |
| [Net identity](net-identity.md) | Sparse registry ownership, continuation, normalized endpoints, anchors and rule reconciliation |

## Board assignment and sheet hierarchy

Flat, hierarchical and combined schematic organization are supported. A normal
schematic occurrence may be schematic-only or explicitly assigned to one board.
Unassigned PCB intent is a separate incomplete state, not a synonym for having
no board. Schematic-only occurrences can contain the same ordinary components,
wires and hierarchy without acquiring a physical owner. A board can have
several root occurrences with `parent: null`; no parent sheet, sheet symbol or
system overview is needed to associate them with that board. In a hierarchical
board branch, descendants use the same owner. A system overview without a board
can optionally organize several board branches and connection annotations, but
may show explicitly system-owned off-board parts. It cannot contain board-mounted
parts with implicit or mixed board ownership.

In a flat project, Controller can own root Power and MCU occurrences while
Display owns root Driver and Buttons occurrences. The browser can show one flat
list with a Board column or filter, group that list by board, or show authored
hierarchy. These presentation choices do not create sheet symbols or electrical
links. Reusing Power on both boards creates distinct occurrences referencing the
same definition, with or without a parent.

Moving a closed set of flat sheets to a new board transfers complete circuit
uses. A closed set includes all units of its components but may still be only
part of a use. In that case, apply the
[partial-use extraction contract](circuit-instances.md#assignment-of-a-partial-circuit-use)
first: allocate a new definition/containing use, preserve selected occurrence and
component IDs, and leave unselected roots under the original use and owner.
This record-level organization does not add a parent sheet to the canvas. If the
source circuit already has several uses, changing only one use's membership
first requires Make independent; it cannot edit the shared group behind the
other board's back. Refuse the partial assignment until that restructuring path
is available.

Board association belongs to circuit uses, occurrences and component
instances, not to a reusable sheet definition. The shared Power definition must
not contain a hard-coded Controller board ID. A board branch explicitly names
its board, and every assigned use/component has that board through its registered
instance document. Schematic-only uses/components instead live in the project's
`schematic_instances` records and have no board binding.
All descendants in the branch must resolve consistently. This redundancy is a
validated ownership relationship, not independent editable defaults. Creating
components uses the captured context; changing a parent cannot recompute and
silently replace stored ownership. The browser can show the board beside a branch
and offer a board-filtered view without duplicating sheet documents.

Reparenting is an organizational operation. Moving a sheet beneath a different
parent must not silently transfer its components to another physical board.
Within the same board, retain occurrence and component IDs. If reparenting would
violate the destination branch's board ownership, refuse the simple move and
offer a separate explicit reassignment operation with an impact preview.

Initially refuse a cross-owner assignment when the selected child would retain
an ordinary electrical parent on the old board/schematic scope. The supported
restructuring path is an explicit, previewed Detach to root operation first,
including removal/remapping of its parent symbol, port wires and affected net
intent. Preserve the detached occurrence/component IDs and its current owner;
only then assign that closed root group to the destination. An affected multi-unit
package must remain closed, and a shared parent definition must be made independent
before changing only one use's structure. Do not convert an electrical parent to
a system overview or discard port connections implicitly. A combined shortcut
must execute the same complete plan atomically or refuse it.

An assignment operation should cover a sheet occurrence and its descendants,
show affected component instances and validate multi-unit packages. Commit the
occurrence association, component ownership and reviewed physical consequences
as one revision-checked, undoable transaction. Existing routing and layer-bound
geometry cannot be copied blindly to a different stackup. Initially allow only
transfers whose affected components are unplaced and have no physical assignments
or accepted-baseline objects requiring removal on the source. The user must
explicitly remove or reconcile those physical objects first; the transfer cannot
quietly delete them. Package choices can travel as instance choices if compatible
with the destination, but physical placement and accepted mapping state cannot.
Preserve component and occurrence IDs, move their records between instance files
atomically and require an explicit reference-renumbering plan for collisions.
Resolve affected system connector endpoints in the same transaction or refuse
the transfer. A transfer that removes routing and creates destination placements
is a separate later workflow.

Every owner transfer, including board-to-board moves and Keep schematic only,
previews both lost and newly joined global-name connections, not just explicit
system links. Require an explicit connection/separation decision or a typed pending
boundary plan. The transaction also reconciles both owners' persistent net
registries, anchors, bindings and scoped references through the
[net-transfer contract](net-identity.md#ownership-transfer-reconciliation); moving
component records alone is insufficient. Source anchors must remain valid and
destination rule conflicts must be resolved or represented explicitly, never
silently dropped.

Assigned but unplaced components are valid, as are intentionally schematic-only
components. An importer or explicit PCB-assignment operation may separately mark
PCB intent as incomplete; report that state without relabeling every boardless
circuit as an error. Never default incomplete content to the first board or drop
it from the relevant completeness report. The importer
must diagnose mixed-owner sheets that the initial editor cannot represent and
offer an explicit restructuring path rather than silently changing ownership.

### Starting from the schematic and assigning boards

A new project starts schematic-only, with an empty board registry. Capture flat
or hierarchical sheets and instantiate components without creating a Board,
physical folder, placed footprint, outline or stackup. Package selection and pin
mapping can be authored before choosing a physical board. Create board is an
explicit action from the Browser or empty PCB workspace; merely opening that
workspace does not create a document. New circuit instances retain stable IDs
and a valid schematic owner while no board exists.

To create the first PCB or divide the design, select sheet occurrences in the
Browser and choose Assign to board. The form offers existing boards and Create board with
a display name. It previews the selected sheets, descendants, affected component
instances, numeric/display-reference collisions and any physical-content blocker.
Apply uses the existing closed-component and unplaced-transfer rules. Flat roots
need no synthetic parent, and units of one package cannot be split across boards.
Cancel changes nothing; creation of a new board together with assignment is one
revision-checked undoable transaction.

New sheet shows Schematic only or the board inherited from the focused schematic
occurrence or explicitly selected board navigation context. A filter alone is
not creation authority. With no explicit context, default to Schematic only even
if the project already contains boards; never bind a simulation sheet to the sole
board merely because it exists. Hierarchical children inherit their parent's
schematic/board and circuit-use context. Create board within the form and create
the sheet atomically; Cancel leaves no unintended board or sheet.

Assigning a closed schematic-only group to a new empty board moves its existing
instance records, preserving component/occurrence IDs, references, overrides and
model links. Preserve use IDs for complete-use moves; partial-use selections
require extraction into a new containing use under the contract above. Preview
that restructuring and collisions for an existing destination board.
Also preview connectivity-scope changes: globals that previously joined other
schematic-only roots must not become silently disconnected or turn into implicit
cross-board links. For affected name-only joins, require an explicit choice to separate their scopes,
preserve continuity through real connector mappings, or leave a visible pending
boundary-resolution state. Explicit wired/port connections need a reviewed
connection/removal plan rather than merely dropping them. Downstream checks cannot
claim a complete design while required boundary intent remains unresolved.
Modeling or simulation of an unresolved boundary cannot silently claim the old
connectivity is still present. This is a deliberate partition operation.

Keep schematic only detaches an eligible unplaced group from its board and moves
its records back to project scope, preserving component/occurrence IDs and applying
the same complete/partial-use ID rules. Existing placements,
copper or accepted objects must be explicitly reconciled first under the same
physical-transfer restrictions and reference/connectivity-scope preview. A detach
must not silently merge its globals with other schematic-only circuits. It does
not delete a board
or its documents/exports implicitly.

Assign to board moves complete uses, extracting partial selections first; it does
not duplicate physical components.
Use on another board is a separate action that invokes closed-circuit extraction
when necessary and creates a new use with new instance identities. The form
distinguishes that reuse from moving the original circuit. Both workflows use the
same validated commands from GUI and MCP; neither asks the user to edit project
files or infer ownership from a folder.

### Schematic-only analysis and simulation

A schematic-only sheet is an ordinary electrical schematic, not a system overview.
It supports wires, labels, power/ground and hierarchy in the project schematic
scope. Global names join within that scope, without joining board-bound or
system-diagram scopes by name. Analysis targets name circuit uses, occurrences
and logical terminal IDs; simulation never requires a fake BoardTarget.

Model references/defaults belong to shared circuit/component definitions, with
compatible instance overrides following the same property-scope rules. Analysis
configurations and stimuli/probes belong to project schematic context, not to a
board's physical file. Large model assets use the existing pinned/source rules.
Assigning an instance to a board preserves those links. Simulator-specific model
formats, solvers and run schemas need their own implementation slice; this design
establishes ownership, not an already implemented simulator.

Simulation-only sources, probes or ideal elements do not acquire invented
footprints or BOM entries. A board-associated use can contain such elements;
its logical owner context does not assert that each element has a physical part.
A later PCB conversion must explicitly distinguish
physical parts from analysis-only content and report missing physical mappings
only for the requested PCB operation. A valid schematic-only save, ERC request
or analysis setup is not invalid merely because footprints and stackup are absent.

### Browser and editor context

For multiboard projects, the Browser offers By board and Schematic organization.
By board shows each board's layout and sheet occurrences under a named navigation
group. These groups are not authored parent sheets. Schematic organization keeps
the flat or hierarchical sheet structure and shows a compact Board column or
badge on root occurrences, with descendants inheriting the visible branch
context. A board filter narrows either view without changing document ownership.
With zero boards, default to Schematic organization. Show Schematic only as a
scope label when it distinguishes mixed project content, rather than repeating
it on every row of an entirely boardless project. Single-board projects retain
straightforward navigation.

A sheet's tab or breadcrumb includes the board context, such as
`Display > Power`. A shared circuit use has a compact `Shared · 2 uses` indicator
when its definition has several uses; count circuit uses, not open views or
symbol units. Power can therefore appear under Controller and Display while
both rows reference the same definition. Opening a row targets that occurrence
and resolves its board-specific properties. An independently opened definition
is labeled `Power · Definition` and exposes the use selector for physical reveal.

Boardless ordinary sheets appear under Schematic only, with normal editing
and no assignment warning. PCB-intended incomplete/imported sheets use a distinct
Needs board assignment state. System
overviews and shared definitions have distinct navigation entries rather than
being shown as ambiguously unassigned board sheets. Keep context visible through
pane/window changes, and preserve the addressed board of other open PCB views.
Browser grouping, filtering and selection are user-local presentation state;
they never reassign components or retarget an already open setup draft.

Changing association uses the explicit Assign to board action and its validated
impact plan. Merely moving a row in a presentation grouping cannot silently
transfer it. Reference the same occurrence/board targets in properties,
cross-probing and MCP so the Browser cannot establish a second ownership model.

### Copying and deletion

Creating another circuit use shares its definition but allocates new use,
occurrence and component instance IDs and fresh board-local references. It starts
unplaced. A copied folder is never discovered as a new board. Duplicating an
entire routed board with remapped physical identities is separate from creating
another schematic circuit use and is outside the first implementation slice.

Make independent forks a closed circuit definition for a selected use, creates
new definition identities and an explicit old-to-new local mapping, and retains
that use's existing physical component and occurrence IDs. Their references,
placements and overrides remain intact when their logical contracts are
unchanged. Definition changes after the fork affect only the fork's uses. A
partial extraction that splits a multi-unit package is refused.
Fork source-owned definitions only; nested reusable circuit definitions remain
shared unless separately made independent. Preserve the nested uses and rebind
their caller slots explicitly, without changing other callers' mappings.

Deleting a shared symbol/unit is a definition edit and affects all its uses.
Removing one unit of a package leaves the component represented by its other
units. Removing its last symbol representation removes that logical component
from the circuit and produces pending physical removals for its uses. Removing a
whole use retires its occurrence and component mappings; it does not delete the
shared definition while another use references it. A separate definition deletion
requires zero remaining uses.

Retired logical IDs in the saved lineage cannot be reused; discarded unsaved
snapshot addresses remain separately qualified as specified in the storage
[contract](multiboard-storage.md#unsaved-snapshots-and-historical-references). Board baselines and pending-removal records
retain enough pinned component/terminal information to review existing physical
objects after the logical source is gone. Removing a logical component does not
erase its physical footprint or attached copper automatically. Undo restores the
same IDs and prior mappings; runtime requests from an older project opening still
fail session/revision validation.

Initially refuse deletion of a board containing circuit uses, components or
physical design geometry. After those are explicitly moved or removed and pending
updates are reconciled, deletion removes only the registered engine-owned empty
design records. Before removing the active board registration, create the
[retired-board preservation record](multiboard-storage.md#retired-board-preservation)
for its retained folder and rebind registered document ownership to that record.
Documents/output history remain at their paths under a valid retired-area owner,
not a project document owner with an out-of-area path. The area is no longer an
active board and cannot be silently reused by a new one. Do not recursively
delete its files; pruning a retirement record requires the explicit empty-area
or reviewed relocation operation defined by the storage contract.

## Cross-probing and navigation

Selecting a symbol in `Controller > Power` resolves occurrence A and its
component instance, then finds that component's Controller footprint. Selecting
the Display footprint resolves occurrence B and returns to `Display > Power`.
Both paths work even if the circuit drawings look identical or their local
symbol labels match. Pin-to-pad selection uses the instance terminal and its
validated mapping, not matching display numbers.

A reusable definition opened without occurrence context has no unique physical
counterpart. The editor should offer a list of its uses or an occurrence selector
before revealing a board target. It must never choose the first occurrence, the
currently visible board or a component with a matching reference string. A
definition-level group preview may deliberately show several counterparts, but
the UI must distinguish that from selecting one physical instance.

Missing package assignments, unplaced footprints, unsupported mappings and
deleted targets should produce explicit states. Highlighting a counterpart does
not create a placement. Board views receive only highlights belonging to their
board; a net that spans several boards cannot produce an implied copper route
between them. Reveal may navigate to the addressed occurrence or board, while
ordinary highlighting preserves the focus and camera behavior in the
[workspace plan](board-workspace-plan.md#correspondence-and-cross-probing).

GUI and MCP must use the same resolution and revision checks. Inspection exposes
the relevant IDs and context, and commands reject omitted targets when ambiguous,
unknown IDs and stale instance mappings. Schematic views must retain their
occurrence targets across panes, windows and saved user-local arrangements;
selections carry the corresponding context without becoming shared design data.

## Independent board setup

Each physical board owns its outline, layer identities, stackup, material
references, placements, copper, local rules and manufacturing settings. Project
fabrication defaults and portable profiles are authored project data that boards
explicitly adopt as pinned copies or refine through reviewed board-local settings.
Changing a project default affects future adoption, not every existing board.
Shared project rules can provide defaults through explicit precedence; neither project
defaults nor shared source material data should force boards to have the same
construction. Controller can have six copper layers and Display two.

Board setup must open for a named board and retain that board's captured target
through editing and Apply. Switching focus to another board must not retarget an
open draft. A stale or deleted target fails without changing either board.
Applying a portable setup profile to two boards is an explicit operation; each
board adopts its own setup, and subsequent edits remain independent. Live linked
profiles would require a separate design.

The current dialog already captures a board target, but the model still stores
one board and one setup. The existing `board_setup()` convenience access and
single-board mutations need a board-specific implementation during migration;
their existence is not evidence that several independent setups already work.
Commands may omit a board only when exactly one physical board is available.
With several boards, require an explicit target rather than selecting the first.

## Project validity and incomplete work

Keep incomplete work structurally valid and editable. A schematic-only circuit,
unplaced component, unset simulation-model assignment or explicitly pending board
boundary can be saved, reopened, inspected and undone. Only an operation that
requires the missing information is unavailable. Do not persist a single editable
Valid flag: derive structural validity and operation readiness from typed records.

Incomplete state cannot excuse duplicate IDs, a component stored at two homes,
missing live targets, cycles, split multi-unit ownership or incompatible records
presented as resolved. A missing optional model assignment is incomplete; a
declared pinned model file that is absent is an integrity error. Current references
must resolve. Retired references in baselines, lineage and historical artifacts
must be explicitly snapshot-qualified and validated as historical data, never
looked up against an unrelated current object with the same numeric spelling.

Every mutating command builds a candidate against a captured revision, validates
all affected records and publishes one complete snapshot. The initial feature
implementation checks whole-project invariants before publication, not just the
operation's claimed dependency set. Later incremental validation must demonstrate
equivalence to that check; it cannot trade away reference or ownership validation.
Failed validation or
Cancel publishes no partial electrical edit or undo entry. Undo/redo restores the
same logical identities and dependencies; allocation metadata follows its separate
monotonic rule. A stale view/request fails before mutation. Stale user-local view
arrangements may show unavailable-target placeholders; they do not mutate or
invalidate the saved electrical project.

Shared edits, extraction, assignment and deletion include their dependent
variants, analysis targets, system links and unresolved records in the impact
plan. Update/remove those live references in the same transaction or refuse the
operation. A disabled analysis requiring retargeting may retain typed historical
context, not a dangling active target. Keep pinned definitions/assets required
by accepted physical baselines until those references have been reconciled;
garbage collection cannot consider only today's schematic components.

Load and migration validate the whole candidate document set before adopting it.
Do not repair failures by dropping records, choosing the first board or ignoring
an unsupported version. Save/recovery follows the journal's documented commit
point: before commit the prior complete data remains; after a committed journal,
recovery completes the recorded successor. It must never expose a mix of two
cross-document revisions. Preserve the existing platform durability limitations
rather than claiming that application tests prove every possible power-loss case.

Each implementation slice must exercise the transitions it adds, including the
failure paths, before the feature is exposed:

| Transition family | Required proof |
| --- | --- |
| Create/assign/detach/transfer | Identity and scope validation; reference collisions; cancel/stale rejection; one storage home before and after save/reopen |
| Reuse/extract/fork/shared edit | Closed component scope; all affected uses validated; overrides, models and baseline assets retained; undo restores the original relationships |
| Delete/unit removal/variant edits | Live dependencies updated or refused; historical removals remain reviewable; no invalid references hidden behind Pending |
| Net and boundary changes | Scope intent and typed unresolved state preserved; no unintended global join, silent disconnect or guessed relationship assignment |
| Save/load/migrate/export | Failure injection at each publication/recovery phase; malformed/unsupported input rejected; prior complete data and unrelated files preserved |

Use the same invariant checker after supported GUI/MCP command sequences, undo,
redo and encode/decode, including randomized sequences and deliberately corrupted
fixtures generated from valid command-built projects. Those tests complement
specific regressions; the scenario list alone is not evidence that the new
implementation already meets the contract.

## Implementation readiness

Implement in bounded slices, recording the migration and acceptance cases for
each. Later changes need a compatibility plan for saved projects.

Board commands report no available board when the registry is empty, rather
than manufacturing an implicit legacy board. A single-board compatibility
default applies only when exactly one board actually exists.

Complete the versioned codecs, command schemas, compatibility UUID test vectors,
net-anchor/property inheritance tables and source-digest/publication test vectors
for the slice being implemented. The behavior and initial defaults are specified here; these engineering details
remain work to perform. Do not silently
change ownership or electrical semantics to avoid an implementation difficulty.

Keep the existing project-wide symbol, component, wire and geometry budgets after
expanding instances, not just when counting definition files. The initial limits allow zero through 64 boards, 64 sheet definitions and 64 expanded
sheet occurrences, with hierarchy depth no greater than the occurrence limit. Existing
document and archive byte/count limits also apply. Validate counts before
expansion and check arithmetic without partial allocation. Larger projects can
be considered through a separate performance and format review.

Mixed component ownership within one sheet occurrence, panelization, repeated
physical assemblies of an already designed board and live linked setup profiles
are separate features requiring their own ownership and lifecycle contracts.
Duplicate component ownership, copied folders and shared mutable setup state
cannot stand in for those contracts.
PCBcraft remains a standalone electronics application; future SolidCraft assembly
links use explicit adapter contracts without transferring document ownership.

## Acceptance scenarios

These scenarios should guide the tests and review for the relevant slices:

- A legacy project migrates without losing data or changing correspondence;
  repeated saves produce no diff and its board ID remains stable.
- Flat roots remain flat after migration. Assigning sheets to two boards creates
  no synthetic parent sheet and changing browser grouping changes no design data.
- Controller and Display have different layer counts and materials. Editing one
  setup leaves the other unchanged, including an already open setup draft.
- Two Power occurrences share one definition but have different component
  instances, footprints and BOM entries. Both directions of cross-probing select
  the correct occurrence and board, even after rename, reparenting and reopen.
- Controller/U1 and Display/U1 coexist. A Display value/part override changes
  only Display's instance file, resets to inheritance and remains visible after
  shared-default edits; incompatible packages fail without guessed mappings.
- An assembly variant changes population or a compatible BOM alternative without
  changing the base wiring, footprint, references or another board's variant.
- Opening the Power definition alone requires an occurrence choice before a
  physical reveal. Opening extra views creates no additional physical objects.
- Multi-unit symbols on different sheets resolve to one package. An assignment
  that would split that package across boards fails without partial changes.
- A same-board reparent preserves ownership. A move across board boundaries
  requires an explicit reviewed transfer; cancel and undo restore the full state.
- Equal net names on different boards or in repeated local scopes create no
  unintended connection. Explicit connector mappings preserve separate board
  copper while reporting the intended system connection.
- A bare pin disconnect preserves the connected remainder's net ID and valid
  class without an automatic fabrication block. Genuine splits/merges follow the
  separate net-identity design's anchor, inheritance and conflict scenarios.
- Unknown board/occurrence IDs, omitted ambiguous targets and stale commands
  fail without fallback, partial edits or a different target being selected.
- Editing a shared circuit reports affected uses, and updating one board cannot
  silently adopt another board's placement or routing changes.
- Make independent changes definition ownership while retaining the selected
  use's component identities and physical associations; another use is unchanged.
- Failed saves or exports preserve prior complete data. Associated documents and
  output history survive normal saves; archive inclusion follows declared scope.
- Per-board exports identify their source and configuration, and later edits
  mark results outdated without changing their recorded contents.
- Older readers refuse unsupported multiboard or instanced projects instead of
  silently truncating them. GUI and MCP exercise the same ownership checks.
- Deleting the highest numeric ID, saving and restarting cannot reuse it after
  watermark adoption; allocation followed by Undo cannot roll the watermark back.
- A legacy board containing both `R1` and `r1` migrates without renaming or
  rejection. They remain distinct under case-sensitive reference matching.
- Ordinary value/part edits in a single-use circuit update defaults without
  adding stale instance overrides. An existing explicit override remains explicit.
- Desired package B with accepted physical package A is reported as pending,
  with both mappings inspectable. A synchronized assembly release cannot silently
  mix B's BOM with A's incompatible footprint.
- A net registry's anchors, lineage and unresolved assignments survive reopen in
  the stated owner document; accepted physical baselines remain separate.
- Place existing sheet again previews extraction when needed, includes every
  unit of shared packages and preserves the source board's existing physical IDs.
- An ordinary wire/global-label edit on a system overview is refused; the System
  connection tool creates only explicitly mapped, typed endpoints.
- A Pro system build selects the intended variants on both boards and includes
  registered off-board parts once. Per-window previews cannot alter its BOM.
- `con`, `aux`, non-ASCII names and case-only names produce distinct portable
  board folders; no variant or design path can escape its registered board folder.
- An archive export writes outside the `.pc.zip`. External/read-only exports take
  only the destination writer lock; internal publication uses source-then-output
  ordering and recovery. Failures leave old complete results.
- A copied, foreign, malformed or conflicting export record cannot register design
  objects or become the current release merely by being found in an output area.
- Controller/U1 and Display/U1 can both use numeric allocation ordinal 1:
  loading and allocation use board-scoped numeric and displayed-reference checks.
- Unplaced package selection assigns and accepts atomically without #55; a
  placed/routed change is refused until the reviewed-update slice is available.
- A read-only project exports externally without creating a source lock or
  recovering its journal; an existing pending journal is reported safely.
- A split retaining an ID still revalidates relationship constraints, and a pin
  with an anonymous wire stub is normalized as an unconnected endpoint.
- User-owned root boards/builds directories remain untouched; migration selects
  and registers an unoccupied layout root instead of adopting their contents.
- Git merges test both conflicting and equal watermark values with colliding new
  IDs; schema-aware repair updates every affected typed reference atomically.
- An unsaved export followed by discard/reopen cannot alias a new component with
  the same numeric ID: historical links remain source-snapshot-qualified.
- The migrated board remains named Board. Off-board parts default to populated,
  and unnamed unconstrained wire edits do not persist every derived net.
- A board folder segment is joined exactly once under the layout root. Registry,
  variant, document and export paths use the same derived board_root helper.
- A laid-out board baseline retains historical membership without promoting all
  unnamed nets. Persistent records have at most four anchors; a non-anchor
  connect/disconnect does not refresh their member lists because none are saved.
- A shared circuit's PCB package picker edits the selected board's override;
  another board changes only after an explicit shared-default action.
- The Pro build's false battery override omits a part populated in the base build.
- User-owned documents/outputs directories trigger the same layout-root collision
  rule as boards/builds and are not default export destinations.
- Concurrent-branch additions are reported as unsupported until identity-aware
  reconciliation exists; disjoint existing-record edits still undergo validation.
- Anchor codec vectors use fixed-width numeric encodings, including 9 before 10,
  and verify kind, scope, occurrence and opaque terminal-token framing.
- A new schematic-only project saves/reopens with zero boards and no PCB folders;
  wires, labels, hierarchy and circuit instances remain fully addressable.
- Creating a board and assigning a closed group preserves instance/model IDs;
  another boardless group stays schematic-only and is not reported as incomplete.
- Ordinary boardless schematics allow electrical wires and power markers while
  system overviews retain their distinct connection-tool restrictions.
- Boardless references/globals use schematic scope, isolated from board and system
  scopes. Assignment previews any affected boundary/annotation resolution.
- Extracting a sheet containing an already shared nested circuit preserves that
  circuit's definition, use mappings and an unaffected third board's instances.
- Moving an electrical child to another board is refused until it is explicitly
  detached to a closed root with reviewed ports/wiring; no old-board parent remains.
- A transfer previews both lost and joined global-label connections and reconciles
  source/destination net registries, anchors and rule bindings atomically. Whole-net,
  split, merge and ID-collision cases leave no dangling current references.
- Removing an eligible empty board preserves registered documents under a valid
  retired-area owner through save/reopen/packaging; its folder is not reassigned.
- Individually valid CAM from an older capture and a BOM from a newer incompatible
  state cannot form a synchronized release; diagnostic packaging labels the mixture.
- Byte-identical unsaved recreation can retain a numeric ID and content digest,
  but receives a different capture ID. Coordinated CAM/BOM share a capture identity
  while their projected input digests can differ.
- After a net gains C/D terminals, deleting fallback B and then disconnecting
  primary A retains the unique connected C/D remainder's ID with a new anchor,
  even though no historical fallback survives.
- Two flat roots share a schematic-only circuit use. Assigning just one closed
  root to Board extracts a new use, preserving its occurrence/component IDs;
  the other root retains the original use and schematic home. Test the reverse
  move, a shared source definition, nested reusable circuits, typed references,
  cancel, undo/redo and reopen. Unsupported partial assignments change nothing.
