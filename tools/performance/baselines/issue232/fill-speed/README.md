# Supplied-demo fill delay

The reported delay reproduced on the saved three-resistor [demo archive](zone-fill.pc).
The supplied native launcher used an unoptimized development build. A bounded
three-sample diagnostic separated request capture, fill calculation and the first
and repeat ratsnest reads. Capture/connectivity took less than 0.1 ms combined in
the baseline, while calculation took about 5.9 seconds. Optimizing the original
code reduced calculation to 427–446 ms, so the build profile did not explain all
of the tiny board's cost.

A separately instrumented development trace attributed roughly 3.7 seconds to
thermal construction and 1.8 seconds to contact checks. Those operations repeatedly
rebuilt the same analytic material and holes. The optimization prepares immutable
operands once, retains already built Boolean/union output and uses SolidCore's
independent-body copy API for subsequent queries. It retains the existing
material-minus-holes recipe, tolerances, clearances and clipping algorithms.

| Calculation sample | Median (ms) | Range (ms) |
| --- | ---: | ---: |
| Original development build | 5,877.211 | 5,859.368–5,881.013 |
| Original optimized build | 428.011 | 427.479–446.356 |
| Intermediate development cache change | 1,281.262 | 1,277.248–1,313.296 |
| Completed cache change, optimized build | 105.841 | 105.439–109.141 |

The matched optimized-build observations are about 4.0 times faster on this
fixture. Moving the test launcher from the original development build to the
optimized app with this change combines the algorithm and profile improvements;
that roughly 55.5-times ratio is not an algorithm-only result. All samples retain
one zone/island, 470 triangles, zero airwires, unchanged encoded native documents
and unchanged revision. The geometry/fill regressions separately check analytic
areas, holes, clearance, contact styles and connectivity; matching triangle
counts alone are not a geometry proof.

The raw [before development](before-dev.json), [before optimized](before-release.json),
[intermediate development](after-dev.json), [after optimized](after-release.json)
and [comparison](comparison.json) reports retain every sample. The baseline
production code is 105c0f4 plus the diagnostic driver; [environment](environment.json)
and [after source fingerprints](after-sources.json) identify the builds. The
release profile uses opt-level s, thin LTO and 16 codegen units on both sides.
The intermediate development sample predates the empty-operand guard and
WASM-only test gating; the release pair measures the completed production path.
The instrumented trace and its source overlay remain in the workspace evidence;
its timings are not mixed into the optimized comparison.

This is a three-sample, single-board headless calculation check on a shared host.
It is not a tail baseline, physical input-to-visible latency measurement,
large-board throughput or independent KiCad comparison. No broader timing replay
was added after the targeted question was answered. Allocation/retained-topology
memory growth and native Wayland endurance remain unmeasured.

To reproduce the diagnostic with the same saved bytes:

```sh
cargo run --release --locked -p pcbcraft-engine --example zone_fill_perf -- tools/performance/baselines/issue232/fill-speed/zone-fill.pc /tmp/zone-fill-performance.json
```

[The failure review](failure-review.md) describes ownership, synchronization,
poisoning and transport checks. Native unit/integration tests pass 2,210 cases
(12 ignored); a documentation-only retry passes its one executable test after
shared-cache dependency loading failed in the initial workspace run. Workspace
all-target Clippy, the optimized native build and 19 WASI geometry tests pass.
Browser packaging and the actual remove/repour oracle are recorded alongside
this guide. The local launcher uses the optimized native app; the older builds
and profile are preserved.

The [rebuilt packaged browser](browser.json) passes actual menu remove/repour
pixel assertions with two successful worker completions and zero page errors.
These checks verify publication and display state; their scripted waits are
not a browser pour-latency measurement.
