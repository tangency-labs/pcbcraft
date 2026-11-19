# Straight via teardrop records

This engine boundary is not a layout feature or a native-file extension. No
command, board document, renderer, rule checker or output uses it yet.

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

Before board storage is enabled, the native format and supported attachment
identities need approval. Persist only intent, not generated vertices. A
consumer must not silently ignore active teardrop copper; clearance,
connectivity, board DRC, pours and fabrication integration are not part of this
boundary. No rule for clearance-based resizing or other attachment types is
specified here.

## Add and remove preview

The read-only engine planner takes a board view, its current intent list, and
proposed adds and removes. It rejects unknown removals, ambiguous IDs or
sites, and records for another board before asking about rules. Removals are
projected first. The planner then checks new sites in UUID order, so the rule
evaluator sees retained intents and earlier accepted additions in a stable
order. The diff records approved additions, requested removals, and rejected
sites. A rule rejection includes the rule, obstacle, required distance and
actual distance. If the evaluator cannot complete a check, no diff is returned.

The rule evaluator is supplied by the caller; the planner has no default allow
path. The synthetic fixture measures spacing to two authored obstacle tracks,
not every board rule. No registered command or production evaluator calls the
planner yet. A caller cannot treat a preview as authority to publish a later
edit: an eventual command must check the current revision, enforce complete
rules and publish all changes through the shared undo path.
