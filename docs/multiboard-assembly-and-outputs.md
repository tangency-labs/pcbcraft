# Multiboard assembly and outputs

This is part of the [multiboard design](multiboard-and-sheet-instances.md) for
#122. It defines intended behavior, with versioned codecs, migration cases and
focused acceptance tests required before shipping.

## Connectivity within and between boards

Keep logical electrical intent distinct from the copper implementation on each
board. Sheet hierarchy organizes circuits, and hierarchical ports establish
logical links; a parent-child relationship alone does not create a connector or
cable. Explicit system connections must identify the board endpoints, connector
component instances, terminals and cable or mating relationship. Each board
still has its own physical nets and routing completeness.

The chosen name scopes are:

| Connection | Resolution scope |
| --- | --- |
| Ordinary local label | One sheet occurrence, including matching interfaces on that sheet |
| Hierarchical port | The explicit child occurrence and its parent-side interface in the same use context |
| Global, power or ground label with a board | That board, across its sheet occurrences and circuit uses |
| Global, power or ground label in schematic-only work | The project schematic scope, across its boardless circuit uses; not board or system scopes |
| System connection | Explicit connector-terminal mapping between addressed boards |

Names retain existing case-sensitive matching. A repeated circuit's local labels
are isolated, but its board-global `GND` connects to the other intended `GND`
labels on that same board. Use local labels and explicit ports when two uses on
one board need separate rails. The definition can use the same global names on
Controller and Display without joining those boards. A schematic-only ordinary sheet
still permits wires, power and ground: lack of a board does not turn it into a
system overview. Assigning part of the project schematic scope to a board must
preview affected name-based connections, migrate applicable rule references and
resolve or explicitly record boundary intent. It cannot silently preserve a
project-global short across boards or silently claim a separated link still exists. A system overview cannot
create a board connection through a global label or an ordinary cross-board
hierarchical wire; it uses system connections instead.

For a legacy one-board project, its existing globals resolve in the original
board scope, preserving connectivity. Adding a second board does not expand that
scope. Scope is part of a net address; a global name is not a persistent net ID.

The initial system connection is an explicit record with a stable ID and terminal
pairs. Each endpoint names either a board connector component and logical terminal
or an explicitly system-owned component terminal, using the tagged address forms
below. Board terminal-to-pad maps remain separately validated. An optional cable
description can accompany a connection without creating a physical BOM item;
a registered cable component has separate ownership and BOM semantics. A cable
catalog and mechanical assembly model are not prerequisites. Matching pin numbers, connector names
or cable descriptions never create a mapping. References and board names can be
renamed without breaking the connection.

Validate that endpoints exist and belong to the stated boards, and distinguish
logical system continuity from each board's copper connectivity and checks.
Removing an endpoint requires an explicit update or removal of its system
connections in the same transaction; it cannot leave a silently dangling link.
Full cable topology, splices and assembly checks remain separate extensions to
this initial connector-pair contract.

### System overview drawing behavior

Register an overview with the explicit `system_diagram` role. It draws board
blocks, explicitly system-owned component symbols and non-electrical annotations.
Board blocks expose selected real connector terminals through stable addressed
interfaces. Their layout is presentation; moving a block does not move a board
or alter its electrical relationships.

Ordinary schematic wires, global labels and power markers are not objects of
the `system_diagram` role and are refused before mutation. Ordinary schematic-only
sheets are a different role with valid project schematic scope; they are not
subject to that refusal merely because they have no board. Drawing an ordinary wire
between two blocks must not create a hidden global connection or silently invent
a cable. Use the explicit System connection tool instead: choose actual endpoints
and review the terminal-pair mapping, then create the typed system connection
record. Its drawn line references that record. A plain annotation line is allowed
but remains non-electrical and visually distinct. Removing a drawn connection
through the connection tool removes its record in the same undoable operation.

An ordinary schematic cannot be converted to an overview by dropping its board
ID. Role conversion requires a reviewed migration of supported objects and must
refuse unsupported electrical content rather than silently changing its meaning.
GUI and MCP enforce the same role restrictions.

### Off-board components and BOM ownership

Store `system_components` in `project.json` for physical parts owned by the system
assembly rather than a PCB: cables/harnesses, panel switches, batteries and other
off-board items. Each has a stable component instance ID, system-scoped reference,
pinned part definition, value metadata and explicit terminal interfaces
when electrically connected. They consume the same project allocation watermark;
they are not unresolved board components and do not need a fictitious footprint.
References are case-sensitive and unique in this system scope; display them as
`System/BAT1`, for example, in combined views.

System endpoints are a tagged alternative to board endpoints. A board endpoint
names board, component and terminal IDs; a system endpoint names a system
component and terminal ID. Connection validation checks the applicable owner
and pinned interface. A harness component with known internal conductors stores
explicit terminal-pair continuity. A cable description without a registered
physical part remains documentation and contributes no invented BOM quantity.

System components are populated by default. Store an explicit false only for
an authored DNP choice; absence of a build override inherits that base population.
System BOMs include populated system component instances once alongside the
selected boards' BOMs. A purchased harness contributes one purchased item;
explicitly modeled manufactured subassemblies need a reviewed roll-up mode to
avoid counting both a purchased assembly and its constituents. Full harness
authoring, lengths, splices and manufacturing documents remain a separate feature,
but the ownership, terminal and BOM records must exist before claiming off-board
parts are supported. Deletion requires updating affected connections and build
selections atomically. A system part is never deleted merely because a board or
overview drawing is removed.

### Persistent net identity

The separate [net-identity design](net-identity.md) covers ordinary single-board
continuations as well as multiboard nets, anchors, splits, merges and rule
reconciliation. Each board's logical registry lives in its `instances.json`;
accepted physical net state lives in `board.json`. The boardless schematic
registry lives in `project.json` under `schematic_instances.net_registry`; any
system registry is a separate project-level record. These storage owners include lineage and unresolved
assignments, not just the current IDs.

A bare isolated pin is an unconnected terminal, so disconnecting it does not
retire the meaningful remaining net or invalidate its class automatically.
Actual conflicting or ambiguous rule changes use that design's reconciliation
policy. The multiboard format must not adopt a stricter global net policy merely
by implementing another board folder.

## Associated files and generated outputs

Authored board data, associated documents and generated results need different
save behavior. Board settings and export recipes are authored design inputs.
Drawings, notes and other imported documents are associated files. CAM, drill,
placement, BOM, schematic PDF and report files are export results tied to a
particular design state and configuration. Neither normal saving nor an unrelated
board edit should overwrite or delete associated files or old export runs.

A schematic-only project can generate schematic/analysis reports and packed
editable designs without a board. Model/run inputs use logical instance and
terminal addresses; they do not require Board setup or PCB output configuration.
Board fabrication jobs require an actual board. Assembly releases count only
explicitly physical board/system parts, not boardless simulation instances or
analysis-only sources/probes. A schematic parts report can include those logical
elements with its scope clearly identified, without inventing physical packages.

Board outputs belong under that board's folder. Project-level schematic exports,
combined BOMs and system reports belong under `join(layout_root, "outputs")`;
registered system documents belong under `join(layout_root, "documents")`. BOM generation
counts physical component instances, not definition files, symbol units or open
views. A definition used on two boards contributes separate physical parts;
project totals should identify each board and its quantity. Build variants can
filter assembly population, but must not duplicate board identities.

Each export run should include a small record naming its board or project scope,
source capture ID and content digest, projected input digest, selected variant or
configuration, generator version and produced files. A source snapshot must cover
relevant unsaved design changes;
a Git commit alone is insufficient. Later edits can mark an export outdated
without rewriting it. Hashes and generation timestamps belong in export records,
not in otherwise unchanged working design documents. Generated outputs should be
optional in version control and should not advance electrical history or mark
the authored design dirty merely because they were generated.

Use a new export destination or a deliberate replace operation, with failures
leaving the previous complete output intact. Deleting a board must separately
account for associated documents and export history; removing a design object
does not authorize recursive deletion of arbitrary user files.

Register authored associated files by stable ID, tagged board, project or
[retired-area owner](multiboard-storage.md#retired-board-preservation), relative
path and media type in `project.json`. A normal packed design and Save As copy the
editable design, its referenced pinned libraries/assets and registered associated
documents. They exclude generated output history by default. A release-package
operation explicitly selects export runs and documents to accompany the design.
Save As does not relocate or remove the original files or create new component
identities; it retains the current project's existing Save As identity behavior.

A synchronized release validates selected artifacts as a set, beyond their
individual checksums. Capture one immutable source for the coordinated release;
all selected CAM, drill, placement and assembly BOM runs must identify that same
capture ID/content digest and the declared system-build/board-variant selection.
Each run's projected input digest must match its inputs in that frozen capture;
CAM and BOM projections need not have identical digests.
For each board, validate the accepted physical baseline, desired/accepted package
consistency and applicable fabrication configuration together. Variant-independent
CAM can declare that scope, but still belongs to the release's captured physical
state; it does not excuse a BOM from incompatible part/footprint intent.

If separately captured or imported runs cannot establish that joint compatibility,
refuse the synchronized release. An explicitly requested historical/diagnostic
package may contain them with per-run provenance and a non-synchronized manifest
status; do not silently downgrade a requested production release or label mixed
states synchronized. Future reuse of unchanged artifacts needs an explicit
validated source-identity reconciliation/provenance record, not merely matching
filenames, board IDs or individually valid content hashes.

Normal saves preserve unknown user files on disk and never adopt them into a
package automatically. Reserve the derived owner document/output areas as preservation areas
with explicit registered or export-owned contents; continue strict validation
of authored design-document locations. Registered associated files missing from
a requested package cause a clear failure, not a silently incomplete archive.
External references must be imported or explicitly excluded before packaging;
symlinks and paths outside the project are not silently followed.

Each export run owns only its declared file list and export record. Enumerating
output areas produces read-only artifact candidates, not registered project
objects; it never creates boards, uses or component mappings. Require supported
format, project/board/build IDs, source capture ID/content digest, projected input
digest and a bounded file list with size/digest integrity before offering a
candidate for use. Confine declared
paths to the run, reject symlinks/traversal, and do not execute artifact content.
Unknown or foreign records are ignored with a concise discovery diagnostic;
invalid records are reported as unusable, without preventing design open/save.
Records naming a preserved former board can be offered as historical artifacts
through its retired-area registration, never as exports of an active board.

Deduplicate matching run IDs with identical validated contents. If one run ID
claims different content at different locations, report a conflict and select
neither automatically. A hand-copied matching run may be shown as another
artifact location, but copying cannot grant it registered status, make it the
current release or change its recorded provenance. Release packaging explicitly
selects a validated run; it does not recursively include arbitrary neighbors.
No run is added to `project.json` merely by generation or discovery, so those
operations do not dirty authored project data. The projected input digest covers
the relevant board's resolved schematic, physical state, rules, pinned dependencies
and configuration, including unsaved edits. Within a validated identity lineage,
an unrelated board's change alone need not make that board's output outdated.
An equal projected digest cannot by itself reconcile different source captures
or admit an older run into a synchronized release.

Keep archive path, type, integrity and expansion checks and existing resource
limits until a separately reviewed change enlarges them. Design and release
packages need distinct versioned manifests so a release containing extra outputs
is never mistaken for an old-format project archive. The binary package format
must register included associated files and selected output artifacts. Current
readers reject unknown managed files and unrelated archive members, so the
illustrated layout requires a format change before it can be used.

### Archive projects and export publication

For a folder project, default board exports beneath `join(board_root, "outputs")`
and system exports beneath `join(layout_root, "outputs")`. All paths use the
[single-segment folder contract](multiboard-storage.md#portable-board-folder-names);
root-level user `documents/` or `outputs/` are not implicitly adopted. For an opened
`name.pc`, the archive is the design save target, not a writable directory.
Suggest a free sibling `name.outputs/` directory with the same board/system run
layout, or let the user choose another external destination. If that default
name already contains unrelated user files, choose a free project-suffixed name
rather than mixing runs into it. Reusing an existing external directory is an
explicit destination choice. Strip the complete `.pc`
suffix when forming that suggested name. Saving the archive never incorporates
external output history automatically or truncates that directory. A release
package explicitly includes selected artifacts. Browser operation downloads a
separate output/release archive and claims no sibling filesystem write.

An export captures an immutable, revision-checked project snapshot, the explicit
board/build/variant selection and all required dependency bytes. It can capture
unsaved state; a save is not a prerequisite. Generation uses that captured input,
not repeated reads of live documents while the user edits them. The export
record identifies the captured source even if it becomes outdated before
generation finishes. Missing or changed pinned dependency bytes fail the job
rather than producing a hybrid snapshot.

All filesystem operations go through the storage/file-job coordinator. An
export to a destination outside the project needs no source writer lock and does
not create a lock sidecar in a read-only source location. Use the existing
read-only capture rules when fetching source-managed dependency bytes: create no
source lock file, take shared access to an existing lock when available, or use
a validated no-journal read when no lock exists. Contention on an existing writer
lock is a bounded busy result, not permission to ignore it. Never recover a
pending source journal without writer authority;
request recovery or a writable copy instead. Already captured immutable project
and pinned dependency bytes are sufficient for generation.

Only publishing inside the project tree requires the source project's writer
lock, journal recovery and owned-preservation-area validation. An unsaved project
or new board without an established owned folder can export to an external
location without first saving. Capturing required external bytes must validate
pinned content/digests and fail on inconsistent reads rather than inventing an
atomic source snapshot from files changed during capture.

Stage results outside design-managed paths, with their own destination transaction
and stable destination lock. Do not put export files in `.pcbcraft-save` or make
them part of an unrelated design save journal. For internal publication, acquire
locks in fixed order source project, then export destination. For external
publication, acquire only the destination lock; it does not wait for a source
writer lock or rewrite an archive. No writer acquires those locks in reverse
order. Use bounded/nonblocking acquisition and a clear busy result.

At internal publication, recover pending saves and revalidate registered owner
and path. A board-folder relocation during generation invalidates the old target
rather than recreating a retired folder. At external publication, validate the
chosen destination and captured snapshot association; concurrent saves can make
the result outdated but cannot change its immutable source or block it merely
because the source is read-only.

Publish a new complete run through a same-filesystem destination transaction;
the complete export record becomes visible only with its declared files. An
explicit replace needs recoverable destination staging/backups and must preserve
the prior complete run until commit. Failure/cancellation removes only this
job's staging, never user files or a previous run. Recovery must distinguish
project-save and export transactions. Test interleaved save/export, archive save,
destination contention, relocation and crashes before/after publication. Existing
platform durability limitations still apply; this design does not claim
stronger fsync guarantees than the storage implementation can provide.

### Assembly variants on one board

An assembly variant is an authored board-local record with a stable variant ID,
board ID and name. Register its ID/owner and derive its path as
`join(board_root, "variants", variant_id + ".json")`, using the storage contract's
single-segment folder rule. The base assembly has no variant record; optional records
contain only population, value and compatible BOM overrides keyed by physical
component instance ID. They do not contain another copy of the circuit or board.

Resolve fields in order: shared component default, board instance override, then
selected assembly variant override. Missing fields inherit; explicit values and
do-not-populate choices remain explicit until Reset. A variant cannot override
component identity, board ownership, reference, wiring, stackup or placement.
An alternate assembly part must fit the already selected physical footprint and
terminal-to-pad mapping; a package override requiring different lands is a board
design change, not an assembly variant. Reject an incompatible alternative rather
than treating it as a harmless BOM substitution.

Variant preview is user-local view state. Export recipes capture an explicit
board and optional variant ID, and output records retain that choice. DNP parts
remain in the circuit and physical design but are marked unpopulated in assembly
outputs. A variant does not assert that the unpopulated circuit is electrically
functional; assembly-specific validation must report that separately. Shared
structural edits validate affected variants and require explicit resolution for
removed components or incompatible alternatives.

### System build selections

A named system build in `<layout_root>/builds/<build-id>.json` has a stable ID, name and one
explicit board-to-variant selection for each active board. A null variant selects
that board's base assembly. It also records sparse population/value/compatible
BOM overrides for system component instances; board components remain controlled
by their selected board variants. Register build IDs and derive their paths from
`<layout_root>/builds/`, not an unrelated user-owned root `builds/` directory. A board variant cannot accidentally become another board's
selection merely because their display names match.

For example, a Pro build can select Controller's telemetry variant and Display's
standard variant while omitting an otherwise populated off-board battery:

```json
{
  "id": "build-pro",
  "name": "Pro",
  "board_variants": [
    {"board": "board-controller", "variant": "controller-telemetry"},
    {"board": "board-display", "variant": "display-standard"}
  ],
  "system_component_overrides": [
    {"component": 501, "populated": false}
  ]
}
```

A combined assembly BOM or system release takes an explicit build ID or an
explicit base selection containing all boards with null variants. It must not
combine whatever variants happen to be previewed in different windows. Record
the resolved selection and source snapshot in its outputs. Validate board and
variant ownership, system component references, population and required connection
endpoints. Deleting a selected variant requires an explicit replacement/base
selection or is refused. Board additions/deletions update affected build rows
as part of the same reviewed project operation, with the base variant proposed
for a new board rather than inferred from another window.

The first build contract selects one assembly of each physical board design.
Board quantities, repeated assembly occurrences and their distinct connector
addresses need a separate assembly-instance contract; increasing BOM quantities
cannot stand in for those identities. Partial-system builds likewise require
explicit excluded-board and connection semantics before support is advertised.
