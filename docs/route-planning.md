# Route planning

`board::route::walkaround` is the engine's first routing solver for
[#246](https://github.com/tangency-labs/pcbcraft/issues/246). It plans one route on
one copper layer around existing copper and returns points; it never edits the
board. The PCB route tool uses it for its single-layer preview and vertex targets. Agents reach it through the
read-only `pcb.plan_route` command.

## What a plan does

A plan runs from a head point toward a target in the 45° posture (one straight
leg and one exact diagonal) or the 90° posture (horizontal and vertical legs).
Where a leg would break clearance, the head follows the blocking copper's hull,
an octagon (or rectangle for 90°) grown by the copper's radius, half the route
width and the pair's clearance, round the cheaper side until the elbow to the
target is clear. Copper met while following a hull joins it. When the walk blocks
further on, it returns to the most recent obstacle and goes round its other side;
a route that blocks either way reports its first try. A final pass replaces runs of
corners with elbows where they are legal. All coordinates stay
whole nanometres and every leg stays in the posture.

Every kept leg is confirmed with the captured [snapshot copper query](copper-clearance-query.md):
the exact `probe` decisions with per-pair, class-aware clearances, and the Board
Outline containment and edge-clearance predicates that DRC uses, so a route cannot
start inside a board cut-out, and a cut-out edge stops it like the outer edge. Hulls only
guide the search. In the default walkaround mode, existing pads, tracks and vias
are fixed obstacles, locked or not.

Copper the head touches lends the route its intended membership and physical
island, so the route may start on and run over it. Copper of the route's single
resolved net is exempt as in DRC. All other copper, including copper of unknown
membership, keeps its clearance during search. Before returning a new or reused
path, the planner combines all of its physical contacts and rechecks every leg
with the resulting membership, width minimum and clearance rules. A failure trims
the path to a legal prefix and recomputes its contacts; it cannot certify a route
that joins distinct nets on different legs.

## Results

| Outcome | Points |
| --- | --- |
| `reached` | From the head to the target |
| `blocked` | From the head to `last_legal`, the farthest legal point; `blocker` names the copper that stopped it, absent for the board edge. When the head itself breaks clearance, the head alone |
| `incomplete` | The head alone: the probe budget or a search bound ran out first |
| `canceled` | The head alone: the caller's cancellation flag was raised |
| `stale` | None: the query no longer describes the session's current board |

Fewer than two points mean there is nothing to add. Reached or blocked points
feed `pcb.add_tracks` (or its preparation job) with the same `layer` and
`width_nm`; that command revalidates them, refuses a stale revision and adds the
whole route in one undo step. The solver adds no saved data and needs no format
change.

If the first approach stops before a legal target, the planner makes one bounded
attempt from the target back toward the head. It retains the original pickup’s
net/island intent and shares the same global probe budget. Only a fully reached,
validated reverse path is returned in the original direction; an obstructed target
cannot lend its net to the route. This avoids false stops in concave pockets such
as a legal gap between pads. If the local walk still cannot resolve a legal goal,
walkaround can search a small visibility graph of up to 32 hull-corner portals
from at most 32 nearby objects, including one bounded ring of neighbors along
their full extents so an attached pad can guide a detour. Its edges are exact-checked posture elbows; only
a complete, connected-path-validated route is adopted. These bounded alternatives
share the original probe budget and contact authority, and do not guarantee a
global shortest path or completeness on arbitrary boards.

When a walkaround cursor is inside copper clearance, the planner can approach a
nearby legal boundary instead of stopping at an earlier obstacle. It projects
onto grown hull edges from at most four blocking objects, includes intersections
between their edges for inside corners, and tries at most four legal nearby
endpoints after at most 64 point checks. A new approach tries endpoints nearest
the cursor; an existing path or fixed bend ranks them through its last endpoint
to keep the continuation on its current side. The searches share the same probe
budget and original pickup intent. Every returned approach is fully validated and remains `blocked` at the requested
cursor. Its optional `adjusted_target` identifies the certified endpoint. Clicking
fixes that visible endpoint after ordinary command preparation; it does not join
copper under the blocked cursor or automatically finish on it. Explicit finishing
still works, and via placement keeps its separate target validation. A fully
validated partial walkaround path also certifies its visible endpoint when no
closer approach succeeds. Invalid starts and canceled, stale or incomplete results
do not grant placement authority; full command preparation still checks board
edges, width and clearance.

While routing, a primary-button release can fix a vertex even if pointer movement
exceeded the canvas's ordinary click tolerance. The release position and modifiers
are captured before later pointer motion, and the clicked target stays owned until
its matching full-path preparation completes. Enter arriving during that work is
retained, including when the pointer has left the canvas. Older prepared geometry
cannot validate a newer target generation.

The search is deterministic: the same board, request and budget give the same
plan. Its work is bounded by a probe budget (default 4,096, at most 65,536 exact
probes; a leg already confirmed costs nothing), at most 24 blocking obstacles per
walk, eight returns to an earlier obstacle's other side, six merges per hull and
256 points. `probes` reports the work charged and
never exceeds the budget. A caller passing `previous`, the last plan of the same
gesture, keeps that plan unchanged while it is still legal and has the same
ends; otherwise the walk prefers the previous side of the head-to-target line
unless the other side is more than 10% shorter, so nearly equivalent routes do
not alternate.

`Session::plan_route` returns `stale` without probing when the query's authority
is not current. A caller adopting a plan later must check the authority again;
`pcb.add_tracks` still checks `expected_revision` at commit.

## Command

`pcb.plan_route` takes `from`, `to` and a copper `layer` UUID, with optional
`board`, `width_nm`, `posture` (`diagonal` or `orthogonal`), `flipped` (the elbow's
other leg first), `previous`, `budget` and `expected_revision`. An omitted width
uses the head contact's net-class preference or the board's preferred width; an
explicit width must satisfy board and applicable class minimums. A given
`expected_revision` that is not current answers `stale`. `execute` returns
`route_plan` with the resolved width and the plan, also with `inspect: false`;
the revision, history and saved bytes do not change.

The command is in the engine catalog, so headless MCP discovers it. The live
editor forwards engine commands to its session like other read-only queries.

## Interactive routing

X starts the existing route tool. Pointer requests capture the view, board, revision,
layer, width, posture and fixed vertices. They coalesce into one current search;
a previous legal path stays visible while a replacement is being solved. An exhausted
or blocked result with no drawable leg keeps that preview for display. A current
reached result can authorize a vertex. Walkaround can also place a vertex at the
certified adjusted endpoint of a blocked result. Copper
snapping has an 8-pixel pickup radius and a 12-pixel release radius, so nearby
contacts do not alternate at the pickup boundary.

A click freezes its exact target until its plan and the ordinary full-path copper
validation finish. The pointer cannot replace that clicked target. An incomplete
plan or a blocked result without a certified adjustment fixes no new vertex. The full route, including
through vias, publishes through the existing prepared `pcb.add_tracks` transaction
with one undo entry. Escape, focus/view release, hidden or changed active layers,
size changes, shared edits and project replacement revoke pending adoption.

Validation extends the fixed vertices with the current eligible plan and checks a
pending via at the last adopted point. Returning to the fixed head can produce a
one-point plan while an older, longer preview remains visible; that preview is
display-only and does not determine the validated path or via location.

`Session::route_planning` captures a reusable `PlanningContext` without projecting
pads. `prepare_route_plan` returns a read-only `PlanningJob`. Hosts check its source
and their input generation before adopting a result. Contact projection yields
in ranges of at most 32 footprints or the supplied contact-pair budget. Search
retains its continuation rather than rerunning earlier probes, advancing at most
eight probe/contact suspension boundaries per call. Final contour, class-policy
and index construction remain bounded by supported model limits rather than a
strict wall-clock slice. Native work uses the existing two-worker limit; browsers
use the existing frame-batch queue. Dropping or canceling a job changes no project.

The synchronous command and resumable editor search run the same solver; existing
MCP clients can continue to plan and submit points with the captured revision.
Neither the query nor a displayed ghost grants authority to bypass final validation.

The route keeps its starting membership while the head advances through fixed
vertices or vias. `Request::origin` carries that original contact and copper
layer for editor requests; a standalone request uses its head. An unconnected
pad has no resolved net to lend, so crossing named copper cannot silently assign
it that net. Contact unions still govern whole-path short and class checks, but
only the starting contact's single resolved net grants a same-net clearance
exemption.

Completed read-only plans are consumed before painting, while final actions stay
after input so cancellation wins over publication. For an already prepared query,
a new pointer request may advance at most eight search suspension units during
its input frame. Cold projection and unfinished searches stay in the existing
queue; a busy query lock yields without waiting. The same continuation moves to
the worker without restarting its completed probes. The
[responsiveness evidence](../tools/performance/baselines/issue246/responsiveness/README.md)
separates frame feedback from CPU service cost and physical presentation latency.

## Limits

The solver plans one copper layer at a time. The editor retains its existing through-via
and layer-change gesture, validating each proposed via with the entire path before
changing layer. It does not search for a multilayer route or choose via locations.
Arcs, zones, differential pairs, length tuning remain
outside this solver. Routing on filled zones needs current fill integration from #232.
A board containing [circular copper records](circular-copper-records.md) refuses
query capture and `pcb.plan_route` rather than planning against incomplete
straight-only copper.

## Track shove

The routing context menu offers **Shove tracks** alongside **Walk around
obstacles**. Shove searches on a disposable overlay, displacing unlocked tracks
while pads, vias and locked tracks stay fixed. Uniform-width, same-layer and
same-membership tracks form open chains through unambiguous degree-two joins.
The captured topology splits runs at contacts made when a trace returns to its
own copper. Free ends, branch contacts, pad/via/lock contacts and width/layer
transitions stay fixed. Ordinary bends are soft hints that can move with their
neighbors when obstructed. Closed or longer-than-255-segment components retain
per-segment anchors in this slice.

The solver starts with the requested elbow. If its cascade blocks, it can try
displaced intervals from their other ends or with a different elbow. When several
runs hit by one approach get in each other's way, a fresh cascade can reserve
ordered lanes and stagger their returns around the cursor. Neighboring runs can
join as those lanes push into their clearance. If a return remains blocked, the
solver can try moving the group farther ahead. Each edit widens across ordinary
bends only as needed and is checked against the same clearance rules and
protected contacts. In diagonal mode, displaced tracks can also retry with square
turns. The solver can continue a previously displayed guide to the new cursor or
change the incoming elbow.

Each candidate runs from the same head to the requested target and shares the
original pickup, fixed gesture copper and probe budget. A previously displayed
guide must be certified again before it can influence placement, including
checking that it stays clear of its own earlier legs on the same layer.

The original pickup is checked against the original copper before any movable
bodies are hidden. A free start inside an existing clearance band is refused,
while a valid contact retains its captured membership through later vertices.

When the board edge blocks the cursor, shove can approach the edge clearance
with an adjusted endpoint. The complete path and its displacements must pass
the same checks as a route that reaches the cursor. A click can fix the adjusted
vertex.

When propagation cannot complete, it discards every replacement and returns a
head-only blocked or incomplete result. Fixed pads, vias and locked tracks guide
the initial walkaround approach. The editor can display the last valid shove
preview during a blocked search. Only a certified result for the current cursor
can authorize placement.

**Merge same-net copper** defaults off. Displaced source tracks that were
originally separated must retain clearance, even if they already belong to one
net or are joined through another branch. Existing direct contacts retain a
local exemption around their junctions. Enabling the option also permits new
contact within the original resolved net or physical island. With merging off,
a displaced track entering a neighbor’s clearance queues that neighbor immediately,
even on the same net; the incoming route need not reach it first. It never permits
joining different nets, moving locked copper or losing an existing connection.
The setting is captured with the gesture and cannot change after fixing a
vertex; it does not rewrite a saved board preference.

`pcb.plan_route` accepts `mode: "shove"`, optional `merge_same_net` and bounded
`fixed` segment/via geometry for an existing gesture prefix. The result includes
source-addressed `replacements` and `shove` work counters. Submit the returned
paths with the incoming route through `pcb.add_tracks`, retaining the same
`merge_same_net` choice and captured revision. The fixed prefix and continuation
share the 256-point placement limit. Each replacement retains its
original width, layer and first segment UUID; additional segments receive new
UUIDs. The compound command independently checks original membership, contact
preservation, clearance, widths and board edges before publishing one Undo/Redo
entry. GUI and command clients use this same transaction.

Trace topology is prepared cooperatively and cached on the immutable source query.
Coalesced requests retain the preparation's progress; a busy cache never blocks a
worker or UI caller. Geometry replacements receive a fresh cache, and query teardown
drops incomplete work without a reference cycle. GUI and direct commands independently
reconstruct replacement chains before accepting their original-contact exemptions.
They preserve protected contact stations, reject centerline crossings and enforce
spacing between returning, originally separate runs. Non-returning corners form a
continuous stroke; existing direct-contact exemptions remain valid. Same-net merging
can relax spacing but cannot authorize a trace self-crossing or lost protected contact.
Unchanged source portions keep their original geometry and IDs; moved bends repartition
only the affected source span, and each original record retains its width and layer.

The counters include the number of attempts within the probe budget and combined collision
and queue work. Geometry payload estimates report the larger attempt, excluding
the captured query/index and allocator overhead; they are not process memory measurements.

Propagation follows the order in which each incoming route piece meets its obstacles,
with source identity breaking ties. Each displaced track stays on the forward side
of its original line; that direction follows the first colliding piece through
cascades and fixed vertices. Interior contact anchors are reserved before moving
any neighbor, so an earlier displacement cannot occupy a later branch junction.
When a local displacement walk fails, the engine tries parallel runs derived
from individual obstacle hulls, retaining forward guide intervals in proportion
to the remaining probe budget while streaming indexed neighbors. Overlapping intervals at the same offset coalesce;
separate intervals remain separate so distant copper does not force a long plateau.
Every body remains in the exact collision checks. These
alternatives share the existing probe budget, cancellation and exact leg/path
validation; they cannot relax a narrow channel’s spacing or move a locked item.

A reached displacement can first try a more compact parallel alternative, spending
at most 128 additional probes within the same gesture budget. Candidate ordering
minimizes swept distance from the original source line, then path length. Only a
complete, exact-confirmed path with lower swept distance replaces the certified
fallback; optional search exhaustion keeps the fallback. This preference changes
the geometry search, not clearance, membership or contact requirements.

A reached displacement with a long backward return can also try up to two local
hull shortcuts, capped at 128 probes within the same gesture budget. A shortcut
must shorten the path, lower its length/backtracking cost and pass full connected
path validation. This cleanup preserves the source interval's fixed contacts,
membership and forward half-plane. If its optional search runs out of work, it
keeps the already-certified path; remaining propagation still needs to finish
within the gesture budget.

The number of displaced segments is bounded by the board's track capacity and
the probe budget. A replacement can contain at most 256 points and a trace
chain preserves at most 128 protected anchors. Shove has a default budget of
65,536 probes, and at most one eighth of that work per cascade is allocated to
optional bend refinement. Nested walks and propagation share that budget.
Cancellation or stale ownership cannot publish partial work, and
the preview hides displaced originals only while their replacement paths are
displayed. A rejected first click inside clearance releases the draft while
leaving the routing tool armed.

## Measurement

`board_perf walkaround ARTIFACT ENVIRONMENT_JSON ITERATIONS REPORT` times complete
plans over one captured query: the fixture's first six airwires, a free-space route
along the channel recipe's corridor and one across the crowded diagonal region.
Every iteration must reproduce the first iteration's plans exactly, and each first
plan with at least two points is committed through `pcb.add_tracks` and checked
against the exhaustive pair reference outside the timed boundary. These are solver service costs, not editor
preview or commit latency. The [#246 evidence](../tools/performance/baselines/issue246/README.md)
records the first measurements, and the [PR #376 follow-up](../tools/performance/baselines/issue246/review376/README.md)
records the connected-path validation cost and corrected search-limit outcomes.

The [review follow-up](../tools/performance/baselines/issue246/review376-followup/README.md)
records bounded previous hints and obstacle avoidance when reusing a plan.

`board_perf walkaround_jobs ARTIFACT ENVIRONMENT_JSON ITERATIONS REPORT` adds
capture, maximum individual preparation/search batches, complete validation,
atomic publication, pointer replacement and cancellation acknowledgement costs.
Every batched plan must equal the synchronous result, and the existing exhaustive
copper oracle checks new geometry outside timed intervals.

The browser export is `run_walkaround_jobs(artifact_json, environment_json, iterations)`.
Its first two arguments are JSON contents, rather than the native command's file
paths; `iterations` is the unsigned 32-bit iteration count, from 1 through 1,000.
The synchronous export returns the report as a JSON string and
does not accept or write a `REPORT` path. These service measurements and rendered
replay readbacks do not certify physical input-to-visible preview latency.
