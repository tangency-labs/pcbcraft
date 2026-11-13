# Spatial query comparison for #244

The selected implementation uses `geo-index` 0.4.0 packed Hilbert R-trees, with
layer-local groups, a shared group for spanning copper and scans below 128 objects.
`rstar` 0.14.0 is retained only as a benchmark alternative. BVH and grid prototypes
are original comparison code. The crossover is a tuning choice, not a universal optimum.

## Selection and curve stress

The synthetic workloads have 64 copper layers, 4,096 tracks, 2,048 through vias,
8,192 pads and 4,000 intent tags. Vias span all layers without index duplication.
Each candidate receives the same envelopes, probes, source identities and required
result ordering. Final comparisons apply equivalent layer partitioning to scans,
BVHs, hash grids, R*-trees and packed Hilbert trees. The corrected comparison in
`review1/` rotates the strategy starting position on each iteration and reverses
order after a full cycle. Reports record `execution_order`; build samples share
its iteration index, while query arrays contain one probe block per iteration.
Each case has 20 construction samples and
1,280 query samples per candidate. All candidates must equal exhaustive inclusive-
envelope enumeration and retain every analytic-geometry neighbor. The earlier
`stress/` reports used a fixed strategy order and remain for audit; their timing
rankings are superseded. Absolute timing differences between passes cannot be
attributed to order alone, since power/thermal state and other activity were not
controlled.

These are bounds simulations. Intent tags do not measure a connectivity graph,
ERC or routing. Circular arcs are analytic test geometry, not native curve support.
The grid settings are fixed comparison configurations, not exhaustive family tuning.

| Layout | Global packed p95 query (µs) | Layered packed p95 query (µs) |
| --- | ---: | ---: |
| straight | 1.203 | 1.343 |
| diagonal | 32.363 | 3.198 |
| curves | 25.870 | 2.775 |
| crowded_curves | 57.479 | 2.595 |
| mixed | 29.357 | 2.825 |

Layer-aware packing remains a balanced choice for these workloads. In the corrected
pass it had the lowest p95 lookup among the compared layered methods in all five
base cases; that does not establish a universal ranking. For diagonals, layered
packed/hash/BVH lookups were 3.198/3.447/3.942 µs p95, with median construction
1.125/3.399/1.440 ms and payload 0.67/2.25/1.05 MB respectively. Layered scans build
more cheaply and can suit one-shot work. Reuse matters to amortize construction.

Eight-piece curve/diagonal indexing reduces candidate counts. For crowded curves,
packed payload rises from 0.67 MB to 2.10 MB and median construction from 1.088 ms
to 2.021 ms, while p95 lookup improves from 2.595 µs to 1.533 µs. The diagonal and
ordinary-curve split cases are slower than their unsplit layer-aware counterparts.
Subdivision is not implemented in production; its tradeoff depends on workload,
and future curve features must measure their own exact predicate costs.
Reported payload is not total heap usage; map overhead is excluded where stated,
and R*-tree payload is unavailable. The Valgrind record covers the whole comparison
process, including loading, all seven earlier candidates, oracles and export;
it is not isolated per-index allocation attribution or a latency baseline.

## Production checks and limits

The native same-process probe comparison alternates measurement order for 1,000
samples after eight warmups. It checks production candidates against an exhaustive
legacy-style scan; that scan omits input validation. On the saved channel fixture,
production p95 channel/crowded lookup was 0.641/2.865 µs versus 7.634/8.145 µs for
the scan. Repeated captured-query preparation was 1.463 µs p95. Cold preparation
is recorded separately and still includes connectivity/contour work.

The separate ten-iteration before/after CPU replay was slower in several changed
and unchanged boundaries. Both reports and comparisons are retained; the cause
is not isolated, and this change makes no whole-editor speed or presentation claim.
The small replay cannot qualify latency tails. No broad run was repeated to seek
a favorable comparison; the focused same-process lookup experiment addressed the
specific attribution question.

Checks passed: 1,635 workspace tests (eight ignored), workspace Clippy, formatting,
12 opt-in harness tests, 18 Python tests, two Node tests, three candidate-example
self-checks, WASI board/query tests, browser compilation and dependency policy.
Chrome/WASM oracle smokes cover both saved reference and channel fixtures; these
are correctness runs, not browser before/after timing or GPU presentation evidence.
The first diagnostic-only changes received affected example/Clippy checks. The
review follow-up passed five example self-checks (including two focused layer and
schedule regressions), affected-example Clippy and formatting. Native extraction
smokes now preserve UUID layer membership: the channel fixture reports 168/338/72
objects with masks 1/2/3, and probes inherit those actual memberships. The reference
fixture has 72 objects on one represented layer. These one-iteration smokes establish
correct extraction, not native fixture latency rankings.

`final-source.zip` contains the implementation/diagnostic overlay on base
`de281fd630212e7198e85df315e38e5db3f7b640`; `final-source.json` records source and
executable hashes. The original stress reports use that diagnostic source. The
`review1/source.zip` and `review1/source.json` files record the corrected diagnostic
overlay on `36fdaf7`, its executable hash, input hashes and the single serial rerun.
Native before/after executable hashes are in their environment records. Earlier exploratory
runs, rejected old-binary runs and local binaries remain under `setup/issue244/`;
they are not used as final selection evidence. Environment/power limitations are
recorded explicitly. Physical presentation, endurance and full large-net routing
remain unqualified.

Reproduce with the engine examples and simulator commands in
[the performance guide](../../../../docs/performance-testing.md). Reuse the saved
native fixture bytes and the retained synthetic inputs rather than regenerating
identities for a comparison. See [failure review](failure-review.md) for ownership
and cancellation boundaries. The initial packaging collected no new benchmarks;
the review follow-up contains the bounded rerun described above.
