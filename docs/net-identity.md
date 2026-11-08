# Persistent net identity

Design recorded November 8, 2026; related work:
[#77](https://github.com/tangency-labs/pcbcraft/issues/77),
[#42](https://github.com/tangency-labs/pcbcraft/issues/42) and
[#122](https://github.com/tangency-labs/pcbcraft/issues/122).
This policy applies to schematic-only, single-board and multiboard projects.
It does not claim that persistent net identity is already implemented.

The current electrical graph is derived and does not have persistent net IDs.
The registry design gives reviewed board updates and authored rules stable
targets without making a second source of electrical connectivity. Wiring,
ports, labels and explicit system connections still determine the graph.

## Storage and addresses

Each board's `instances.json` owns its logical `net_registry`, including stable
net IDs, primary and fallback anchors, lineage, explicit net/class bindings and
unresolved reconciliation records. Its physical `board.json` owns that board's
accepted net/terminal baseline and pending physical removal information.
`project.pc` owns the schematic-only registry under
`schematic_instances.net_registry`, plus a distinct system registry for system
connections when required. System continuity does not merge the separate board copper-net records.

The registry is sparse. Persist intentionally named nets, nets with authored
rules/classes, other authored records that explicitly require a stable current
net target, and identities promoted through a revision-checked stable-ID command.
Accepting a board baseline does not promote all of its nets. The baseline stores
historical terminal membership and source-snapshot-qualified net keys in
`board.json`, with an optional persistent ID only where one already exists.
That history supports comparison with a later derived graph; it does not define
today's logical connectivity or require a mirrored logical member list.

Ordinary unnamed unconstrained networks remain derived even after layout.
A fully routed or heavily constrained board may still have many persistent nets
because real authored bindings need them; sparsity is a registration policy, not
a promise of few entries on every board. The fixed-size anchor records prevent
that registry from becoming a second saved membership graph.
Inspection returns revision-scoped derived addresses without dirtying the project.
Exports capture derived addresses in their source manifests without promotion.
A persisted registry record stores one primary anchor and at most three fallback
anchors: four anchors total, independent of net size. It stores no full or
canonical endpoint-membership list. Lineage/reconciliation records refer to
source snapshots, affected stable targets and resulting identities rather than
serializing another complete current netlist.

A registry address includes project, schematic/board/system scope and net ID. Anchors
use stable component terminal, label/port or junction identities, never display
names or coordinates. The immutable before/after graph provides transient
membership during an edit. If stored anchors vanish but there is one meaningful
continuation, choose a replacement from that continuation and store the bounded
result. If several continuations remain unanchored, expose the ambiguity.
A missing anchor is not permission to reconstruct a net from an unbounded saved
fallback list. Validate stored anchors against the graph before adoption.

Adding or disconnecting a non-anchor terminal does not refresh the fallback list
or rewrite the registry merely because membership/revision changed. Remove a
vanished fallback when necessary, but do not refill or sort the list on every
wire edit. Only anchor/binding/lineage changes modify the persisted record.

### Choosing and retaining the primary anchor

At creation, use a single explicitly selected identity anchor when present.
Otherwise apply fixed kind priority: named electrical label, hierarchical port,
component terminal, then authored junction. Select the lowest canonical key in
that order, and at most three additional distinct keys as fallbacks. Persist the
chosen primary and fallback order once. Adding a lower-sorted endpoint later
does not replace them. This tie-break assigns identity, not electrical privilege
or permission to retain a relationship constraint.

### Canonical anchor key encoding

Compare keys lexicographically as bytes with this fixed field order: kind tag,
project UUID, scope tag, scope UUID, occurrence UUID, numeric object ID, terminal
token length and token bytes. Kind tags are `0x01` label, `0x02` port, `0x03`
component terminal and `0x04` junction. Scope tags are `0x01` board, `0x02`
system and `0x03` schematic-only. System and schematic-only scopes use the project
UUID as scope UUID; their different tags prevent aliasing. No fake board UUID is
allocated for the schematic scope.

Encode tags as one byte, UUIDs as their 16 raw RFC-order bytes, every numeric
object ID as unsigned 64-bit big-endian, and token length as unsigned 32-bit
big-endian followed by the exact UTF-8 bytes of the opaque logical terminal ID.
A component-terminal key uses its physical component ID and a zero occurrence
UUID because its identity spans symbol units. A sheet-owned label/port/junction
uses its occurrence UUID and local object ID, with a zero-length token. Validate
these forms; do not locale-sort, normalize opaque IDs or compare rendered ID
strings. Numeric 9 encodes as `0000000000000009`, before numeric 10 encoded as
`000000000000000a`. A library declaring numeric terminal identity must encode it
as numeric data rather than silently treating display pin text as that identity.

Codec test vectors must cover 9 versus 10, UUID ordering, empty versus populated
tokens, all kind/scope tags and component terminals represented on different
sheets. Identical keys are one anchor, not distinct fallback entries.

If the primary is removed, retain the ID on a unique meaningful continuation and
record a new primary from surviving fallbacks. If several meaningful branches
remain without a unique anchored continuation, expose the ambiguity. Any user
resolution becomes stored state rather than being repeated on reopen.

Keeping the net ID does not authorize keeping relationship constraints by
accident. On every relevant membership change, revalidate differential partners,
matching groups and other relationship constraints on the continuation as well
as new branches. Preserve a relationship only when its type-specific continuity
check proves that its intended endpoints and partner still agree; otherwise
retain it as pending review. The numeric/UUID survivor alone is never evidence.

## Ordinary edits and disconnected terminals

Moving geometry, renaming without changing connectivity, and a one-to-one
continuation preserve the net ID and its valid assignments. Connecting a new
previously unconnected pin to an existing net is a continuation, not a merge
with a fictional singleton net.

A bare unconnected terminal is an unconnected endpoint, not a competing net
for split/merge reconciliation. Disconnecting one pin from a classified network
therefore retains the remaining connected network's ID and class. The pin's
unconnected state is available to ERC, inspection and any terminal-specific
constraints, without forcing a net-class decision. If that pin was the primary
anchor, the sole meaningful remaining network keeps the old identity and adopts
a surviving fallback anchor in the same recorded transaction.

For reconciliation, collapse wire segments, bends and unnamed degree-two
junctions to endpoint connectivity. A meaningful network has at least two logical
component terminals, an authored electrical label/port defining a net, or a
constraint explicitly targeting a standalone endpoint/network. One terminal
with only an unnamed dangling wire stub and otherwise anonymous junctions is
still a lone unconnected terminal. A zero-terminal unnamed stub is orphan wire
geometry. Neither becomes a competing net just because the graph has edges.

A class binding on the original multi-terminal net does not turn every detached
pin into an explicitly constrained singleton. Terminal-specific constraints are
still validated separately. Thus deleting a middle segment that leaves one pin
and an anonymous stub preserves the meaningful remainder's identity and class.

An intentionally named singleton or a singleton with explicit authored net
constraints remains an authored net. Such intent cannot be discarded under the
unconnected-terminal rule. Likewise, a split into two connected branches needs
the actual split policy below. Ordinary disconnects must not be reported as
unresolved merely because the old registry contained that terminal.

## Splits and merges

When a net splits into several meaningful networks, the branch containing its
surviving primary anchor retains the old ID. Other branches receive fresh IDs
and lineage to the source net. If there is only one meaningful continuation,
it retains the ID even when the primary anchor is among the detached bare
terminals. If several branches remain and no persisted anchor establishes a
unique continuation, record that ambiguity instead of choosing the largest
branch or a UUID/traversal-order winner. Inspection must show which endpoint
memberships continued or moved to each new identity.

If no meaningful network remains, retire the old identity and retain its prior
intent in historical/review context; do not invent net records for orphan stubs.
For a purely derived network, the same classification rules apply without creating
persistent identities merely to describe the edit.

Class bindings and constraint types explicitly defined as shareable may inherit
on all split branches, with their origin recorded. Constraints whose meaning
depends on one net or a relationship, such as a differential partner or matching
group, require type-specific reconciliation. Unknown constraint types cannot
be copied automatically. The implementation rule contract must specify and test inheritance
per property; no implementation may blanket-copy every binding or discard it.

A merge receives a fresh ID with lineage to its sources. Equal/shareable class
bindings reconcile without a user task, and unclassified connectivity does not
by itself conflict with an existing class. Conflicting explicit classes or
non-combinable constraints remain unresolved, with their source assignments
retained. Many-to-many topology changes use the same anchor and property rules
on the affected subgraph. IDs retired by an actual merge are never reassigned
to another net.

## Review and downstream behavior

Collect genuine reconciliation conflicts without a modal prompt for every wire
edit. An unresolved binding prevents operations that rely on that binding from
claiming a complete rules result; fabrication is gated only when the affected
job requires those unresolved constraints. A bare pin disconnect or a split
with fully valid inherited classes is not such a conflict. ERC and DRC may
independently report disconnected terminals or incomplete physical routing.
Unrelated boards and checks continue to operate.

Undo/redo and save/reopen restore anchors, lineage and unresolved intent together.
Each board retains the accepted source data needed to review physical changes
after a logical terminal or net has been removed. The project-wide numeric ID
watermark specified in the [storage contract](multiboard-storage.md#numeric-id-allocation)
protects any numeric object identities referenced by this registry; new net IDs
are independently allocated and stored.

## Acceptance before implementation

The implementation brief must exercise an isolated pin disconnect and reconnect,
a pin with a dangling unnamed wire stub, deletion of the primary anchor,
a named singleton, a classified two-branch split,
a compatible merge, a conflicting class merge and a lost-anchor ambiguous split.
Tests must distinguish ordinary continuation from a real unresolved rule choice,
including undo/reopen and a pending board update. The fixed anchor encoding above
and each per-constraint inheritance rule must be tested in the implementation
slice; an arbitrary graph traversal cannot supply them. Test the four-anchor cap
and a laid-out board with unnamed baseline nets: a non-anchor wire edit changes
its authored sheet without promoting all baseline nets or rewriting a saved
logical membership list.

A zero-board save/reopen test must retain the schematic registry without a board
file. Scope-key vectors must distinguish board, schematic and system anchors
and ensure a later board assignment updates affected typed scope references
without changing component identities or silently joining global labels.
