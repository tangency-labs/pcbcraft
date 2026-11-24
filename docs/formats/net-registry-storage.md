# Persistent net registry

The identity foundation implements slice 3 of [the net registry plan](../design/net-registry-path.md),
#230, using the single-use sheet occurrences from #229. Net-owned generated
numbers extend it under [#353](https://github.com/tangency-labs/pcbcraft/issues/353).
The existing `project.json`, registered board folders and container-v3 `.pc`
archives remain the storage layout. Connectivity continues to come from wiring,
named anchors and hierarchy; the registry records identity and reconciliation.

## Records and version boundary

The current project entry point accepts an optional registry in the existing
instance home. A board-owned registry uses instances/2, instances/3 with
[class bindings](net-class-assignments.md), or instances/4 with generated numbers.
Board-owned memberships use instances/5 or /7, and pairs use instances/8.
Designator locks remain independent of the registry. Every record stored under project/23 has a positive
`generated_number`; duplicate numbers, explicit nulls, missing numbers and use
under older headers are rejected. Project/24 carries the same records beside
[part groups](part-groups.md), with generated numbers optional: a
registry there numbers every record under the same rules or numbers none.
Without pairs, board-owned expanded memberships select instances/5 and compact
memberships select instances/7. A part-group catalog without
memberships keeps the instance header the other records need, such as
instances/4 for a numbered registry. Active, ambiguous and retired records reserve
their numbers, which are never inferred from component references.
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

## Differential pair records

The native registry can retain explicit differential intent for
[#306](https://github.com/tangency-labs/pcbcraft/issues/306). A nonempty `pairs`
array follows `records` in the existing instance home. Each object has an
independent `id` UUID, persistent net UUIDs `positive` and `negative`, and a
`name`. Member UUIDs must differ and belong to this registry; a net can belong
to at most one pair. Pair IDs are non-nil, distinct from net IDs, and stored in
strictly ascending UUID order. Polarity is explicit, never inferred from a name.
Names are nonempty, trimmed, control-free text of at most 200 Unicode characters.

Omitted `state` means active intent. The reader accepts `{"kind":"active"}`
and the writer omits it. `{"kind":"needs_review","reason":"Member endpoints require review"}`
retains review context, with the same text bounds as `name`. Unknown fields and
states, explicit nulls and missing required fields are refused. Active intent
is not an electrical or routing qualification. When a topology change retires,
merges, splits, or alters the endpoint membership of a member net, the stored
relationship keeps its original pair and member UUIDs without following
successors and transitions to `needs_review` with a review reason. Creating a
pair or changing its endpoints requires active (live) member nets.

The current project format accepts an optional pair list, and board-owned pairs
use instances/8. Legacy project/45 and instances/8 require pairs. A present
`pairs` field must be nonempty. Registries can be fully numbered or fully
unnumbered. Existing
class catalogs and bindings, parameters, no-connect intent, part groups and
assembly profiles keep their existing validation rules. Either expanded or
compact memberships can follow the registry; the field name selects which
encoding instances/8 contains. Removing the last pair keeps the current project
format. The board-owned instance file uses the version required by the remaining
records.

At most 2,048 pairs fit alongside 4,096 net records. Pair content counts toward
the registry's existing 8 MiB serialized limit. Incremental writing charges one
record per pair, and history snapshots share the pair list and individual values.
Read-only registry inspection includes `pairs` only when present. The
[editor](../pcb/net-class-editor.md#differential-pairs) and [shared commands](../automation/board-commands.md#differential-pairs)
author this same relationship without changing the format or the member records:
name-only edits preserve stored review context (`NeedsReview`), while passing
`reviewed: true` (which requires both resulting members to be active) or
changing a member net when both resulting members are active clears
`NeedsReview` back to `Active`. There is no pair class, inference or router.

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

All meaningful nets created by electrical changes receive identities and numbers.
Compatibility projects are not rewritten or promoted by reading or an unchanged
save. Before their first topology edit, the transaction adopts the numbers shown
by its before graph and then reconciles the result, preserving a surviving net's
number even when the edit deletes its original representative component. Explicit promotion
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
inherit source registry identities or numbers; pasted meaningful nets receive destination IDs and numbers
through the normal reconciliation transaction.

## Verification boundary

Regression tests cover disconnect/stub, lost-primary, named singleton, split, ambiguity,
merge, fallback retention and four-anchor cases through shared commands. Include
undo/redo/reopen, malformed codec inputs, fixed anchor-key vectors, multi-unit
terminals, stale revisions, zero-board saves, first-board scope transfer and
journal interruption. Non-anchor wire edits leave instance bytes unchanged,
and board-baseline acceptance does not promote unnamed networks. Promotion alone
leaves the accepted baseline unchanged and its connectivity update a no-op.
Pair regressions cover both storage homes, packed and split round trips,
older-header refusal, invalid record shapes and references, history sharing,
first-board transfer and retention when a member retires. Pair authoring and
physical consumers remain separate.
