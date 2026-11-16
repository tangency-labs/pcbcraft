# Walkaround route planning

`board::route::walkaround` is the engine's first routing solver for
[#246](https://github.com/tangency-labs/pcbcraft/issues/246). It plans one route on
one copper layer around existing copper and returns points; it never edits the
board. The PCB route tool does not use it yet. Agents reach it through the
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
Outline containment and edge-clearance predicates that DRC uses. Hulls only
guide the search. Existing pads, tracks and vias are fixed obstacles, locked or
not; nothing is moved or shoved.

Copper the head touches lends the route its intended membership and physical
island, so the route may start on and run over it. Copper of the route's single
resolved net is exempt as in DRC. All other copper, including copper of unknown
membership, keeps its clearance; a route into an unconnected pad is therefore
blocked rather than joining it.

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
explicit width must satisfy applicable class minimums. A given
`expected_revision` that is not current answers `stale`. `execute` returns
`route_plan` with the resolved width and the plan, also with `inspect: false`;
the revision, history and saved bytes do not change.

The command is in the engine catalog, so headless MCP discovers it. The live
editor forwards engine commands to its session like other read-only queries.

## Limits

Vias and layer changes, arcs, zones, differential pairs, length tuning and
moving existing copper are not planned. The route tool's interactive preview,
snap hysteresis and commit gesture are later #246 slices.

## Measurement

`board_perf walkaround ARTIFACT ENVIRONMENT_JSON ITERATIONS REPORT` times complete
plans over one captured query: the fixture's first six airwires, a free-space route
along the channel recipe's corridor and one across the crowded diagonal region.
Every iteration must reproduce the first iteration's plans exactly, and each first
plan is committed through `pcb.add_tracks` and checked against the exhaustive pair
reference outside the timed boundary. These are solver service costs, not editor
preview or commit latency. The [#246 evidence](../tools/performance/baselines/issue246/README.md)
records the first measurements.
