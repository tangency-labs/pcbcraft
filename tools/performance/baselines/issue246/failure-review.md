# Failure-path review (#246 walkaround solver)

The solver is a pure computation over an immutable captured query. It adds no I/O,
threads, locks or saved state; `pcb.plan_route` reads the session's existing
clearance cache through the query constructor's non-blocking `try_lock`.

| Case | Behavior | Evidence |
| --- | --- | --- |
| Board edited, undone or reopened after capture | `Session::plan_route` answers `stale` without probing | Test `stale_queries_and_cancellation_plan_nothing` |
| Old `expected_revision` on the command | `stale` outcome, no plan | Test `the_command_plans_without_editing_and_feeds_add_tracks` |
| Plan adopted after a later edit | `pcb.add_tracks` refuses a stale `expected_revision`; the solver authorizes nothing | Reading: commit path unchanged |
| Cancellation raised | Checked before every probe; `canceled`, head only | Test `stale_queries_and_cancellation_plan_nothing` |
| Probe budget exhausted | `incomplete`, head only; `probes` never exceeds the budget | Test `the_probe_budget_is_never_exceeded` |
| Invalid budget, width, point or layer | Error before any probe; nothing changes | Tests on budget 0 / 65,537, foreign layer and unknown posture |
| Head breaks clearance or joins two nets | `blocked` at the head, nothing to add | Reading: head probe before walking |
| No way round (dead end, board edge, too-narrow channel) | `blocked` at the farthest confirmed point | Tests for dead end, board edge and channel; legal prefixes commit with DRC agreement |
| The cheaper side of an obstacle dead-ends | The walk returns to the most recent obstacle and goes round its other side; a route blocked either way reports its first try | Test `a_dead_end_on_the_cheaper_side_is_walked_round_the_other_way`; dead-end and edge tests keep their blocked prefixes |
| Hull rounding cuts a corner | Every kept leg is re-probed; the legal prefix is kept | Reading: final confirmation pass |
| Search bounds (24 obstacles, 8 other-side retries, 6 merges, 256 points) | `blocked` or `incomplete`, never an unbounded loop | Reading: every loop is bounded |
| Planning mutates the document | Revision and saved bytes unchanged | Tests `locked_copper_stays_unchanged`, command test |
| Locked copper | Treated as an obstacle; geometry and lock unchanged after commit | Test `locked_copper_stays_unchanged` |
| Nondeterminism | Same plan from reused, fresh and reopened queries and on every benchmark iteration | Test `plans_are_deterministic`; `walkaround` diagnostic |
