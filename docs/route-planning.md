# Walkaround route planning

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
guide the search. Existing pads, tracks and vias are fixed obstacles, locked or
not; nothing is moved or shoved.

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
or blocked result with no drawable leg keeps that preview; only the current reached
result can authorize a vertex, so the retained path cannot be placed for a different
target. Copper
snapping has an 8-pixel pickup radius and a 12-pixel release radius, so nearby
contacts do not alternate at the pickup boundary.

A click freezes its exact target until its plan and the ordinary full-path copper
validation finish. The pointer cannot replace that clicked target. A blocked or
incomplete plan reports its state and fixes no new vertex. The full route, including
through vias, publishes through the existing prepared `pcb.add_tracks` transaction
with one undo entry. Escape, focus/view release, hidden or changed active layers,
size changes, shared edits and project replacement revoke pending adoption.

Validation extends the fixed vertices with the current reached plan and checks a
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
Arcs, zones, differential pairs, length tuning and moving existing copper remain
outside this solver. Routing on filled zones needs current fill integration from #232.
A board containing [circular copper records](circular-copper-records.md) refuses
query capture and `pcb.plan_route` rather than planning against incomplete
straight-only copper.

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
