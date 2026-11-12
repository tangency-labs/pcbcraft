# Performance testing for #237

Agents implementing the ERC/routing work in [#237](https://github.com/tangency-labs/pcbcraft/issues/237)
should use this harness before and after changing a measured path. It is the first
measurement slice of [#240](https://github.com/tangency-labs/pcbcraft/issues/240),
which remains open for the unmeasured work below. Timing results describe one
machine and build; they are evidence for choosing work, rather than portable CI
pass/fail limits.

## What the harness measures

Enable `pcbcraft-app/performance-harness` to build the diagnostics. Ordinary
application builds exclude them. Both native and browser runners execute the same
engine operations and `PcbApp::frame`, using the existing Craft chrome and board
painting code. They do not create an eframe window or submit pixels to a GPU.

| Boundary | Included work |
| --- | --- |
| `decode.cold` | Unpack, decode and validate a fresh production archive |
| `connectivity.cold` / `connectivity.cached` | First lazy schematic connectivity construction / subsequent access |
| `clearance.prepare` | Snapshot query preparation, including board copper connectivity and contours |
| `clearance.pairs` | Current sweep candidate enumeration and represented-contour decisions |
| `clearance.candidates.empty` / `.crowded` | Candidate enumeration in empty space / through the diagonal copper region |
| `clearance.probe.empty` / `.crowded` | Complete disposable segment probes against an already prepared query |
| `drc.synchronous` | Complete current synchronous DRC, including its own preparation |
| `command.move_tracks` | Complete shared command, including validation, connectivity and history publication |
| `replay.<step>.ui` | Raw egui input processing through real editor layout and shape generation |
| `replay.<step>.tessellate` | CPU tessellation of those shapes; no renderer submission |
| `replay.shared-edit.command` | Shared engine edit while a real route draft is pending; excludes MCP transport |

Cold engine samples decode a fresh project each time; the executable/WASM module
stays loaded, so this means cold model caches rather than cold process startup.
Replay creates a fresh app
and warms fonts/layout for eight frames before measuring the trace. Logical input
time advances at 60 Hz without sleeping; this measures CPU service cost and does
not measure event-loop scheduling, dropped frames or actual refresh cadence.
Each boundary retains all samples in milliseconds and nearest-rank p50/p95/p99/max.
Use at least 100 samples for baseline evidence and preferably 1,000 for a useful
p99 tail. One-sample runs are correctness/build smoke tests.

## Generate and retain fixtures

Run from the repository root using the pinned toolchain/dependencies. The actual
shipping profile is `[profile.release]` in `Cargo.toml`, currently size optimized;
using a different optimization profile changes the question being measured.

```sh
cargo build --locked --release -p pcbcraft-app --features performance-harness --example board_perf
# Substitute the executable's actual target directory if CARGO_TARGET_DIR is set.
target/release/examples/board_perf generate reference-v1 "$(git rev-parse HEAD)" /tmp/reference-v1.json
target/release/examples/board_perf generate dense-v1 "$(git rev-parse HEAD)" /tmp/dense-v1.json
target/release/examples/board_perf generate capacity-v1 "$(git rev-parse HEAD)" /tmp/capacity-v1.json
```

A small canonical artifact is also checked in at
`tools/performance/fixtures/reference-v1.json`; reuse that file for quick shared
checks and comparisons with the [initial evidence](performance-baseline.md).
Generating a fresh artifact is useful for other workloads, but cannot be compared
to those saved reports because its document identities differ.

The generator refuses to overwrite files. It uses shared commands, original
bundled resistor/through-hole definitions and the production `.pc` encoder, with
accepted schematic membership before adding copper. The JSON envelope holds the
ordinary archive bytes, explicit project/board address, recipe revision, geometry
seed, generator revision, project SHA-256 and pointer/key trace. It is a diagnostic
artifact, not a new native project format. Do not hand-edit the embedded project.

| Recipe | Symbols / footprints | Straight tracks | Through vias | Purpose |
| --- | ---: | ---: | ---: | --- |
| `reference-v1` | 8 | 60 | 0 | Fast harness and comparison reference |
| `dense-v1` | 128 | 1,008 | 112 | Mixed pad sizes, crowded long diagonals and alternating layers |
| `capacity-v1` | 512 | 4,080 | 2,032 | Near current 4,096-track / 2,048-via limits |

Geometry follows fixed integer patterns with recipe seed `237240`. Production
commands allocate UUIDs, so regenerating a recipe does **not** recreate identical
native bytes. Retain and reuse the exact artifact across revisions and runtimes;
reports hash the entire envelope, including the trace. Changing recipes, seeds,
trace semantics or measurements requires a version change and a new baseline.
Fixtures are synthetic workloads, not manufacturable reference designs or BGA
capacity claims. Dedicated BGA, branching/narrow-channel, hierarchy/ERC and curved
routing recipes remain work for #240 and their owning feature issues.

`route-reverse-layer-cancel-edit-v1` arms X, starts a route in empty board space,
reverses pointer direction repeatedly, changes copper layer with PgDn, cancels
with Escape, starts another route and injects a shared command before exiting.
Every frame checks whether a draft should exist. Additional checks require the
layer transition, byte-identical authored state after cancellation, one external
edit and an inactive tool at completion. Missed canvas input fails the run instead
of producing misleading fast samples. Shared edits exercise session revision
invalidation, not concurrent transport or worker completion ordering.

## Record native evidence

Create environment metadata using observed facts. For example:

```json
{
  "revision": "full Git hash, with dirty diff hash if applicable",
  "hardware": "CPU model; RAM; host/VM; relevant GPU/driver when measuring presentation",
  "os": "OS release and kernel; Wayland/X11 for actual window tests",
  "runtime": "native CPU replay",
  "compiler": "full rustc -Vv output",
  "profile": "release",
  "power": "AC/battery; governor; thermal/power mode; background load",
  "notes": "test date, affinity, unknown conditions and scope limitations"
}
```

```sh
target/release/examples/board_perf run /tmp/reference-v1.json /tmp/native-env.json 100 /tmp/before.json
# Repeat from the candidate checkout, consuming the same artifact and metadata
# except for revision/notes. Output files must not exist.
target/release/examples/board_perf run /tmp/reference-v1.json /tmp/native-env-after.json 100 /tmp/after.json
python3 scripts/compare-performance.py /tmp/before.json /tmp/after.json > /tmp/comparison.md
```

Also retain `git diff --binary`, `Cargo.lock`, compiler/target versions, executable
hash, commands and raw reports alongside the artifact. For measured changes,
alternate before/after runs on an otherwise idle machine to reveal drift. Keep
cold and warmed samples separate. Use OS profilers for allocator counts and peak
resident memory; the harness does not report invented zeros for these metrics.

The comparator rejects changed fixture/trace bytes, target, sample counts,
measurement IDs, warmup, geometry or environment. Revision and notes may differ.
Keep a stable binary identity in notes, rather than changing the `runtime` label
between comparable native runs. Use runtime for the runtime version/mode. Native
and browser reports are separate baselines and cannot be compared as one series.
If a feature needs a new boundary, apply the same measurement-only instrumentation
to both revisions and document that patch; do not silently compare different code.

## Record browser evidence

The browser diagnostic is a separate opt-in WebAssembly example, without changing
the shipping web launcher. Use the `wasm-bindgen` CLI version matching Cargo.lock.
It needs a real browser Window, not Node/WASI.

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked --release -p pcbcraft-app --features performance-harness --example board_perf_web --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir tools/performance/pkg target/wasm32-unknown-unknown/release/examples/board_perf_web.wasm
# Match the shipping Trunk postprocessing (data-wasm-opt="z"); record tool versions.
wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int tools/performance/pkg/board_perf_web_bg.wasm -o tools/performance/pkg/optimized.wasm
mv tools/performance/pkg/optimized.wasm tools/performance/pkg/board_perf_web_bg.wasm
cp /tmp/reference-v1.json tools/performance/fixture.json
cp /tmp/browser-env.json tools/performance/environment.json
python3 -m http.server 8782 --bind 127.0.0.1 --directory tools/performance
```

Visit `http://127.0.0.1:8782/?artifact=fixture.json&environment=environment.json&iterations=100`.
Save the displayed JSON, or obtain `globalThis.performanceReport` through browser
automation; `globalThis.performanceError` signals a failed run. Record browser
name/full version, headless/headed mode, flags, timer precision/isolation settings,
OS/hardware/power conditions, WASM/glue hashes and build commands. Browser timer
resolution can quantize very short samples to zero, so retain those raw samples
and avoid percentage conclusions when the baseline is zero. Use a fresh tab for
cold module startup studies; module download/startup is outside these boundaries.
The synchronous diagnostic occupies its tab while running and is not an example
of responsive background execution.

## Agent acceptance instructions

Before implementing #243, #244, #246, #247, ERC or related #237 work, read the
owning issue and this guide, then record a baseline for the affected fixtures on
available native and browser targets. Preserve the saved artifacts for the PR.
Run the same suite afterward and explain both improvements and regressions.
Correctness assertions are mandatory even when timing gets better: compare query
findings with the exhaustive reference, reject stale authority and retain the
feature's own cancellation/rollback tests. The harness checks current query
contact/violation completeness against all pairs outside the timed boundary on
the first iteration; it does not replace a feature's exhaustive correctness suite.

The proposed #237 targets are routing-input p95 < 4 ms, input-to-valid-visible-preview
p95 < 16.7 ms / p99 < 33 ms, cancellation by the next rendered frame and commit
p95 < 100 ms with responsive preparation. The comparator lists CPU frames whose
p95 reaches 4 ms as candidates for investigation. A CPU frame below that value
cannot certify the window's input budget or either visible-preview budget.
`command.move_tracks` can expose an expensive validated edit, but is not proof
that a future walkaround/shove commit meets its target.

For each budget, report **measured pass**, **measured miss** or **unmeasured**, with
the actual boundary, fixture, profile, sample count and tail distributions. Do not
turn missing measurements or unsupported features into passes. A useful PR report
contains the before/after revisions and artifact hashes, environment, comparison,
correctness results, identified misses and remaining gaps. Explain why the measured
operation represents the changed feature. Keep #237 and #240 open for incomplete
acceptance work.

When adding jobs (#243), extend the harness with queue length, coalesced/replaced
requests, preparation/publication time, cancellation acknowledgement and stale
completion counts. Exercise continuous input, cancellation, layer/rule/board/opening
changes, overlapping MCP edits, queue saturation, worker failure and shutdown.
When adding an index (#244), retain the current exhaustive oracle and measure cold
build, ordinary updates and disposable overlay queries using identical artifacts;
record memory and allocation costs before selecting an implementation. Walkaround
and shove need their own valid-preview/commit traces, reversal and rollback cases.
ERC needs symbol/hierarchy-limit recipes and progress/cancel coverage. None of
these future capabilities is supplied by this first harness slice.

Actual end-to-end claims require a rendered native/browser window and instrumentation
at input receipt, valid preview acceptance, frame submission and presentation, with
clock domains explained. Add GPU/driver/renderer metadata and presentation/frame
traces. Cover Windows, macOS and Linux where available; identify unavailable systems
explicitly. On Linux read the workspace record
`setup/2026-11-11-wayland-freeze.md` before native performance/endurance work. It is
outside this Git repo, under the PCBcraft workspace root. Include long idle,
occlusion/workspace switching, lock/unlock, display sleep/wake and monitor changes,
with a matched X11 run where available. Short CPU replays do not investigate or
resolve the reported overnight Wayland redraw stall.

## Harness verification

```sh
cargo test --locked -p pcbcraft-app --features performance-harness performance::
cargo clippy --locked -p pcbcraft-app --features performance-harness --all-targets -- -D warnings
cargo check --locked -p pcbcraft-app --features performance-harness --example board_perf_web --target wasm32-unknown-unknown
python3 -m unittest discover -s scripts -p test_compare_performance.py
node --no-warnings --experimental-vm-modules --test scripts/test-performance-page.cjs
```

Run the repository's ordinary required checks as well. CI smoke tests establish
artifact/replay correctness; shared-runner timings are not accepted performance
baselines and no machine-dependent wall-clock threshold gates CI.
