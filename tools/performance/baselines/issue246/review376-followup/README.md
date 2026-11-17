# PR #376 follow-up review

The automatic review of `4c71221` identified two remaining behavior errors and incomplete MCP guidance. Direct Rust requests now validate previous-hint length and coordinate bounds before side arithmetic. Complete-path validation retains the union of contacted nets for short and class-rule checks, while only head-contact islands receive an island clearance exemption. Reusing a previous route therefore goes around a newly added unconnected via or track instead of authorizing contact with it. MCP guidance explicitly requires at least two planned points before calling `pcb.add_tracks`, including when equal endpoints produce a head-only reached result.

Two added regressions fail before these fixes and pass afterward. The [before-fix log](before-tests.log.gz) records the overflowing coordinate panic and invalid reuse. Tests cover empty/singleton hints, extreme coordinates and excessive length at the public Rust boundary, refusal before cancellation/probing, new via and track obstacles, exact obstacle gaps, commit/DRC/exhaustive agreement and the existing membership/class regressions. The full route suite has 23 passing tests.

The failure-path review checked both public entry paths, constructor arithmetic, temporary membership/cache cleanup and connected-path validation. New checks precede walker construction and allocate no new state. Island authority remains fixed from the head, while net membership is recomputed after trimming. Cancellation, budget exhaustion and stale authority still follow the existing tested paths. No dependency, saved format, I/O, worker or lock changed.

The four adjacent compressed reports retain one matched 20-iteration pair on each canonical fixture. Both versions produce identical case outcomes, point counts and probe counts; each run asserts deterministic plans and commits first-iteration plans with at least two points against exhaustive clearance/short decisions. These cases do not use previous hints, so the reuse regression is established by the focused tests rather than the timing workload. Source, fixture and executable hashes and complete environment metadata are adjacent.

| All eight plans, milliseconds | Before p95 | After p95 |
| --- | ---: | ---: |
| Channels | 0.669 | 0.779 |
| Dense | 1.643 | 1.657 |

The channel after-run is about 0.109 ms higher and dense about 0.014 ms higher. These short runs on a shared Linux host do not isolate implementation cost from competing load and thermal/power state. The raw observations are retained without another run to seek a cleaner comparison. They establish no speedup, latency-tail, allocation, browser-runtime, editor-latency, presentation or endurance claim.

The [first review evidence](../review376/README.md) remains the immutable record for `4c71221`, including its full workspace run. The follow-up repeats affected checks and retains results for unchanged areas under the repository's bounded verification rule.

All eight [affected checks](checks.json) passed: formatting, workspace Clippy, 348 native board/query tests, 58 MCP tests, 345 WASI board/query tests, the browser check, harness Clippy and local Markdown links. The earlier complete workspace/harness/script/dependency results remain recorded for their unchanged areas; these affected checks do not claim a second full local workspace run.
