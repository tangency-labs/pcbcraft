# Circuit definitions and instances

This is part of the [multiboard design](multiboard-and-sheet-instances.md) for
#122. It defines intended behavior, with versioned codecs, migration cases and
focused acceptance tests required before shipping.

## Definitions and instances

A sheet definition describes circuit content: symbols, wires, labels and child
sheet references. A sheet instance, also called an occurrence, identifies one
use of that definition within a project hierarchy. Even a definition used once
needs an occurrence identity when the instancing model is introduced.

For example, one reusable Power definition can be used beneath two different
board branches:

```text
System                                  system overview
  Controller                            Controller board branch
    Power                               Power occurrence A
    MCU
  Display                               Display board branch
    Power                               Power occurrence B
    LEDs

Power occurrence A ----> shared Power definition <---- Power occurrence B
         |                                                    |
  component instance A                                 component instance B
         |                                                    |
  Controller footprint                                  Display footprint
```

The two Power occurrences share circuit content, but their components, reference
annotations, footprint assignments, physical placements and resolved electrical
endpoints are distinct. Creating another occurrence allocates distinct component
instance identities; adding an empty board allocates only a board identity.
Multiple editor views of the same occurrence do
not create additional components or BOM quantities.

An edit to a shared definition updates every logical occurrence in one validated,
undoable project transaction. The editor shows the scope and affected uses for a
structural edit. Each board retains its own accepted physical baseline and pending
update; accepting Controller's update does not accept Display's. New logical
components are unplaced until the corresponding board update is accepted, and
logical removals leave physical removals pending for review rather than silently
deleting footprints or copper. Ordinary geometry edits need no repeated modal
confirmation merely because a definition has several uses.

If circuits should evolve independently, an explicit copy or fork creates a new
definition. Project-local occurrences do not independently pin older revisions
of that same editable definition. An external reusable circuit pins a source
revision and retains a project copy; accepting an external update changes that
local definition and all its uses in the same reviewed transaction.

For a definition with more than one use, a board-context editor must establish
structural edit scope before enabling wiring or structural tools. Offer Edit
shared circuit and Make independent, and retain the selected scope while editing
that definition. Do not ask again for every wire segment. A view change, changed
use membership or stale authority must not silently carry the choice to another
definition. With one use, ordinary structural editing needs no shared-scope
choice. Definition-level commands and MCP requests carry explicit shared-edit
authority or a fork operation; GUI protection cannot be bypassed by a less
specific command. A shared geometry edit affects every use of that drawing,
even though no component identity changes.

Repeated nested definitions must expand into distinct child occurrences for each
parent occurrence. Parent references and electrical ports resolve within that
context. Definition cycles remain invalid, and occurrence expansion needs
explicit bounds; sharing a definition cannot bypass project limits.

## Stable identities and addresses

Names, reference text, filenames, array positions and hierarchy display paths
are labels or locations. They must not determine correspondence or ownership.
The conceptual identities below describe required distinctions rather than
prescribing a Rust type or JSON encoding.

| Entity | Identity and relationship |
| --- | --- |
| Project | Existing stable project ID |
| Physical board | Persisted board ID independent of name, folder, stackup and list position |
| Sheet definition | Stable document ID with stable local object and interface identities |
| Circuit definition | Stable grouping of component definitions and sheet definitions, registered in the project |
| Circuit use | One stable instantiation of a circuit definition, shared by all ordinary sheet occurrences within that use |
| Sheet occurrence | Stable project occurrence ID, circuit use, definition reference and optional parent occurrence relationship |
| Circuit component instance | Stable logical identity for one use of a component definition, with a project schematic home or explicit board association; no physical package is required for schematic-only analysis |
| Schematic symbol or unit | Definition-local identity resolved in a sheet occurrence; links to the owning component instance |
| Logical terminal | Stable terminal identity on that component instance, independent of its displayed pin text |
| Footprint and pad | Board-qualified placement and pad identities linked to the component instance through a validated terminal-to-pad mapping |
| Editor view | Runtime view identity plus a project and occurrence or board target; opening another view does not create design objects |

The effective schematic address carries the project, sheet occurrence and local
object ID. A board address identifies the project, board and physical target.
Persisted instance mappings survive save/reopen and reparenting; they are never
regenerated from a display path or traversal order. New board, circuit definition,
circuit-use and sheet-occurrence IDs are independently allocated UUIDs, validated
for uniqueness and stored once. Existing numeric component and object identities
retain their namespace during migration rather than changing identities solely
to adopt UUID spelling. Allocation follows the [storage watermark contract](multiboard-storage.md#numeric-id-allocation); keeping the
current reconstruct-from-live-objects allocator cannot provide non-reuse.

Several schematic units can represent one physical package, including units on
different sheets. They must resolve to one component instance in one schematic
or board ownership context. When physically implemented they have one package
and footprint, rather than one per symbol unit. Schematic-only instances need
neither footprint nor physical BOM item. Sheet-local symbol identity alone is
therefore insufficient for physical ownership. If a proposed sheet assignment
would split a multi-unit package across boards, reject it or require an explicit
change that includes every affected unit and sheet. The instance representation
establishes a common component context across those sheets as described below.

### Circuit use scope

A circuit definition groups its component definitions and sheet definitions.
Its record lives in `project.json`; a circuit does not require another folder or a
file per component. It may contain one root sheet, several flat roots, or roots
with subsheets. This component scope need not appear as another parent sheet in
the editor. A circuit use is one instantiation of that group, with its own stable
ID. Every sheet occurrence names its circuit use. Child sheets inside the same
circuit retain that use; instantiating a reusable nested circuit creates a
distinct child use instead.

Persist an explicit mapping from `(circuit_use, component_definition)` to one
logical component instance. Board assignment may later give that instance a
physical package; it does not create a second component identity. Symbols on
different sheets reference the same component definition and use context when
they are units of one package. Two uses resolve to distinct component instances;
when physically implemented they are distinct packages. Two units within one use
resolve to one component instance. Require exactly one mapping per pair and
reject foreign or missing component definitions.

Reusable circuit boundaries must include every unit of each component they own.
A component cannot be partly inside a reusable circuit and partly in its caller.
Ordinary subsheets can share a component through their common circuit-use scope;
extracting or reparenting sheets across scopes requires validation and an explicit
mapping, not a new package allocated by accident. Nested use expansion persists
the mapping from parent use and definition-local child-use slot to child use.
Reordering or renaming that slot does not change instance identities.

### Extract and place a reusable circuit

Reuse is an operation on a closed circuit, not on an arbitrary sheet file.
The UI can still offer Place existing sheet again. If the selected sheet is
already the complete reusable circuit or one of its declared entry roots, the
action places another use of that circuit. If it is only an ordinary sheet in a
larger circuit, offer Extract reusable circuit first and preview the closure.
Do not instantiate just that sheet while omitting its component scope.

Extraction traverses selected source-owned sheets and ordinary descendants,
including every source-owned sheet needed for all units of their components
until that set is closed. At a nested reusable-circuit boundary, include the
nested use in the impact plan but do not absorb its definition or its component
definitions into the caller's closure. Its own closed component scope remains
independent, even if this caller currently has only one use.
Show additional required sheets, component membership and interface changes.
The user can accept that set or cancel; a component with units outside the set
cannot be partly extracted. An incompatible external interface requires a
reviewed boundary edit in the same operation, not a guessed connection.

For a single-use source, move only the selected source-owned sheet/component
definitions into the new circuit group. Allocate the new group and containing
use IDs, then rebind existing occurrence/component mappings while preserving
their identities, references, owner context, overrides and accepted placements.
Keep every nested circuit definition and its definition-local child slots
unchanged. Rebind the selected nested use's caller/slot mapping explicitly while
preserving its use, occurrence and component IDs; other callers keep their
original mappings. A nested circuit shared elsewhere is not implicitly forked,
moved or rewritten because its caller is extracted. Preserve authored
flat roots and parent/port connections; prove electrical equivalence before
adopting an extraction that claims to be organizational only. If only one use of
an already shared source is changing, Make independent first. No other board's
definition membership may be changed implicitly.

The subsequent placement on another board creates fresh use, occurrence and
physical component IDs and independent board-local references. It is unplaced
on the destination, while its shared wiring/defaults refer to the extracted
circuit. Extraction and placement can be one previewed, atomic undoable command;
cancel leaves both boards unchanged. Reusing Power from a migrated legacy circuit
therefore includes this extraction step, even if the UI presents one action.

### Assignment of a partial circuit use

Each circuit use has exactly one schematic or board ownership home. Component
closure alone does not make a selection a complete use: two independent flat
roots can still belong to the same use. Before an ownership change, classify the
selection as complete uses or partial uses, including the required nested-use
closure. Never move a use while leaving any of its ordinary occurrences or
component mappings behind in another ownership scope.

For a complete use, move its records and preserve its use ID. For a partial use,
first apply the extraction contract above within the source owner, allocating a
new circuit definition and containing use for the selected closed group. Preserve
the selected occurrence/component IDs, overrides and model links; rebind their
use/definition mappings explicitly. The unselected roots retain the original use
ID and owner. If the source definition is shared, Make independent for the
affected source use before changing its membership; other uses remain unchanged.
Preserve nested reusable definitions and selected nested-use IDs as extraction
requires. An electrical child must also satisfy the explicit detach/port rules.

Then transfer only the extracted use and its required nested uses. Update current
use-qualified references, pending intent and scoped net bindings in the same
revision-checked transaction; historical references keep their captured source.
Preview restructuring and reference collisions, and publish extraction plus
assignment as one undoable operation. Cancel, failure and Undo restore both
owners and all prior mappings. Until this restructuring is implemented, refuse
partial-use assignments rather than moving unselected roots or creating foreign
use references; it is not a prerequisite for the initial single-board slice.

### References and resolved properties

Both reference fields use an explicit owner scope. Preserve `reference_number`
as an instance-owned ordinal, unique on `(board, prefix, number)` for board parts,
`(schematic, project, prefix, number)` for schematic-only circuit instances and
`(system, project, prefix, number)` for off-board assembly parts. Scope kind is
part of the key, so schematic and system scopes do not alias even though both
use the project ID. Allocation computes the next number within the addressed owner/prefix, and skips displayed
references already used in that owner. The new loader must replace both current
project-wide checks: numeric allocation ordinals and displayed reference strings.
Changing only the displayed-text check does not enable Controller/U1 plus Display/U1.

Displayed references remain case-sensitive within their owner, accepting existing
`R1` and `r1` together. Manual reference edits retain their separate allocation
ordinal, as today; the displayed suffix need not equal that ordinal. Migration
preserves both fields exactly on the original board. New instances on other
boards can allocate ordinal 1 independently. A board transfer checks both
ordinal and displayed-reference collisions and includes any required reallocation
or renumbering explicitly in its impact plan.
The UI may warn about visually confusing case-only references, but migration
must not reject or silently rename them. Controller
and Display may both contain `U1`. In system views, searches, combined BOMs and
reports, display the board and reference together, for example `Controller/U1`.
Machine addresses still use IDs; board names and reference text are presentation.
Allocate an available reference independently for every board when instantiating
a circuit. If a transfer creates a reference collision, require an explicit
renumbering plan rather than changing references silently.

A definition stores default values and compatible part/package choices. Instance
records store sparse overrides, with absence meaning inherit and a reset action
removing the override. Do not copy all resolved defaults into every instance:
doing so would freeze values unintentionally and create unnecessary Git changes.
Inspection returns the default, override and resolved value with its origin.
An explicit override remains an override even if it currently equals the default;
only Reset restores inheritance.

When a circuit definition has one use, ordinary value/part property edits update
its defaults, preserving existing single-use overrides rather than creating new
ones. If the selected field is already overridden, edit that explicit override
and keep Reset available. The UI identifies the field's origin. A deliberate
Create instance override action remains available, but ordinary single-use schematic
editing must not accumulate stale defaults behind every changed component.

When a definition has several uses, ordinary contextual property edits create
or edit an override for that instance. A separate Edit shared default action
shows all affected uses. The command captures the definition's use count/scope
at the expected revision; a concurrent new use requires reinspection rather than
silently changing an operation's scope.
The schematic renders the resolved value and part for the current occurrence;
the properties form makes overrides visible without adding repetitive guidance
to every symbol. A definition view without a use shows defaults and an instance
selector, not an arbitrarily chosen physical component.

For example, `Controller > Power` can show `U1` using default Regulator A while
`Display > Power` shows its own `U1` using overridden Regulator B. Both components
have distinct IDs. A part override must retain the shared logical terminal and
unit contract; its package may differ only with a complete, validated mapping
from the same logical terminals to pads. Pin numbers alone do not prove
compatibility. An override cannot alter wires, add units or change terminal
identities. A circuit needing different electrical structure must be forked.

Shared default edits update inheriting fields and retain overrides. Structural
edits that invalidate an override or package mapping fail with the affected
instances identified until the same transaction explicitly resolves them. The
editor must not silently discard an override, guess a mapping or leave an
apparently valid but incompatible part. Existing references and values survive
single-board migration; board-local uniqueness permits additional boards without
renumbering the original design.

### Schematic-only instance resolution

The same definition/use/component mapping applies before a board exists. Store
those instances in the project's `schematic_instances`, with no physical owner,
and show references qualified as Schematic/R1 only when the UI needs to distinguish
other owner scopes. Reusing a circuit there creates new component identities and
references in that schematic scope; it does not create footprints or system BOM
items. Shared/default/override behavior is unchanged, including compatible model
parameters for future analysis.

Cross-probing a schematic-only component returns a normal Schematic only/no
board association state, not a missing footprint error. Analysis-only sources or
probes have no physical counterpart even if a surrounding circuit later acquires
a board. Physical conversion must resolve implementation roles and physical parts
explicitly. Logical analysis targets use stable component/terminal IDs and survive
movement between project and board instance storage.

### Package intent and physical acceptance

A physical package choice is optional for schematic-only work. Where present,
there is one authoritative desired package choice: resolve the shared default
and any instance override. It includes a pinned package reference and a validated
logical-terminal-to-pad mapping proposal. Keep these fields together so they
cannot resolve from incompatible layers of defaults. `instances.json` stores
only an override when one exists; it is not another independent package picker.

`board.json` stores the board's accepted physical package/mapping and placement.
That is authoritative for the geometry currently manufactured and checked, not
a competing source of schematic intent. Its accepted source baseline identifies
which desired component state was adopted. A difference between desired and
accepted package/mapping is reported as a pending board update, including an
assigned-but-unplaced change when appropriate. Neither side silently overwrites
the other. Package selection made from the PCB UI submits the same desired-intent
command; it cannot author a third choice.

A PCB-side package picker uses exactly the same field-scope rule as schematic
properties. In a circuit with several uses, Display's ordinary selection creates
or edits Display's instance override and never changes Controller's shared
default. In a single-use circuit it edits the default unless the field is already
explicitly overridden. Edit shared default is a distinct action showing affected
uses; only that deliberate action can create pending updates on other boards.
Capture this intent layer and use-count authority at the expected revision before
choosing the immediate or reviewed physical acceptance path.

For an unplaced component with no physical footprint, copper or accepted objects
requiring review, assign-and-accept is one atomic command: validate the desired
package/mapping, update its intent layer and adopt the board's unplaced assignment
and baseline together. This preserves today's immediate unplaced
`pcb.assign_footprint` workflow without making the whole feature depend on #55.
If the operation would change existing physical objects, do not use this fast
path. Reviewed placed/routed changes require the #55 board-update slice; until
it exists, reject those changes with a clear unsupported-review result. The
implementation brief must name #55 as a dependency for that later path.

Inspection and properties expose desired, accepted and pending states explicitly.
Editing an assembly value alone need not change copper, but adopting a different
footprint or pad mapping needs physical update review. Fabrication always uses
accepted physical data; the export record reports its synchronization state.
A release requiring synchronized schematic/BOM/physical data is refused until
relevant pending differences are resolved. An explicit diagnostic export of an
older accepted board is labeled as such and cannot claim to be the current
assembly release. Build BOMs use resolved design intent only after this release
consistency check, or explicitly identify their schematic-only scope.

## Worked identity scenarios

In the following tables, short names stand for stable IDs rather than display
paths. Each row is a persisted relationship, not a new file.

For a two-board Power circuit, component definition `17` has one physical use on
each board:

| Board | Circuit use | Sheet occurrence | Component definition | Physical component | Reference | Value override |
| --- | --- | --- | --- | --- | --- | --- |
| Controller | use-C | power-C | 17 | 101 | U1 | absent; inherit default |
| Display | use-D | power-D | 17 | 202 | U1 | Regulator B |

Clicking the regulator in `power-D` resolves `(use-D, 17)` to component 202 and
then to Display's accepted footprint or an explicit unplaced state. Renaming
Display or reparenting `power-D` within its valid scope changes no ID in that
chain. Resetting its value override changes only Display's instance document.
Changing the shared default changes the resolved Controller value and leaves
Display's explicit override intact. Adding component definition 18 allocates one
new physical component per use and one pending physical addition for each board.

For a two-unit amplifier spread across flat sheets, both units use one component
definition and one circuit-use scope:

| Sheet occurrence | Parent | Circuit use | Component definition | Symbol unit | Physical component |
| --- | --- | --- | --- | --- | --- |
| input-C | null | amplifier-C | 31 | A | 301 |
| output-C | null | amplifier-C | 31 | B | 301 |
| input-D | null | amplifier-D | 31 | A | 401 |
| output-D | null | amplifier-D | 31 | B | 401 |

Controller has one package, component 301; Display has another, component 401.
Their BOM quantities do not increase because their units occupy two sheets.
Assigning only `output-C` to Display would split component 301 and is refused.
Deleting unit A does not remove a package still represented by unit B; retiring
the component definition or the whole use produces a pending physical removal.
Undo restores the same component and occurrence IDs, including their prior
overrides and accepted physical associations.

## Flight controller and ESC example

A flight controller and an ESC can be separate boards in one project while
using one shared Power supply circuit. Power can itself contain Input filter
and Regulator subsheets. Register Flight controller, ESC and Power supply as
circuit definitions; each board has its own main circuit use and a distinct
child use of Power. The shared Power definition contains its root/child sheet
definitions and closed component set once.

The assigned project can have this working layout (`layout_root` is `.` here;
sheet filenames below stand for UUID-based paths):

```text
flight-system/
  project.json
  sheets/
    <fc-main-id>.json
    <esc-main-id>.json
    <power-id>.json
    <input-filter-id>.json
    <regulator-id>.json
  boards/
    flight-controller-a1b2c3d4e5f6/
      instances.json
      board.json
      outputs/
    esc-0f1e2d3c4b5a/
      instances.json
      board.json
      outputs/
```

Each board's instance file points to the same Power circuit definition and shared
sheet files, with distinct use/occurrence/component IDs. For example:

| Board | Power use | Regulator definition | Component instance | Resolved reference |
| --- | --- | --- | --- | --- |
| Flight controller | power-FC | 17 | 101 | U3 |
| ESC | power-ESC | 17 | 202 | U1 |

References are allocated in the owning scope, so the reusable definition does
not force the regulator to be U1 on both boards. Each Power use expands all its
subsheets in that use context. A multi-unit part spanning Filter and Regulator
resolves to one package per Power use, not one package per sheet.

The Browser can show Power supply under each board with Shared 2 uses, while
the filesystem still contains one copy of each shared sheet. Flat presentation
can instead list each occurrence with its board label. Different values or
compatible part choices live in the respective instance overrides; footprints,
routing and stackups live independently in the two physical board files. Shared
wiring edits affect both logical uses, and Make independent forks one use when
its topology must diverge. The equal GND names do not connect the boards by
themselves; real connector terminals use explicit system mappings.

The same design may begin without either board: both main circuits and Power
uses live in `project.json` under `schematic_instances`, and no board folders are
needed. Later create the boards and assign the closed sheet groups, preserving
their instance IDs and analysis links while reviewing reference and connectivity
scope changes. That operation changes physical association and storage home,
not the identity of the circuit already captured.
