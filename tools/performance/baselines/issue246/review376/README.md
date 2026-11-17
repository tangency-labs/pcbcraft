# PR #376 review follow-up

This records the fixes requested on [PR #376](https://github.com/tangency-labs/pcbcraft/pull/376): board minimum width enforcement, connected-path membership validation, incomplete outcomes on search exhaustion, empty/singleton previous-plan inputs and accurate diagnostic coverage wording. The contributor's four commits were rebased onto `f3bb288`; only documentation conflicts needed resolution. Existing authors and Claude credits were retained, with Codex credit on the conflict resolution and review fixes.

Five new regression tests pass with the fixes and fail against the rebased implementation before them. The retained [counterfactual log](counterfactual.log.gz) records those failures. They cover board minimums through both public entry points, separate legs contacting different accepted nets, a later class contact tightening earlier clearance/width, obstacle-search exhaustion with ample probe budget, and empty/head-only continuity hints without weakening ordinary track-path validation. Returned multi-point routes are committed through the shared command and checked with full DRC and the exhaustive clearance reference.

The final validation collects the complete path's contacts before checking its legs with their combined membership and rules. If validation trims the path, it recomputes contacts for the remaining prefix. Search branches and rejected previous plans cannot retain those temporary memberships or their cached legal legs. The command remains read-only, and existing revision, cancellation and stale-query regressions still pass. No saved format or dependency changed.

The failure-path review followed the command dispatch, captured query, search retries, final validation and track-commit boundary. Tests exercise cancellation, exhausted budgets, stale edits/Undo, invalid inputs, legal blocked prefixes and the new membership/width failures. Code inspection additionally checked that all new loops are bounded, temporary membership/cache state is cleared on success and error, and no I/O, worker, lock or session mutation was added. A later commit still uses the existing revision-checked track command.

The matched diagnostic uses the exact saved `channels-v1` and `dense-v1` artifacts, the shipping release profile and 20 iterations before and after. Raw reports are the four adjacent compressed JSON files; environment, compiler, executable hashes and source/fixture fingerprints are adjacent. Both runs reproduce their plans exactly on every iteration. First-iteration plans with at least two points are committed and checked against exhaustive clearance/short decisions; head-only results have no commit-based check.

| Workload, milliseconds | Before p95 | After p95 | Interpretation |
| --- | ---: | ---: | --- |
| Channels, all eight plans | 0.647 | 0.716 | After run is about 0.070 ms higher; this short shared-host pair does not isolate code cost from load |
| Channels, crowded route | 0.519 | 0.559 | Both reach the target with six points; probes increase from 119 to 122 |
| Dense, all eight plans | 3.161 | 1.747 | The crowded result changes, so this is not a speedup comparison |
| Dense, crowded route | 3.126 | 1.694 | Previously `blocked`, nine points/444 probes; now correctly `incomplete`, one point/151 probes when a search bound exhausts |

These observations describe solver service cost on one Linux development host with uncontrolled competing load and thermal/power state. They do not qualify latency tails, allocations, editor preview/commit latency, browser runtime, physical presentation or Wayland endurance. The scope stops after this matched pair and the required correctness checks; no additional broad replay is implied.

All twelve [submission checks](checks.json) passed: formatting, workspace and harness Clippy, 2,193 workspace tests (12 ignored), browser and browser-harness checks, 16 harness tests, 343 WASI board/query tests, dependency license/source/ban checks, local Markdown links, Python script tests and Node performance-page tests. These are automated checks, not a new manual editor or platform qualification.
