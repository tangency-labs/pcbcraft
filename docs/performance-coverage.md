# Performance infrastructure coverage

This extends the [initial CPU baseline](performance-baseline.md) for
[#240](https://github.com/tangency-labs/pcbcraft/issues/240). The implementation
adds command-generated capacity/channel/hierarchy fixtures, real-window replay,
authenticated native MCP edits, GPU/canvas readback, process resources and strict
rendered comparisons. The later #243 work adds asynchronous DRC and copper preparation. The infrastructure itself does not implement ERC, an index,
walkaround, shove, BGA capacity or curved routing. Their measurements must accompany
their owning features rather than appear as fabricated baseline values here.

The saved [evidence directory](../tools/performance/baselines/extended/) contains
raw samples, environment metadata, immutable source overlays and hashes. These
are observations on one development host on November 12, 2026, with concurrent
development activity and unmeasured AC/thermal conditions. They are not a controlled
before/after optimization experiment. Follow the [guide](performance-testing.md)
to collect matched evidence for a change.

## Acceptance and remaining measurements

| Requirement | Implemented and exercised | Limit or follow-on owner |
| --- | --- | --- |
| Reproducible inputs | Original native-command/encoder artifacts, recipe/seed/revision, exact project/envelope hashes, versioned pointer/key trace | Production commands allocate UUIDs; reuse the saved bytes instead of regenerating comparison inputs |
| Current supported limits | Exactly 4,096 tracks / 2,048 vias; 4,096 schematic symbols / 64 sheets; explicit 63-link hierarchy | Full board-limit run is a smoke sample, not a tail baseline |
| Dense/crowded geometry | Existing dense diagonals/mixed pad widths, plus parallel narrow rails and attached branches; oracle-checked channel probe | High-pin-count BGA cases belong with #251–#253; radius/outline/offset traces with #259–#262 |
| Cold versus steady | Fresh model decode and lazy sheet connectivity separated from repeated access, preparation, queries, validated command publication and warmed replay | Whole-project first/repeat projection is not labeled as a cache hit; process/module startup is outside timings |
| Cancel/layer/reversal | Actual app assertions before each captured frame; cancellation preserves authored bytes; layer transition and inactive final tool required | Serialized screenshots pace inputs; this is not an uninterrupted OS event-stream benchmark |
| Concurrent edits | Native authenticated loopback MCP edit during an active route; exact revision increment and final draft invalidation verified | Browser uses the shared command, without a native transport endpoint |
| Painting/latency | Real eframe native and browser renderers; app UI, synthetic injection-to-UI, injection-to-readback and readback-delivery distributions | Physical input receipt, compositor presentation and input-to-photon latency remain **unmeasured** |
| Memory/allocations | Raw Linux process RSS/VmHWM samples and separate Valgrind whole-process allocation totals | Isolated index/query allocations need instrumentation in #244; profiler timings are not baselines |
| Comparisons | CPU and rendered comparators reject changed fixture, trace, environment, sample boundaries, renderer, viewport or window-state sequence | Keep native/browser and software/hardware series separate |
| Jobs/queues | #243 adds shared bounded native/browser checker and copper jobs, separate `checks`/`routes` CPU diagnostics, replacement/cancellation counters and publication timings; regression tests cover stale source/view/request authority | Physical cancellation-to-visible latency and isolated allocation attribution remain unmeasured; later index metrics belong with #244 |
| Later feature evidence | Reusable fixtures, raw distributions, query oracle, trace driver, resource collector and documented commands | ERC progress/cancel #47; walkaround/shove #245–#247; BGA and curves as above |

GPU readback proves that pixels were produced after a checked app pass. Its transfer,
software rendering and instrumentation costs prevent treating it as a substitute
for physical presentation. Actual input-to-visible-preview latency remains
unmeasured. Presentation qualification under #237 is separate from completion of
the reusable measurement infrastructure; closing the tooling work does not turn
this unmeasured boundary into a pass.

## Recorded platforms

| Platform/runtime | Result | Scope |
| --- | --- | --- |
| Linux native CPU, shipping release profile | Four 100-iteration reports plus one exact-board-limit smoke | Ryzen 9 7950X, about 62 GiB RAM, rustc 1.98.1; environment in every report |
| Linux native X11, isolated TigerVNC / lavapipe | One iteration, 44 captures and one real MCP edit | Software virtual-display smoke; observed host focus is false while synthetic input is focused |
| Same X11 window, two iterations with 2 s hold | 88 captures and two MCP edits | Verifies reset/hold mechanism over about five minutes; not endurance evidence |
| Linux native Wayland / RTX 4070 Ti | Corrected replay completed 100 iterations / 4,400 captures and 100 authenticated MCP edits | GNOME Shell 50.6, NVIDIA 615.78.08, Vulkan; raw samples and exact source/binary identity retained; earlier failed attempts kept separately |
| Same Wayland window, final two-iteration / 2 s hold smoke | 88 captures in 5.812 seconds | Corrected input retention and idle mechanism; not endurance evidence |
| Chrome 156.0.8078.79 headless, WebGL/SwiftShader | 100 iterations, 4,400 captures | Shipping `release` plus wasm-bindgen 0.2.130 and wasm-opt 123 `-Oz`; 1400×900 CSS pixels, DPR 1 |
| Chrome default WebGPU selection on this host | Diagnostic readback failed; no baseline accepted | Explicitly selecting WebGL does not qualify WebGPU |
| Windows and macOS | Unavailable | No executable, driver, latency or endurance claims |

The earlier native software smoke had roughly one-second event/readback-delivery delays.
Those observations are retained without attributing a cause or generalizing them
to ordinary hardware use. Initial native Wayland warmup also failed before the
launcher matched the shipping Linux titlebar/decorations. A later hardware smoke
completed in about 1.81 seconds, while its 100-iteration attempt stopped after the
`cancel.down` injection at 11.075 seconds. The sampled main thread was waiting in
the Wayland event loop. A later short replay also exposed the diagnostic clearing
pending synthetic clicks across eframe logic-only callbacks; the final runner
retains each batch until an accepted UI pass, with a regression test. The earlier
attempt is an observed loss of diagnostic progress, without establishing
compositor/driver/toolkit cause or identifying it with the earlier
overnight incident. `platform-status.json` records concurrent conditions. Stacks
and logs were preserved before ending only that diagnostic instance. No partial
timings were accepted as a baseline. After input retention was corrected, a fresh
100-iteration run completed in 244.983 seconds, with all 4,400 captures and 100
native MCP edits. This demonstrates the corrected diagnostic run, without
retrospectively establishing a cause for every earlier failure.

The same-window hold option supports longer collection, but the Wayland exposure, lock/unlock, sleep/wake and monitor transitions in the
workspace freeze record still need an actual endurance session.

## Selected observations and budgets

The table selects p95 values from the complete raw reports; all values are ms.
The one-iteration row is labeled separately because its quantiles are the same
single observation. DRC and preparation have no standalone budget in #237.

| Native CPU recipe | Iterations | Clearance preparation | Validated track edit | Synchronous DRC | `reverse.3` UI |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reference | 100 | 0.289 | 0.258 | 0.583 | 2.038 |
| Channels/branches | 100 | 6.972 | 7.224 | 15.412 | 17.508 |
| 64-sheet hierarchy | 100 | 1.089 | 2.766 | 2.823 | 17.663 |
| 4,096 symbols / 64 sheets | 100 | 0.299 | 15.339 | 0.584 | 1.979 |
| 4,096 tracks / 2,048 vias, **smoke** | 1 | 1,135.193 | 1,448.516 | 2,364.691 | 185.222 |

| Proposed #237 budget | Evidence and assessment |
| --- | --- |
| Routing input p95 < 4 ms | **Measured miss for the CPU service-cost proxy** on channel and hierarchy reversal rows; reference proxy passes. Native rendered `reverse.2.ui` p95/p99/max is 6.952 / 7.945 / 8.541 ms. Browser p95/p99/max is 4.000 / 5.100 / 6.800 ms, so both rendered boundaries also miss a strict < 4 ms threshold. These boundaries start at synthetic injection, not OS input receipt. |
| Valid visible preview p95 < 16.7 / p99 < 33 ms | **Unmeasured presentation budget.** Native `reverse.2.input_to_gpu_readback` p50/p95/p99/max is 27.207 / 38.427 / 41.585 / 56.388 ms; browser is 56.700 / 62.600 / 151.900 / 173.000 ms. Both readback boundaries exceed the numerical thresholds but cannot classify physical presentation. |
| Cancellation by next rendered frame | State assertions verify cancellation before requesting the corresponding image, with unchanged authored bytes; the physical presentation deadline is **unmeasured**. |
| Commit p95 < 100 ms | Existing validated track-edit proxy passes on the four 100-sample workloads. The exact board-limit smoke takes 1,448.516 ms and exposes a cost to investigate, but cannot establish a p95 or qualify future shove/walkaround commits. |
| Responsive preparation, bounded queues | Current synchronous preparation/DRC costs are retained above. Future job/index queue and stale-result budgets are **unmeasured** until those features exist. |

No budget is relaxed by this change. The reference process's observed peak sampled
RSS is 28,499,968 bytes and sampled VmHWM is 28,794,880 bytes. The separate
one-iteration Valgrind run reports 3,167,269 allocations, 3,159,113 frees and
419,523,893 cumulative allocated bytes. These include startup, model loading,
correctness oracles, fonts and reporting, rather than isolated geometry storage.

## Reproducing the retained evidence

Decompress reports to temporary files before using the comparators. For example:

```sh
gzip -dc tools/performance/baselines/extended/browser-gl-reference.json.gz > /tmp/browser-before.json
python3 -B scripts/compare-rendered-performance.py /tmp/browser-before.json /tmp/browser-after.json
gzip -dc tools/performance/fixtures/channels-v1.json.gz > /tmp/channels-v1.json
target/release/examples/board_perf run /tmp/channels-v1.json /tmp/environment.json 100 /tmp/channels-after.json
```

`manifest.json` hashes every retained evidence file. `rendered-source.json` and
`rendered-source.zip` identify the native/resource collection source: apply the
overlay to base commit `14bbda9ec1f8e4fc80b309010dae53f0b420447a` in a disposable
checkout. The initial projection names were `cold/cached`; their lack of a distinct
whole-project cache was corrected to `first/repeat` before collecting the five
`cpu-*.json.gz` reports. `cpu-source.json` and its zip identify that CPU executable
and its core inputs. Resource timings retain their historical labels and must not be
compared to the corrected CPU schema as though the boundary IDs were identical.

`browser-optimized-source.json` and its zip identify the later optimized browser
collection, including raw/optimized WASM and generated glue digests. Source
archives contain the exact instrumented files used during collection, rather than
pretending a later commit hash existed at measurement time. Final changes add
failure-path tests, stricter comparison/window-state checks, missing-counter
handling and browser startup deadlines; healthy measurements are retained as
observations of their recorded snapshots. Binaries and generated browser packages
are not checked in. Rebuild with the guide's locked release/profile commands;
record fresh hashes and raw outputs rather than expecting timings or randomized
build artifacts to be byte-identical.

The native launcher now uses an independent watchdog, renewed by accepted UI
progress or an explicit idle hold. Unlike an in-window timer, it can fail the
diagnostic process if the event loop stops calling the app; it writes no partial
report. Endurance investigations use `--preserve-on-stall` to retain the failed
instance for diagnostics, with no passing report even after recovery. Browser host
polling also detects missing UI progress independently of animation callbacks.
`native-final-source.json` / zip retain the earlier hardware smoke and
incomplete-run source before this additional failure handling.
`watchdog-source.json` / zip retain the later lost-input failure. The corrected
100-iteration native collection uses `accepted-input-source.json` / zip and its
explicit input-retention policy, which the comparator will not silently equate
with earlier policies. The browser's 100-iteration baseline keeps its own earlier
source/policy; final browser changes are checked separately by a smoke run.

Final failure-path changes keep the native watchdog armed until actual app
teardown, rather than disarming it when a result is merely queued. A lost window
close must not leave an otherwise completed diagnostic indefinitely running.

The failure-path review followed renderer callbacks, device polling, the MCP
client and live-control service, page startup and child-process shutdown. Tests
exercise lost/disconnected/blank captures, GPU polling errors, stalled/disconnected
MCP completion receivers, late renderer startup, missing process counters and
incomparable reports, watchdog expiry with no UI progress, input retention across
logic-only callbacks and cancellation on drop. Code review additionally covered
bounded socket I/O and queues, reply size, exact revision guards, callback cancellation, one capture in
flight, endpoint retirement and stopping only the collector's own child. No UI
callback joins a worker or waits on its socket.
