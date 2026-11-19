# Straight via teardrop records

`pcb.edit_teardrops` adds or removes checked straight-via teardrop intents.
It takes a board UUID, the current `expected_revision`, and an `edits`
object with `add` records and `remove` UUIDs. Each add record has `id`, `via`,
`track` and `parameters`; use a fresh UUID for `id`. A `length` is
`{"percent":20}` or `{"nanometres":100000}`; name the width limit
`width_pct` and use `max_length_nm` and `max_width_nm` for optional caps.
For an add-only or remove-only batch, supply an empty list for the other field.

A straight-via intent names one physical board, a via and a straight track by
identity. The track must end at the via center; the track supplies the layer
and the departure direction. The extension past the outer land on the track
centerline can be set in nanometres or as a whole percentage of the via's outer
diameter. A length cap can reduce that request. The width percentage and
optional absolute width cap limit the finished via-side chord; a contour that
exceeds that limit is refused rather than changing the tangent construction.
Absolute values are positive and bounded by the board coordinate limit;
percentage conversion rounds to the nearest nanometre, ties up.

The outline joins the track-side corners at that extension to the outer
same-side tangents of the via circle. It is a clockwise quadrilateral in
board coordinates (Y down). Coordinates are rounded to the nearest nanometre,
ties away from zero. The rounded vertices must be bounded and strictly convex.
The result is disposable: derive it from the current board view, never write
vertices back as authored data. This is an additive copper outline, not a drill
outline or a checked routing decision. The isolated construction does not query
other copper or need a curved track representation.

The native board stores only intent in [format 37](native-project-storage.md#straight-via-teardrop-intents).
PCB rendering, routing, connectivity, board DRC, and fabrication do not consume
these polygons yet and refuse boards with active intents rather than omitting them.
Clearance-based resizing, curved edges and other attachment types are not implemented.

## Editing tracks and vias

The shared track and via edit commands regenerate straight-via teardrops
against the proposed board before committing. To move a centered via and keep
its taper, move the attached track and via together with `pcb.move_tracks`.
Deleting either source or breaking the centered attachment removes the intent;
an attachment whose requested taper no longer fits also loses its intent.
Undo restores the copper edit and any removed intents in one step.

Regeneration checks the surviving polygons against the new copper and board
outline using the same rules as an addition. If a surviving taper fails those
rules, the edit is refused with the intent, obstacle and rule; no partial edit
is published. Other board edits that cannot account for active teardrops
still require removing the intents first.

## Add and remove preview

The read-only engine planner takes a board view, its current intent list, and
proposed adds and removes. It rejects unknown removals, ambiguous IDs or
sites, and records for another board before asking about rules. Removals are
projected first. The planner then checks new sites in UUID order, so the rule
evaluator sees retained intents and earlier accepted additions in a stable
order. The diff records approved additions, requested removals, and rejected
sites. A rule rejection includes the rule, obstacle, required distance and
actual signed gap. If the evaluator cannot complete a check, or supplies an
incomplete or contradictory report, no diff is returned. A copper clearance
report requires a gap strictly below the required clearance; conflicting copper
contact includes zero gap. Edge spacing requires a nonnegative gap strictly
below the required clearance. An edge entering the copper interior is a
separate crossing report, valid at zero gap and zero required clearance; copper
outside the closed outline is a separate report regardless of edge distance.

The command uses the board's copper clearance query, including applicable net
class rules, and the same outline predicates as board DRC. An addition needs a
closed outline and cannot join separate copper islands. It reports rejected
intents with the rule, obstacle and distances or a geometry reason. For edge
clearance it reports the nearest outline drawing UUID; for crossing it reports
the nearest crossing drawing. For copper outside the outline it references the
nearest closed drawing, not an open path. Distance ties pick the lowest drawing
UUID. Reports for approved additions include derived vertices. A `dry_run: true`
validates the candidate, including the native board size limit, without changing
revision or undo history. Applying the batch rechecks it and records the
accepted additions and removals in one undo step.
A fully rejected batch without removals does not take an undo step. Native save
and reopen preserve the accepted intents, not the session's undo history.
