# Initial CPU performance evidence

These observations were collected on November 12, 2026 UTC (November 11 locally)
on the Linux host described in the raw reports: AMD Ryzen 9 7950X and roughly
62 GiB of reported RAM. Both runners used the repository's size-optimized release
profile; the browser module also used wasm-bindgen 0.2.130 and wasm-opt 123 `-Oz`,
matching the shipping postprocessing choice. The browser was headless Google
Chrome 156.0.8078.79 with GPU rendering disabled. Power, thermal state and
background activity were incompletely controlled, so these are initial
observations for selecting work, not a controlled improvement claim.

The [agent guide](performance-testing.md) explains boundaries and reuse. The
[source record](../tools/performance/baselines/source.json) identifies the exact
working-tree harness over main `627982ca7a186f2b5eeb28db0ba860302c4010ea`, and the
[build record](../tools/performance/baselines/build.json) retains executable/module
hashes, tool versions and commands. No optimization was made between these runs.
Native and browser columns are separate baselines, not a matched before/after
comparison. These measurements predate the net-registry and shared-length merges
in main `a022438`; rebasing the harness does not refresh the saved timing evidence.

## Observed p95 CPU costs

All entries below are milliseconds over 100 samples per boundary. Each replay
starts with eight untimed font/layout frames. Raw reports retain p50/p95/p99/max
and every sample; browser timer precision can quantize short measurements to zero.

| Boundary | Native reference | Browser reference | Native dense |
| --- | ---: | ---: | ---: |
| `clearance.prepare` | 0.420 | 0.600 | 44.036 |
| `clearance.pairs` | 0.091 | 0.200 | 6.063 |
| `clearance.probe.crowded` | 0.010 | 0.100 | 0.048 |
| `drc.synchronous` | 0.903 | 1.500 | 97.089 |
| `command.move_tracks` | 0.388 | 0.700 | 45.189 |
| `replay.reverse.3.ui` | 3.781 | 4.300 | 65.869 |
| `replay.shared-edit.ui` | 4.403 | 5.200 | 109.888 |

The reference has eight footprints and 60 tracks; the dense fixture has 128
footprints, 1,008 tracks and 112 vias. The full editor CPU boundary includes
input processing, layout and shape generation. These costs do not isolate routing
input handling or measure when a preview becomes visible.

The highest CPU frame p95 values were 7.042 ms for native reference
(`replay.layer.up.ui`), 5.200 ms for browser reference and 109.888 ms for native
dense (both `replay.shared-edit.ui`). Each exceeds the guide's 4 ms CPU-frame
investigation threshold, while the routing-input-only budget remains unmeasured.

## Capacity smoke and memory

The capacity recipe generated and replayed successfully with 512 footprints,
4,080 tracks and 2,032 vias. Only one iteration was collected; its report's equal
quantiles are a serialization of that observation, not estimates of tail latency.
In that iteration, query preparation took 1,119.1 ms, synchronous DRC took
2,132.9 ms and the validated move command took 1,073.7 ms. A larger sample set
remains needed before using this case as a latency baseline.

Whole-process maximum resident set sizes from `/usr/bin/time -v` were 26,380 kB
for reference, 58,500 kB for dense and 737,284 kB for capacity. These include
archive handling, exhaustive correctness checks, report storage and editor/font
state; they do not isolate query/index memory or allocation counts. Resource logs
are retained beside the raw reports.

## Budget status and reuse

Routing-input-only latency, input-to-valid-visible-preview latency, cancellation
by the next rendered frame and future routing commit latency are **unmeasured**.
CPU replay assertions do verify route state, a layer transition, cancellation
without authored changes and stale-draft invalidation after a shared edit. They
do not establish responsiveness of a window, background jobs, MCP transport,
walkaround or shove. No Windows/macOS or headed presentation run was available
in this collection. The reported Wayland endurance failure remains unresolved.

Canonical artifacts are checked in, including compressed dense/capacity envelopes:

```sh
gzip -dc tools/performance/fixtures/dense-v1.json.gz > /tmp/dense-v1.json
gzip -dc tools/performance/fixtures/capacity-v1.json.gz > /tmp/capacity-v1.json
# Use the native executable built by the agent guide, with current environment metadata.
target/release/examples/board_perf run tools/performance/fixtures/reference-v1.json /tmp/native-env.json 100 /tmp/current-reference.json
```

Retain these exact files for later comparisons; do not regenerate their random
identities or hand-edit the embedded native archive. Compare only reports with
matched environments and sample counts using `scripts/compare-performance.py`.
The checked-in observations can guide investigation, but their uncontrolled
conditions make a new matched before/after pair preferable for a performance claim.

Raw evidence:

- [Native reference](../tools/performance/baselines/native-reference.json)
- [Native dense](../tools/performance/baselines/native-dense.json)
- [Native capacity smoke](../tools/performance/baselines/native-capacity-smoke.json)
- [Browser reference](../tools/performance/baselines/browser-reference.json)

At this initial collection, #240 remained open for rendered-window timing,
broader workloads, allocation/queue measurements, cross-platform baselines and
endurance work. The subsequent [coverage record](performance-coverage.md)
describes the extended tooling and its observed limits without rewriting these
historical samples.
