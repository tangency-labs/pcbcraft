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

## Review revision (`038b30e1`)

The revision for the PR #446 reviews merges upstream main and changes the solver and
the diagnostic. In the solver, a route settles with the nets it touches, and probes
apply the width minimums of the net classes the route touches. Confirmation checks
island and same-net exemptions against the connectivity after the shove, and it
rechecks copper whose net membership the shove changes, moved or not. Off-lattice
legs enter hulls on the crossed edge, non-plated holes block before any electrical
exemption, and the board's track limit is checked. `pcb.shove_tracks` now runs in
the copper job's bounded steps. The `shove` diagnostic (report schema 2) now:

- compares each committed shove with its plan and with the connections that existed
  before;
- refuses any violating pair that is new after the shove;
- adds a command-generated pushing case;
- times the pushing case's copper job and its confirmed cancellation.

The results of earlier review runs, at `ff529625` and `6754a9f3`, were replaced. They
remain in the history at `6fc4d203` and `302dccc4`.

`board_perf` was rebuilt at `038b30e1` with rustc 1.100.0 in the release profile on
the same host; see the [environment](environment-review.json). Its SHA-256 is
`9cead69455e628a10f415cdb68a7b2e8e858edb5c06023473718d544d465b7ee`. The fixtures
are the same artifacts as above. The pushing case is generated on a copy of each
fixture through `pcb.add_tracks` and `pcb.add_via`: four parallel 0.25 mm tracks
0.45 mm apart, a via with a track ending on it, and a route along them at
y = 80.4 mm. New identities are random, so its project digest differs between runs;
each report records its own digest (`push_case.project_sha256`).

This run shared the host with heavy build and agent load: the load average was
about 51 right after it. The maxima below include outliers that also slowed
unrelated stages of the same iteration, so read the p50 and p95 values. The run was
not repeated to get a cleaner one.

```sh
cargo build --locked --release -p pcbcraft-app --features performance-harness --example board_perf
board_perf shove FIXTURE environment-review.json 100 shove-FIXTURE-review.json
```

Raw reports: [channels-v1](shove-channels-v1-review.json.gz),
[dense-v1](shove-dense-v1-review.json.gz). The run was 100 iterations per fixture.

| Fixture | Straight cases reached / blocked | Pushing case | Straight plans p50 / p95 | Pushing plan p50 / p95 |
| --- | ---: | --- | ---: | ---: |
| channels-v1 | 6 / 2 | reached, depth 4, 22 changed (5 reshaped, 16 added, 1 via), 91 probes | 0.845 / 1.039 ms | 0.295 / 0.357 ms |
| dense-v1 | 7 / 1 | reached, depth 4, 22 changed (5 reshaped, 16 added, 1 via), 91 probes | 1.321 / 1.611 ms | 0.375 / 0.448 ms |

In the first runs, one airwire case per fixture (`airwire.1` on channels-v1,
`airwire.3` on dense-v1) was blocked by its own target pad `footprint-5:1`. Both are
now reached, which is the effect of the route-membership correction. Both commits
pass the plan, connection and exhaustive clearance checks.

The pushing case's `pcb.shove_tracks` job was driven in 256-unit host batches. It
took 747–748 steps on channels-v1 and 1,497–1,499 on dense-v1; the count varies by a
step or two between iterations.

| Fixture | Longest uninterrupted step p50 / p95 / max | of which planning p50 / p95 / max | Confirmed cancel p50 / p95 / max | Whole job p50 |
| --- | ---: | ---: | ---: | ---: |
| channels-v1 | 0.230 / 0.950 / 33.591 ms | 0.138 / 0.206 / 1.231 ms | 0.117 / 0.157 / 2.235 ms | 4.514 ms |
| dense-v1 | 0.479 / 1.197 / 5.602 ms | 0.237 / 0.287 / 1.197 ms | 0.183 / 0.222 / 0.279 ms | 9.973 ms |

The 33.591 ms channels-v1 maximum came from one iteration. That iteration's whole job
took 39.585 ms against a 4.514 ms p50, so the time went to the host, not to one
large unit of work. In the quieter run at `6754a9f3`, the maxima were 0.459 ms and
0.716 ms.

The cancels were issued while the job reported planning, except in 6 iterations on
channels-v1 and 1 on dense-v1, where the job had finished before the cancel. Those
are reported as `finished` and not timed. Before this revision, the whole obstacle
preparation and search ran in one advance. That advance was not timed, but it
contained the full obstacle preparation, which alone takes 2.576 ms (channels-v1)
and 5.644 ms (dense-v1) at p50 here (`shove.push.prepare`).

These remain service costs on one loaded host. Route-tool preview and commit
latency, the browser runtime, allocations and presentation are still **unmeasured**.
