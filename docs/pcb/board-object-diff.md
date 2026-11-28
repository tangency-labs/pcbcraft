# Compare board objects by ID

The engine's `board::diff::compare` takes two immutable project revisions and
an explicit board address. The address must identify the same project and
physical board in both revisions. This is a Rust API; it does not open files,
expose a command, or change either revision, the saved files or undo history.

It compares authored track segments, through vias, board drawings and zone
outlines. An object is matched by its authored UUID within its kind, never
by geometry. Deleting and redrawing an object at the same coordinates reports
one removal and one addition. Comparing a revision with itself reports no
changes. The [identity inventory](../formats/object-identity.md) explains why placements
with counter-only identities are not compared. A saved legacy rectangular
outline is compared using the same drawing ID and properties it gets when
an edit converts it to a layer-addressed drawing.

Circular arc tracks and via teardrops are not compared yet. If either revision
stores an arc or a teardrop, the comparison returns an error naming that kind
instead of a report, so such an edit is never reported as no change.

The report can be serialized to JSON. It gives the board address and complete
added, removed and changed counts. Each detail identifies the object's kind
and UUID. Added and removed details include the new or old authored properties;
changed details include only the top-level properties that differ, with their
before and after values. A missing side means the property was absent. For
example, moving a via reports its `at` point, with coordinates in nanometres.
Via test-point marks appear under `test_point` when present, with the marked
assembly and fabrication purposes and their faces.

Callers can request 0–256 detailed changes. The report keeps the first changes
in UUID byte order, using kind for ties, and sets `truncated` if any details
were omitted. A limit of zero asks for counts only. Larger limits are refused.
The engine walks the sorted authored records and serializes properties only
for details it keeps. The comparison does not resolve geometry or connectivity,
compare rule sets, compute copper fill or routed length, or render an overlay.
