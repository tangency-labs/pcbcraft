# Sparse persistent net registry

This implements slice 3 of [the net registry plan](net-registry-path.md), #230,
using the single-use sheet occurrences from #229.
The existing `project.json`, registered board folders and container-v3 `.pc`
archives remain the storage layout. Connectivity continues to come from wiring,
named anchors and hierarchy; the registry records identity and reconciliation.

## Records and version boundary

The low-level codec preserves existing project bytes when no registry is present;
the ordinary first-save board-folder/occurrence migration still applies.
The first record selects project/17 and, for board projects, instances/2, unless
another feature requires a newer envelope. [Class bindings](net-class-assignments.md)
select project/21 and board-owned instances/3 in the same registry home.
`net_registry` is optional within the existing schematic/board instance home;
an explicitly empty or malformed registry is rejected. The registry stores
UUID-sorted records, each with one primary anchor, up to three ordered fallbacks,
snapshot-qualified lineage and an active, ambiguous or retired state. It never
stores a complete endpoint membership list. Retired records reserve their UUIDs.
The registry is limited to 4,096 records, including ambiguous and retired IDs,
and 8 MiB of serialized data. Exceeding either limit refuses the transaction.

Live anchors use the current project and schematic/board scope. Named labels and
hierarchical ports use their sheet occurrence and numeric object ID; component
terminals use the physical component ID, exact terminal token and zero occurrence.
This keeps multi-unit appearances of the same terminal in one identity. The key
codec follows net-identity.md, including the junction/system tags needed for
codec compatibility, while this slice refuses junction anchors and system owners
in registry records and commands.

An ambiguous source keeps its old UUID without assigning it to an arbitrary
branch. Bounded branch references point to independently identified results with
lineage to the source capture. Explicit resolution assigns the ambiguous identity
to the selected current branch and retires its provisional identity. Historical
anchors on retired/ambiguous source records are distinguished from live anchors;
their source capture is retained even when their objects were deleted.
Lineage contains a source capture, a nullable persistent source UUID and one
typed source anchor. A null UUID identifies a meaningful derived source in that
capture. Captures identify the original bundle; they do not embed it or claim
that historical endpoints can be resolved from the current project alone.

## Transactions and reconciliation

Reconciliation runs once at the canonical project transaction boundary. Sheet
transactions defer it until their isolated candidate rejoins the complete project.
Validation, allocation or reconciliation failure publishes no snapshot, history
step or revision. Undo/redo restores the recorded decision instead of rerunning
allocation. Inspection and Open never promote a net or dirty the project.

Meaningful networks collapse anonymous wire geometry and count logical terminals,
labels and ports. Bare pins and unnamed one-pin/orphan stubs are not competing
branches. Transient before/after membership supplies continuation evidence;
it is never saved as a second netlist. Creation picks an explicit selected anchor,
or the fixed kind/key priority. Ordinary non-anchor edits retain stored anchor
order. Removing an anchor prunes it; losing the primary uses a surviving fallback
or a canonical replacement on the unique meaningful continuation.

An anchored split keeps the source UUID on its selected continuation and gives
other meaningful branches fresh UUIDs. Missing/distributed anchor evidence records
an ambiguity. A merge of meaningful networks receives a fresh UUID, with lineage
to persistent or snapshot-qualified derived sources. A bare new pin is not a
second source. No remaining meaningful network retires the identity without
promoting orphan geometry. Later edits keep ambiguity branch references current;
resolving an ambiguity is explicit stored state.

Named nets are promoted by authored electrical changes; compatibility projects
are not rewritten or promoted by reading or an unchanged save. Explicit promotion
uses `nets.promote`, and `nets.resolve_ambiguity` selects a branch. Both commands
are revision-checked, undoable and exposed to headless/live MCP through the shared
catalog and existing live dispatch/refusal policy. There is no new UI. First-board creation moves
the existing registry and requalifies current anchors without changing net UUIDs;
historical source captures and accepted physical baselines retain their identity.

## Commands and inspection

Both commands require `expected_revision` and `anchor`. An anchor can be a
terminal endpoint (`{"symbol":"symbol-1","terminal":"1"}`), a net-label or
hierarchical-port endpoint, or the complete typed anchor returned by inspection.
`sheet` is required when the project has multiple sheets; an optional `board`
must match the current physical owner. `nets.resolve_ambiguity` also requires
the ambiguous source UUID as `net`. The selected endpoint must belong to one
of its current candidate networks. Stale revisions, foreign owners, unsupported
anchors and bare/unnamed one-pin targets fail without changing history.

`Project::inspect_net_registry` reports the scope, saved records and meaningful
current networks, each with a nullable persistent ID, canonical typed anchor and
names. Ordinary project inspection includes this under `project.net_registry`
only when records exist; its existing netlist remains the derived membership
view. Explicit inspection of an older project remains read-only. Copies do not
inherit source registry identities; named pasted nets receive destination IDs
through the normal reconciliation transaction.

## Verification boundary

Regression tests cover disconnect/stub, lost-primary, named singleton, split, ambiguity,
merge, fallback retention and four-anchor cases through shared commands. Include
undo/redo/reopen, malformed codec inputs, fixed anchor-key vectors, multi-unit
terminals, stale revisions, zero-board saves, first-board scope transfer and
journal interruption. Non-anchor wire edits leave instance bytes unchanged,
and board-baseline acceptance does not promote unnamed networks. Promotion alone
leaves the accepted baseline unchanged and its connectivity update a no-op.
Persistent class assignments use this registry's identity and reconciliation;
relationship constraints, repeated circuit instances and zones remain separate.
