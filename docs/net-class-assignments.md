# Persistent net-class assignments

Issue [#301](https://github.com/tangency-labs/pcbcraft/issues/301) attaches sets of
[project class IDs](net-classes.md) to the existing [sparse net registry](net-registry-storage.md).
Naming a net does not assign a class. Assignments belong to the current schematic
or board instance registry, with no saved endpoint membership graph and no lookup
by net or class name. This slice supports single-use sheet occurrences; repeated
instances and board ownership transfers remain under #307.

## Commands and inspection

`nets.inspect_classes` without parameters returns the current `revision` and
`net_assignments`, containing the registry scope, saved records and meaningful
derived networks. Each network exposes a nullable persistent ID and a canonical
typed `anchor`, including the project, owner, occurrence and object identities.
Inspection does not promote nets or change saved bytes, history or dirty state.

`nets.set_classes` requires `expected_revision`, `target` and the complete
`classes` UUID array. For a saved net, the target is
`{"kind":"persistent","project":"<uuid>","scope":{"kind":"schematic","id":"<uuid>"},"net":"<uuid>"}`;
board-owned targets use `scope.kind: "board"` and the exact board ID. For an
eligible derived network, wrap its inspected anchor as
`{"kind":"derived","anchor":<inspected anchor>}`. The command promotes that
network and sets its classes in one undoable transaction. An empty array removes
assignments; on an unpromoted derived network it is a no-op. Clearing a set keeps
an already promoted net ID. Identical class sets retain their origins and do not
create history steps.

Passing `target` to `nets.inspect_classes` instead returns that net's bindings,
identity status and dimensional resolution under `net_assignments`. Active status
is `ready` or `conflicted`; historical targets report `retired` or
`identity_ambiguous` without a live resolution. Unpromoted targets report
`derived`. General project inspection remains the registry's raw projection;
class resolution occurs only in the explicit target query. MCP returns query
data even with `inspect: false`. Both commands use the shared headless/live path
and the existing live modal/file-operation refusal policy.

Unknown classes, stale revisions, foreign owners, retired/ambiguous edit targets,
bare pins and unnamed one-pin stubs are refused without changing the snapshot,
history or pending board update. `Project::resolve_net_assignment(NetAddress)`
offers the same typed active resolution to native consumers, including the exact
project/owner/net address and binding origins.

## Topology and origin

A record's optional `class_bindings` array contains sorted, unique
`{"class":"<uuid>","origin":"<net uuid>"}` pairs. Origin identifies the net
where that class was explicitly assigned, within the same registry home. It is
retained through split/merge lineage; multiple origins of one class remain
explainable while the resolver evaluates that class once. New explicit classes
use the current net ID. Retained historical records keep origins resolvable.

Renames and geometry-only moves retain assignments. A detached bare pin or
unnamed one-pin stub acquires no class; the unique meaningful continuation keeps
its ID and bindings, including when the primary and fallback anchors disappear.
Every supported dimensional property explicitly permits inheritance onto
meaningful split branches. New property kinds must declare their own policy;
relationship, routing, electrical and thermal constraints are not represented or
implicitly propagated by this vocabulary.

A meaningful merge gets a fresh net ID and the union of source bindings. Hard
minimums combine by maximum, hard maximums by minimum, and differing preferences
at equal precedence remain conflicts through the #300 resolver. Names and UUID
order never choose a winner. The result retains class/revision/property
contributions plus original assignment sources. An explicit complete-set edit can
resolve the choice. Identity ambiguity is separate: resolving it adopts the
selected branch's current assignments, including edits made after the split.

Removing a referenced definition with `net_classes.remove` is refused, including
references retained in historical records. An optional `rebind` UUID replaces
every current and historical reference with a different existing class and
removes the old definition atomically. Undo/Redo restores the exact definitions,
bindings, origins and identities.

## Storage and consumer boundary

Nonempty bindings select `pcbcraft-project/21`, retaining the required nonempty
class catalog. Schematic-only bindings remain under
`schematic_instances.net_registry`; board-owned bindings use
`pcbcraft-instances/3` in the existing board folder. First-board creation moves
the live registry and requalifies its anchors without changing net IDs or
binding origins. Physical `board.json` bytes, accepted connectivity baselines and
pending board updates remain independent of classification edits.

Each record permits at most 4,096 binding pairs, with the existing 4,096-record
and 8 MiB registry limits still enforced. Missing classes/origins, duplicate or
unsorted bindings, empty/null stored arrays, old headers carrying bindings and
new headers without bindings are refused. Projects without bindings retain their
existing encoding; removing the last live binding only lowers the version when
no historical binding remains. Native folder/archive reopen and interrupted
forward/inverse publication preserve the same authority.

Class-aware routing, geometry queries and DRC remain under #302, while #303 owns
editor controls. Until those consumers support classes, a current dimensional
assignment makes those rule-dependent operations unavailable instead of allowing
a board-default-only pass. Direct snapshot DRC reports `UnsupportedNetClasses`.
Deleting tracks remains possible, and an empty-rule classification adds no
dimensional requirement. Naming or inspecting ordinary unclassified nets does
not invoke the class resolver. This slice adds no editor form or automatic
classification by name.
