# Walkaround solver evidence (#246)

The implementation is `343e04a`, based on `b19753b`. It adds the engine walkaround
solver and the read-only `pcb.plan_route` command described in
[walkaround route planning](../../../../docs/route-planning.md). The editor route tool
is unchanged, so no rendered replay or presentation measurement applies to this slice.
The [failure review](failure-review.md) lists the cases checked and how.

## Inputs and environment

The saved command-generated fixtures were reused unchanged:

| Fixture | Uncompressed artifact SHA-256 |
| --- | --- |
| `channels-v1` | `2bcba0154913f8e396b975f470040edc649342e4ed40bbe77af107c5ae2f0efa` |
| `dense-v1` | `df1731d2e45075cf29c595d9e8c997bb284bbcc51e144ae99d49b0a25eec55da` |

Both revisions were built with rustc 1.100.0 in the shipping release profile on an
Apple M5 Max (macOS 27.2.1, AC power, ordinary desktop load). The
[before](environment-before.json) and [after](environment-after.json) environments
differ only in revision. Before/after runs were alternated per fixture.

```sh
cargo build --locked --release -p pcbcraft-app --features performance-harness --example board_perf
board_perf run FIXTURE environment.json 20 run.json
board_perf checks FIXTURE environment.json 20 checks.json
board_perf walkaround FIXTURE environment.json 100 walkaround.json   # after only
python3 scripts/compare-performance.py run-before.json run-after.json
```

| Executable | SHA-256 |
| --- | --- |
| `board_perf` at `b19753b` | `8a304e9a6f7c7482e0d6e516ee97731ebffadddd76036e845cb19abc345ce2f6` |
| `board_perf` at `343e04a` | `0878b00fbfdeb77df4274961ad6b49ca27670f8999cd24e15d7801f0da28e9d8` |

## Existing boundaries

The only change on a measured existing path is DRC's outline collection, moved into a
shared helper. The official comparator accepted both pairs
([channels](comparison-run-channels-v1.md), [dense](comparison-run-dense-v1.md)).
`drc.synchronous` p95 was 2.793 → 2.623 ms (channels) and 8.499 → 7.952 ms (dense);
checker `engine_total` p95 was 1.599 → 1.497 ms and 4.461 → 4.888 ms. Other boundaries
moved in both directions by similar margins, including unchanged code such as
`electrical_connectivity.repeat` (+23%) and `clearance.prepare` (+10.6% / −20.5%).
These 20-sample runs do not isolate a cause for those differences and support no
speed claim; the checker reports have no comparator and are retained raw.

## Walkaround solver

The new `walkaround` diagnostic has no before counterpart. Each of 100 iterations
captures one query and plans six airwires, a free-space corridor route and a route
across the crowded diagonal region; every iteration reproduced the first plans
exactly, and each first plan committed through `pcb.add_tracks` with no exhaustive
clearance or short decision about the new copper.

| Fixture | Case | Outcome | Probes | p50 ms | p95 ms | p99 ms | max ms |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| channels-v1 | six airwires (each) | reached | 2–3 | ≤ 0.0025 | ≤ 0.0054 | ≤ 0.0090 | 0.0114 |
| channels-v1 | channel | reached, 6 points | 35 | 0.0426 | 0.0513 | 0.0624 | 0.0640 |
| channels-v1 | crowded | blocked | 32 | 0.1024 | 0.1104 | 0.1138 | 0.1154 |
| channels-v1 | all eight | | | 0.1592 | 0.1793 | 0.2043 | 0.2073 |
| dense-v1 | six airwires (each) | reached | 2–3 | ≤ 0.0053 | ≤ 0.0090 | ≤ 0.0139 | 0.0216 |
| dense-v1 | channel | reached | 2 | 0.0016 | 0.0018 | 0.0018 | 0.0019 |
| dense-v1 | crowded | blocked, 9 points | 53 | 0.3130 | 0.3385 | 0.4029 | 0.4637 |
| dense-v1 | all eight | | | 0.3331 | 0.3724 | 0.4248 | 0.4859 |

Query preparation (`walkaround.prepare`) was 1.219 / 3.971 ms p95 and is shared with
other query consumers. Raw samples are in the compressed reports.

## Limits

These are solver service costs on one host. Route-tool preview and commit latency,
browser runtime, allocation and presentation are unmeasured: the tool does not call
the solver yet. The airwire cases in these fixtures are short and mostly unobstructed;
the crowded cases measure blocked searches rather than long successful detours.
