# Via net assignments

Standalone vias support **Automatic** mode and an explicit net target. While the
Via tool is active, the ribbon's Via net button shows its placement choice. In
Properties, select one or more vias and click Net. Both buttons open the same
searchable chooser: type part of a net name, select a result and confirm Use net.
Results have at most 50 net rows per page, with Automatic always available to
clear explicit intent. Canceling preserves the previous choice. Locked vias must
be unlocked before assignment edits.

Automatic vias inherit the single resolved intent of contacted copper. A via in
empty space has no net unless explicitly assigned. Routing inserts automatic
vias that follow the route's contacted net. Conflicting contacts do not choose
an arbitrary net, and new geometry or reassignment that would join distinct
intended nets is refused without a partial edit.

An explicit target survives movement and physical disconnection. It does not
create copper connections: an assigned island without a pad connection remains
unconnected and is reported by DRC. Copper names, routing contacts, clearance
queries and dimensional class rules consume the same transient membership,
including islands containing only vias and tracks.

## Identity and board review

Assignments use the existing persistent registry, qualified by project and board
scope; names are presentation only. Renaming or uniquely continuing a net retains
its identity. Retired, ambiguous, missing and foreign saved targets remain
inspectable and produce an unresolved-via DRC finding. They never select a
successor by name. Choose a reviewed current target or Automatic to repair them.

Accepted pad connectivity remains separate from current schematic topology. An
explicit target aliases an accepted net only when the complete accepted
membership and current identity agree uniquely. Pending splits or merges retain
distinct intent until reviewed Update PCB; assignments do not accept an update.

## Commands and native storage

`pcb.add_via` accepts optional `target`, using the same scoped persistent or typed
derived target as the net-class chooser. Omission selects Automatic.
`pcb.set_via_net` accepts a nonempty unique `vias` array and exactly one of
`target` or `automatic: true`. Both commands accept `board` and require
`expected_revision`. A derived target is promoted inside the same transaction;
invalid targets, stale requests and canceled preparation do not change history.
GUI, live MCP and headless MCP share these commands.

Board inspection includes `net_assignment` when authored, `assignment_mode`,
`net_state`, resolved `net` and `connected_to_pad`. The latter describes physical
pad contact; an assignment alone never makes it true. A conflicting island has
no single resolved display net, while DRC retains its complete conflicting intent.
On a board with circular copper records, geometry is unsupported: authored
assignment and registry state remain inspectable, but `net` and `connected_to_pad`
are null with a `geometry_error`. Neither a false disconnection nor a partial
straight-only membership is reported.

Current board files use [format 42](../formats/native-project-storage.md#current-board-records).
A via's optional `net_assignment` contains `project`, `scope` and `net`.
Automatic mode omits the address and keeps the same board header. Explicit
null and nil identities are rejected. Historical format 24 requires at least
one assignment. Derived island membership is computed from copper and is
omitted from storage.

**pcbcraft-clipboard/4** carries explicit targets unless a newer [board clipboard
format](../schematic/clipboard.md#pcb-canvas) is required for other copied content. Copy,
duplication and relocation within the same board preserve those addresses. Foreign destinations or foreign
saved scopes are refused rather than silently reusing IDs; cross-project net
remapping is not implemented. Native Undo/Redo and folder/archive codecs preserve
authored intent, including unresolved historical targets.

Zone-based selection (#370), filled-zone connections (#232), plane defaults
(#263), stitching arrays and advanced via constructions remain separate work.
