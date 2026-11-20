# Shove solver evidence (#247)

The implementation is `83bdf997`, based on `52601864`. It adds the engine shove solver,
the read-only `pcb.plan_shove` query and the `pcb.shove_tracks` edit described in
[shove](../../../../docs/route-shove.md). The route tool is unchanged, so no rendered
replay or presentation measurement applies to this slice.

## Inputs and environment

The saved command-generated fixtures were reused unchanged:

| Fixture | Uncompressed artifact SHA-256 |
| --- | --- |
| `channels-v1` | `2bcba0154913f8e396b975f470040edc649342e4ed40bbe77af107c5ae2f0efa` |
| `dense-v1` | `df1731d2e45075cf29c595d9e8c997bb284bbcc51e144ae99d49b0a25eec55da` |

`board_perf` was built with rustc 1.100.0 in the release profile on an Apple M5 Max
(macOS 27.2.1, AC power) while other builds ran on the host; see the
[environment](environment.json). Its SHA-256 is
`ca56f22fbc5236cdb0c1b116010517c56facb486d2efc29f6daadc7a70315516`.

```sh
cargo build --locked --release -p pcbcraft-app --features performance-harness --example board_perf
board_perf shove FIXTURE environment.json 100 shove.json
```

## Existing boundaries

No existing measured path changed. The clearance overlay gained a membership probe
that only the shove uses, the walkaround's width selection moved into a shared helper
on the command path, and the hull gained functions the walkaround does not call. No
before/after replay was recorded for this reason.

## Shove diagnostic

The new `shove` diagnostic has no before counterpart. Over 100 iterations it plans a
straight two-point shove along each walkaround case on one captured query, requires
identical plans on every iteration and, on the first iteration, commits each reached
plan through `pcb.shove_tracks` against the exhaustive pair reference. Raw reports:
[channels-v1](shove-channels-v1.json.gz), [dense-v1](shove-dense-v1.json.gz).

| Fixture | Reached | Blocked (prefix by bisection) | Most probes | All plans p50 / p95 |
| --- | ---: | ---: | ---: | ---: |
| channels-v1 | 5 | 3 | 39 | 0.762 / 26.501 ms |
| dense-v1 | 6 | 2 | 39 | 1.282 / 36.442 ms |

No case in these fixtures pushes copper: every straight line either runs clear or
meets a pad or a track that cannot move (it ends inside the pusher's hull or is joined
in mid-span), so the reports measure clear shoves and blocked shoves with their prefix
bisection, not propagation. Pushing and propagation are exercised by the engine's
command-built fixtures, which are not timed. The p95 values were taken under other
build load on the host and support no latency claim; all-plan p50 values are below 1.3 ms.

These are solver service costs on one host. Route-tool preview and commit latency,
the browser runtime, allocations and presentation remain **unmeasured**.

## Review revision (`ff529625`)

The revision for the PR #446 review merges upstream main and changes the solver and
the diagnostic: a route settles with the nets it touches, probes apply contacted
net-class width minimums, confirmation checks island exemptions against the
connectivity after the shove, off-lattice legs enter hulls on the crossed edge, and
`pcb.shove_tracks` runs in the copper job's bounded steps. The `shove` diagnostic
(report schema 2) now compares each committed shove with its plan and with the
connections that existed before, adds a command-generated pushing case, and times
the pushing case's copper job and its cancellation.

`board_perf` was rebuilt at `ff529625` with rustc 1.100.0 in the release profile on
the same host, again under other build and agent load; see the
[environment](environment-review.json). Its SHA-256 is
`7643603e686f46f43aec4d929479df801eecdde5d8ab15768457c24e3a3371de`. The fixtures
are the same artifacts as above. The pushing case is generated on a copy of each
fixture through `pcb.add_tracks` and `pcb.add_via` (four parallel 0.25 mm tracks
0.45 mm apart, a via with a track ending on it, a route along them at y = 80.4 mm);
its project digests are recorded in each report (`push_case.project_sha256`):
`a32cac83…a48cc` (channels-v1) and `9225ae92…1b574` (dense-v1).

```sh
cargo build --locked --release -p pcbcraft-app --features performance-harness --example board_perf
board_perf shove FIXTURE environment-review.json 100 shove-FIXTURE-review.json
```

Raw reports: [channels-v1](shove-channels-v1-review.json.gz),
[dense-v1](shove-dense-v1-review.json.gz). The run was 100 iterations per fixture
and was not repeated.

| Fixture | Straight cases reached / blocked | Pushing case | Straight plans p50 / p95 | Pushing plan p50 / p95 |
| --- | ---: | --- | ---: | ---: |
| channels-v1 | 6 / 2 | reached, depth 4, 22 changed (5 reshaped, 16 added, 1 via), 91 probes | 0.548 / 0.585 ms | 0.207 / 0.224 ms |
| dense-v1 | 7 / 1 | reached, depth 4, 22 changed (5 reshaped, 16 added, 1 via), 91 probes | 0.818 / 0.888 ms | 0.261 / 0.283 ms |

One airwire case per fixture (`airwire.1` on channels-v1, `airwire.3` on dense-v1)
was blocked by its own target pad `footprint-5:1` in the first runs and is now
reached: that is the route-membership correction. Both commits pass the plan,
connection and exhaustive clearance checks.

The pushing case's `pcb.shove_tracks` job, driven in 256-unit host batches
(748 steps on channels-v1, 1,505 on dense-v1, the same on every iteration):

| Fixture | Longest uninterrupted step p50 / p95 / max | of which planning | Cancel acknowledged p50 / p95 / max | Whole job p50 |
| --- | ---: | ---: | ---: | ---: |
| channels-v1 | 0.180 / 0.194 / 0.383 ms | 0.097 / 0.109 / 0.150 ms | 0.081 / 0.090 / 0.135 ms | 3.378 ms |
| dense-v1 | 0.334 / 0.365 / 0.536 ms | 0.164 / 0.176 / 0.231 ms | 0.133 / 0.145 / 0.173 ms | 7.137 ms |

The longest steps are obstacle-preparation batches, which the walkaround planner
shares. Every one of the 200 cancels was issued while the job reported planning, and
the snapshot worker retired it within 0.173 ms. Before this revision the whole
obstacle preparation and search ran in one advance; that advance was not timed, but it
contained the full obstacle preparation, which alone takes 1.929 ms (channels-v1) and
4.137 ms (dense-v1) at p50 here (`shove.push.prepare`). The browser's incremental
host yields after 2 ms of batches, so no measured step exceeds one slice.

These remain service costs on one loaded host. Route-tool preview and commit
latency, the browser runtime, allocations and presentation are still **unmeasured**.
