# Net-class routing and checking

[#302](https://github.com/tangency-labs/pcbcraft/issues/302) connects
[persistent assignments](net-class-assignments.md) to routing dimensions,
[snapshot copper queries](copper-clearance-query.md) and DRC. The consumers use
current stable net identities and the shared [dimensional resolver](net-classes.md).
Net names, class names and physical traversal order do not select rules.

## Contact defaults and copper edits

`pcb.route_rules` reads the current routing policy at `at: {x_nm, y_nm}`.
It accepts an explicit `board` UUID and an optional copper `layer` UUID; omitting
`board` follows the existing single-board compatibility rule. With a layer,
only contacts on that board's layer participate; without one, through-via pickup
combines contacts across the copper stack. Free space returns `applied: null`.
Foreign boards, technical/foreign layers and out-of-range points are refused.
The query returns `revision` and `route_rules: {board, at, layer, baseline, applied}`
without changing the snapshot, history or saved bytes. Live and headless MCP keep
this result when `inspect: false`.

An applied policy contains scoped persistent `nets`, original `bindings` and
`resolution`, including exact class IDs/revisions, values, contributions and
conflicts. Terminal-to-pad correspondence and the accepted physical connectivity
baseline determine which copper carries that intent. Pending schematic changes
can combine current class intent on an accepted physical net; classification does
not silently accept a pending board update. Unknown copper acquires no invented
net identity or class.

Omitted track/via sizes use the starting contact's effective preferences, falling
back to board preferences in free space. The GUI adopts those defaults at pickup
and uses the contacted net's clearance for the existing preview halo. Explicit
ribbon choices retain priority independently for width and via sizes until the
board or its saved defaults change. A later unclassified pickup restores only
dimensions still using defaults. Every completed candidate is validated
against its actual membership. Starting in free space and ending on a classified
net does not silently resize a route; an insufficient width is refused.
The halo displays the source-net requirement; obstacle-specific pair spacing is
available from queries and DRC rather than a walkaround router.

Changed classified tracks must satisfy effective minimum width; changed classified
vias must satisfy diameter, drill and annular-ring bounds. Explicit sizes may
resolve a preference choice but cannot bypass those hard bounds. Supplying one
via operand can repair a preferred-pair conflict when the omitted operand has a
single unambiguous preference; differing class preferences still require an
explicit choice for that operand. Conflicting preferences block only omitted defaults that require them, while impossible via
bounds block affected vias rather than unrelated tracks. Empty-rule classifications
add no dimensional enforcement. Existing copper is not resized: it remains
inspectable and DRC reports violations. Unclassified copper retains the earlier
explicit-dimension behavior described in [board rules](board-rules.md).

Validation applies to added or changed copper, so unrelated existing violations
do not prevent a valid edit and deletion remains possible. A changed route that
would join distinct intended nets is refused even when their displayed names
match. Contact enumeration yields through the existing preparation job; dropping,
cancelling or rejecting a job publishes no partial copper. Class/binding edits,
Undo/Redo, reopening or another session invalidate prepared publication authority.

## Spacing and DRC provenance

Each object's applicable minimum is resolved independently; pair clearance is the
larger of the two. Conservative candidate enumeration expands by the maximum
captured requirement, then exact contour decisions use their actual pair threshold.
Tangency remains contact, and a gap exactly equal to the required clearance passes.
Known same-net and physical-island exemptions retain the query's existing semantics;
unknown membership does not establish a same-net exemption.

`Authority::class_rules()` and DRC's optional `class_rules` contain deduplicated
`policies` plus an `objects` map from exact copper address to policy index. These
indices expire with the captured result. `Decision::clearance_nm` reports actual
pair spacing; `Authority::clearance_nm()` is the conservative broad-phase radius.
Disposable probes expose combined `probe_rules`. Overlay results expose policy
through `rules_for(ObstacleId)` with source and added-object identities kept
separate. Neither probes nor overlays save classifications or authorize edits.

Synchronous and incremental DRC consume the same per-object width, via, spacing
and board-edge limits and return the same provenance. Hard contradictions produce
`net_class_conflict` findings on affected object kinds; unrelated checks continue.
An unavailable policy capture produces an explicit incomplete finding rather than
a successful board-default-only report. Unused routing preference conflicts remain
inspectable without blocking checks of existing geometry. Request-local diagnostic
baselines keep `request` provenance and cannot weaken applicable class limits;
authored board-default restrictions still apply before class resolution.

This slice changes no saved vocabulary or project version and adds no class-editor
controls. [#303](https://github.com/tangency-labs/pcbcraft/issues/303) owns that
workflow; repeated-instance/board-transfer qualification, pours/thermals,
relationship/electrical intent, impedance and walkaround/shove routing remain in
their respective follow-on issues.
