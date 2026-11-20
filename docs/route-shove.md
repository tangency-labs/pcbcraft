# Shove

`board::route::shove` is the engine's shove solver for
[#247](https://github.com/tangency-labs/pcbcraft/issues/247), the follow-up to
[walkaround route planning](route-planning.md). It lays a new route, or drags
existing tracks, on one copper layer and pushes other copper's unlocked tracks and
vias out of the way instead of going round them. Agents reach it through the
read-only `pcb.plan_shove` query and the `pcb.shove_tracks` edit. The route tool
does not use it yet.

## What a shove does

The shover stays where it is asked to go: a new route's points, or dragged tracks'
new ends. Copper of other islands that breaks clearance with it is pushed, and
copper that moved pushes in turn, breadth first in a deterministic order (vias
before tracks, then by query address).

- **Tracks move as chains.** A chain is a run of tracks of one width on the shove
  layer joined end to end where exactly two track ends meet and no other copper
  touches. Its two ends stay put, so everything joined to it stays joined and the
  ratsnest does not change. The part of the chain inside the pusher's hull is
  replaced by the hull's boundary on one side. The hull is the walkaround's:
  an octagon (a rectangle for a chain of horizontal and vertical legs, which keeps
  its posture) grown by the pusher's radius, the chain's half width, the pair's
  clearance and 2 nm of rounding. The shorter side is tried first; a side is taken
  when it clears every copper that cannot move and turns no corner sharper than
  90°. When the side still meets another leg of the pushing copper, that leg's hull
  merges in and the sides are chosen again, up to four times.
- **Vias move the least distance.** A pushed via moves, in one of the eight 45°
  directions, the fewest whole nanometres that take it out of the pusher's hull;
  the shortest legal move wins. Chains ending on it follow with an elbow from their
  last fixed vertex.
- **What never moves:** pads, locked tracks and vias, the shover itself, copper
  on other layers, the board edge, a chain joined in mid-span (a T or a via on the
  track), a closed loop, and a via joined to a pad, another via, another layer's
  track or a track that does not end at its centre. Copper of the route's own net,
  and of the islands its head touches, is exempt as in walkaround and DRC.

Every probe is an exact overlay probe of the captured
[snapshot copper query](copper-clearance-query.md) with per-pair, class-aware
clearances, plus the Board Outline containment and edge-clearance predicates DRC
uses. When nothing is left to push, the whole result is checked again: every
route leg with the union of every net the route touches, and every dragged, pushed
or moved item. Hulls only guide the search.

## Results

| Outcome | Points and edits |
| --- | --- |
| `reached` | The whole route (or drag) with every push it needs |
| `blocked` | A route's longest legal prefix found by bisection along the path, ending at `last_legal`, with that prefix's pushes; `blocker` names the copper that could not move or be passed, absent for the board edge or a rule. A drag does not stop part way: nothing moves and `last_legal` is the first dragged track's current start |
| `incomplete` | Nothing moves; the route's head alone. A propagation bound ran out (`blocker` names the copper being pushed) or the probe budget did (no `blocker`) |
| `canceled` | Nothing moves; the route's head alone |
| `stale` | Nothing planned: the query or `expected_revision` is no longer current |

A plan lists `reshaped` tracks (identity and new ends), `added` segments (with the
width, layer and membership of the track `like`), `removed` track identities and
moved `vias`, plus `probes`, the deepest push (`depth`) and the number of track
segments and vias changed, added or removed (`moved`). A pushed chain's segments
that equal an original keep its identity untouched, so unchanged tracks keep their
saved lines; the other segments reuse the chain's remaining identities in path
order before new ones are added.

The solver is deterministic: the same board, request and budget give the same
plan. Its work is bounded by a probe budget (default 16,384, at most 65,536 exact
probes), a push depth of 8 (copper pushed by the shover is at depth 1), 64 moved
track segments and vias, 128 single pushes and 256 points per chain.

## Commands

`pcb.plan_shove` and `pcb.shove_tracks` take a copper `layer` UUID and either
`points` for a new route, with an optional `width_nm` (an omitted width uses the
head contact's net-class preference or the board's preferred width, as
`pcb.plan_route` does), or `tracks` to drag, each `{track, start, end}` as
`pcb.reshape_tracks` takes them. Dragged tracks must be unlocked tracks on the
layer; they keep their width and membership, and the caller moves any neighbours
that must follow. Both take optional `board`, `budget` and `expected_revision`.

`pcb.plan_shove` returns `shove_plan` without changing revision, history or saved
bytes, also with `inspect: false`; an `expected_revision` that is not current
answers `stale`. `pcb.shove_tracks` plans the same shove against the exact board
its copper preparation captured and commits it only when it is `reached`: the route
segments, dragged tracks, pushed tracks and moved vias land together in one undo
step through the shared copper preparation and publication path, which still
refuses shorts and class-limit violations. Anything else is refused with nothing
changed, naming the blocker and the last legal point; a blocked route's reported
prefix can be submitted as its own shove.

Both commands are in the engine catalog, so headless MCP discovers them. The live
editor's adapter forwards engine commands to its session; whether to gate the edit
differently there is left to the maintainers.

## Limits

This first slice shoves straight segments and through vias on one copper layer.
Copper on other layers never moves, so a via joined to another layer's track stays
put. There is no springback: a shove starts from the committed board, and pushed
copper does not return when a later edit makes room. The route tool's interactive
preview, multi-layer propagation, arcs, zones, differential pairs and walking round
copper that cannot move (a shove-then-walkaround fallback) are follow-ups. A board
containing [circular copper records](circular-copper-records.md) is refused, as for
walkaround.

## Measurement

`board_perf shove ARTIFACT ENVIRONMENT_JSON ITERATIONS REPORT` times complete
shoves over one captured query: a straight route along each walkaround case (the
fixture's first airwires, the channel corridor and the crowded diagonal). Every
iteration must reproduce the first iteration's plans, and each first reached plan
is committed through `pcb.shove_tracks` on a fresh session and checked against the
exhaustive pair reference outside the timed boundary. These are solver service
costs, not editor preview or commit latency.

The [#247 evidence](../tools/performance/baselines/issue247/README.md) records the
first runs on the `channels-v1` and `dense-v1` fixtures.
