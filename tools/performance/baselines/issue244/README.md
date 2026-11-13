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
BVHs, hash grids, R*-trees and packed Hilbert trees. Each case has 20 construction
samples and 1,280 query samples per candidate. All candidates must equal exhaustive
inclusive-envelope enumeration and retain every analytic-geometry neighbor.

These are bounds simulations. Intent tags do not measure a connectivity graph,
ERC or routing. Circular arcs are analytic test geometry, not native curve support.
The grid settings are fixed comparison configurations, not exhaustive family tuning.

| Layout | Global packed p95 query (µs) | Layered packed p95 query (µs) |
| --- | ---: | ---: |
| straight | 0.481 | 0.491 |
| diagonal | 20.368 | 1.302 |
| curves | 17.233 | 1.202 |
| crowded_curves | 35.787 | 1.322 |
| mixed | 19.226 | 1.263 |

Layer-aware packing was the balanced choice for these workloads, rather than the
fastest method in every row. A layered hash grid won the diagonal query case but
used more payload storage and took longer to build. The layered BVH also won that
query case narrowly, with higher construction/storage costs. Layered scans build
more cheaply and can suit one-shot work. Reuse is important to amortize tree construction.

Eight-piece curve/diagonal indexing reduces candidate counts, but in the crowded
curve case raises packed payload from about 0.67 MB to 2.10 MB and construction
from about 0.68 ms to 2.02 ms, without improving query time. It is not implemented
in production; future curve features must measure their own exact predicate costs.
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
The final diagnostic-only changes received affected example/Clippy checks.

`final-source.zip` contains the implementation/diagnostic overlay on base
`de281fd630212e7198e85df315e38e5db3f7b640`; `final-source.json` records source and
executable hashes. The final stress reports use that diagnostic source. Native
before/after executable hashes are in their environment records. Earlier exploratory
runs, rejected old-binary runs and local binaries remain under `setup/issue244/`;
they are not used as final selection evidence. Environment/power limitations are
recorded explicitly. Physical presentation, endurance and full large-net routing
remain unqualified.

Reproduce with the engine examples and simulator commands in
[the performance guide](../../../../docs/performance-testing.md). Reuse the saved
native fixture bytes and the retained synthetic inputs rather than regenerating
identities for a comparison. See [failure review](failure-review.md) for ownership
and cancellation boundaries. Packaging this directory collected no new benchmarks.
