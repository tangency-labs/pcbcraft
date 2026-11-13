# Performance testing for #237

Agents implementing the ERC/routing work in [#237](https://github.com/tangency-labs/pcbcraft/issues/237)
should use this harness before and after changing a measured path. The
[#240](https://github.com/tangency-labs/pcbcraft/issues/240) infrastructure covers
CPU replay, actual rendered windows, authenticated native MCP edits and process
resources. The [coverage record](performance-coverage.md) distinguishes exercised
boundaries from feature-dependent or unavailable measurements. Timing results
describe one machine and build; they are evidence for choosing work, rather than
portable CI pass/fail limits.

## Choose a runner

For changes to the paths below, collect a baseline before implementation and
repeat the affected workloads afterward with identical saved inputs and matched
environments. The [AGENTS.md rule](../AGENTS.md#performance-evidence) and
[contribution guide](../CONTRIBUTING.md#build-and-find-your-way-around) make this
part of the development workflow even after #240 closes.

Before collecting evidence, apply the [verification stopping rule](../AGENTS.md#keep-verification-bounded).
State which decision the measurement will inform, the boundary it must isolate,
the smallest suitable workload and the stopping condition. For example, routing
job capture, cancellation and publication need measurements of those operations;
a whole-editor replay supplies integration context but cannot alone attribute a
slowdown to the job scheduler.

Run the selected before/after pair, then assess whether it answered that question.
Noise or a comparison rejected for differing environment or window state is a
limitation to preserve, rather than a reason to repeat the broad run until it
passes. Further measurement needs a specific hypothesis and a test shaped to
resolve it. Scope extensive statistical baselines, allocation profiles and
endurance runs as qualification work instead of automatically adding them to
routine feature verification.

| Changed path or question | Tool and instructions |
| --- | --- |
| Geometry queries, connectivity, DRC or validated engine commands | `board_perf` with the [CPU collection and comparison commands](#record-native-evidence); the CPU example also has a [browser runner](#record-browser-evidence) |
| Editor input, routing interaction, painting or live MCP edits | `board_perf_window` and browser `rendered.html`, with [rendered replay and comparison](#rendered-native-and-browser-replay) |
| Memory/allocation costs, including a new index or worker implementation | Add [process-resource collection](#extended-workloads-and-process-resources) to the affected workload; keep profiler timings separate |
| Native idle/exposure behavior or the recorded Wayland stall | Use the [idle and presentation diagnostics](#idle-and-presentation-diagnostics) and preserve failed instances for investigation |
| Harness correctness or ordinary CI verification | Run the short [harness verification](#harness-verification); it validates replay, failure paths and saved evidence without collecting full GPU baselines |

Saved inputs live under `tools/performance/fixtures/`, and recorded results and
source fingerprints under `tools/performance/baselines/`. Select relevant recipes
from the tables below and report raw before/after samples, their comparison,
correctness results and limitations in the PR. New feature-dependent boundaries
must be added with their owning implementations. Full rendered baselines,
allocation profiling and endurance sessions are separate targeted runs; routine
CI and unrelated/documentation-only changes do not collect them.

## What the harness measures

For the #244 candidate comparison, run the engine-only example against a saved
fixture, or generate separate synthetic bounds stress cases:

```sh
cargo run --locked --release -p pcbcraft-engine --example spatial_index -- tools/performance/fixtures/reference-v1.json 100 /tmp/index-reference-new.json
python3 scripts/simulate-spatial-obstacles.py /tmp/index-simulations-new
target/release/examples/spatial_index /tmp/index-simulations-new/mixed.json 20 /tmp/index-mixed-new.json
```

The simulator covers 64 copper layers, 4,096 tracks, 2,048 through vias, 8,192 pads
and 4,000 synthetic intent tags. Analytic circular arcs and long diagonals stress
conservative envelopes, including crowded curves and optional eight-piece bounds.
Reports retain construction, query and replacement-query samples, candidate counts
and payload sizes where available. Strategy order rotates per iteration and reverses
per full cycle; `execution_order` records the order associated with raw sample
indices. Native fixture envelopes and default probes preserve actual copper-layer
membership. These measure indexing with layer filtering
and deduplication, not routing, connectivity, ERC or physical presentation. Intent
tags describe the fixture; the bounds lookup does not solve the electrical graph.
See the [retained comparison](../tools/performance/baselines/issue244/README.md).

Enable `pcbcraft-app/performance-harness` to build the diagnostics. Ordinary
application builds exclude them. Both native and browser CPU runners execute the same
engine operations and `PcbApp::frame`, using the existing Craft chrome and board
painting code. They do not create an eframe window or submit pixels to a GPU.

| Boundary | Included work |
| --- | --- |
| `decode.cold` | Unpack, decode and validate a fresh production archive |
| `connectivity.cold` / `connectivity.cached` | First lazy schematic connectivity construction / subsequent access |
| `electrical_connectivity.first` / `.repeat` | First/repeated whole-project connectivity projection after sheet access; may clone a named graph or rebuild unnamed connectivity, so neither implies a separate project-cache hit |
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
For a statistical latency baseline, use at least 100 samples and preferably
1,000 for a useful p99 tail. That sample guidance does not make a full statistical
baseline mandatory for every change. A smaller targeted run can verify correctness
and expose service costs; label its scope and avoid tail-latency conclusions that
its sample count cannot support. One-sample runs are correctness/build smoke tests.

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
capacity claims. The extended recipes below add branching/channel and current
hierarchy-limit workloads. BGA, ERC progress and curved-routing cases still need
their owning feature implementations.

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
the first iteration. Every disposable probe also compares contact, clearance and
conflicting-membership findings against an exhaustive shared-layer contour and
membership reference outside its timed boundary. These guards do not replace a
feature's exhaustive correctness suite.

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
operation represents the changed feature. Keep the owning feature or qualification
issue open for its incomplete acceptance work; completing the reusable harness
does not qualify unimplemented features.

When adding jobs (#243), extend the harness with queue length, coalesced/replaced
requests, preparation/publication time, cancellation acknowledgement and stale
completion counts. Exercise continuous input, cancellation, layer/rule/board/opening
changes, overlapping MCP edits, queue saturation, worker failure and shutdown.
When adding an index (#244), retain the current exhaustive oracle and measure cold
build, ordinary updates and disposable overlay queries using identical artifacts;
record memory and allocation costs before selecting an implementation. Walkaround
and shove need their own valid-preview/commit traces, reversal and rollback cases.
ERC can reuse the symbol/hierarchy-limit recipes added below, but still needs
progress/cancel coverage with its implementation. Worker and indexed-query
capabilities are not supplied by this harness.

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
python3 -B -m unittest discover -s scripts -p 'test*performance*.py'
node --no-warnings --experimental-vm-modules --test scripts/test-performance-page.cjs scripts/test-rendered-performance-page.cjs
```

Run the repository's ordinary required checks as well. CI smoke tests establish
artifact/replay correctness; shared-runner timings are not accepted performance
baselines and no machine-dependent wall-clock threshold gates CI.

## Rendered native and browser replay

`board_perf_window` and the browser's `rendered.html` open the production editor
through eframe and submit its geometry to the GPU. They consume the same saved
artifact and assert route start, reversals, copper-layer change, cancellation
without authored changes, stale-draft invalidation and an inactive tool at exit.
Native replay sends the shared edit through a real authenticated loopback MCP
connection while the window continues painting. Browser replay uses the shared
engine command because the browser has no native MCP endpoint.

```sh
cargo build --locked --release -p pcbcraft-app --features performance-harness --example board_perf_window
# ENVIRONMENT_JSON must identify the window system, GPU/driver, compositor and power conditions.
target/release/examples/board_perf_window tools/performance/fixtures/reference-v1.json /tmp/window-env.json 100 /tmp/window-report.json
python3 scripts/compare-rendered-performance.py /tmp/window-before.json /tmp/window-report.json
```

The diagnostic window uses fresh presentation state, does not restore or save
user settings and never saves its synthetic project. It requests an exposed,
fixed 1400×900 window. Close before completion is a failed run. Native runs need
an available display and loopback sockets; use an isolated display when needed
and identify software adapters/virtual displays in the environment. A software
or virtual-display result is not a hardware/compositor baseline.

For the browser, rebuild `board_perf_web` with the existing WASM instructions,
serve `tools/performance`, and open:

```text
http://127.0.0.1:8782/rendered.html?artifact=fixtures/reference-v1.json&environment=environment.json&iterations=100
```

`backend=gl` explicitly selects wgpu's WebGL backend; the default retains eframe's
normal selection. Keep backend, browser, flags, CSS viewport, device scale and
headless/headed mode identical in comparisons. A failed WebGPU run cannot be
silently relabeled as a passing WebGPU baseline after selecting WebGL. Retrieve
`globalThis.performanceReport` or the visible JSON. `performanceError` is a failed
run; `cancelPerformanceReplay()` releases the runner without returning a report.

The rendered report has its own schema and comparator. Each boundary retains
raw samples and nearest-rank p50/p95/p99/max:

| Boundary | Observed interval |
| --- | --- |
| `rendered.<step>.ui` | Production app input handling, layout and shape generation |
| `rendered.<step>.input_to_ui` | Synthetic framework input injection through completion of that app pass; the native shared-edit step also includes transport waiting |
| `rendered.<step>.input_to_gpu_readback` | Injection through native screenshot callback, or browser canvas pixel readback after frame submission |
| `rendered.<step>.readback_delivery` | Pixel availability through consumption by the next UI pass, including hashing and delivery/scheduling delay |
| `mcp.roundtrip` | Native authenticated socket request through its complete reply |

Press/release pairs are delivered atomically in the rendered runner, unlike the
CPU runner's separate logical frames. GPU readback must not stretch a click into
a long press. Actions are paced at no more than 60 Hz, using the real framework
clock; reports record this input policy, so these measurements cannot be compared
to a different injection policy. One accepted UI pass corresponds to one sample.
Input batches remain pending across logic-only callbacks until an accepted UI
pass consumes them; an obscured/temporarily unpainted window must not erase a
click. The original artifact remains unchanged. Images must have nonzero, stable
physical dimensions and varying pixels; raw callback deadlines, dropped callbacks,
state assertion failures and renderer shutdown never produce passing reports.
The browser page also bounds module import, WASM initialization and renderer
startup to 30 seconds each, destroying a runner that arrives after its deadline.
Native startup and accepted UI progress are monitored by a separate 30-second
watchdog. Intentional idle holds extend that deadline; a stalled event loop cannot
hide failure merely by preventing its in-app timer from running. Watchdog expiry
normally exits this diagnostic process with an error and no report. Browser host
polling likewise rejects 30 seconds without accepted UI progress, independently
of the app's animation callbacks.

Only one screenshot is outstanding. Native mapping completion is serviced by a
nonblocking `Device::poll(PollType::Poll)` worker at a 1 ms interval while that
capture is pending. It stops on completion, cancellation or timeout; the UI never
joins it or waits on a socket. Browser readback copies the submitted canvas in a
microtask before the drawing buffer may be discarded. Image-copy and mapping
costs are included in the readback boundary; hashing follows its timestamp.
The renderer and last image digest are retained with capture counts and dimensions.

These are rendered **readback** measurements. They include instrumentation costs
and do not observe physical input arrival or the compositor's presentation time.
A readback budget miss is evidence about this boundary, not proof of a visible
preview miss. Do not promote a CPU pass or a GPU readback pass to an
input-to-photon claim. Native and browser clocks remain separate monotonic domains.

## Extended workloads and process resources

The following original command-generated recipes extend the existing artifacts:

| Recipe | Supported coverage |
| --- | --- |
| `board-limits-v1` | Exactly 4,096 tracks and 2,048 vias, with 512 footprints; the older `capacity-v1` artifact stays unchanged |
| `channels-v1` | 64 footprints, mixed widths/pad sizes, 410 tracks, through vias, two narrow parallel rails and 48 attached branches |
| `hierarchy-v1` | The current 64-sheet limit, 63 explicit child links, shared global labels and one symbol per sheet |
| `schematic-limits-v1` | The current 4,096-symbol / 64-sheet limits; eight physical footprints and explicit board exclusions for the other schematic parts |

Generate with `board_perf generate RECIPE REVISION ARTIFACT`, retain the exact
artifact, then reuse it across runs. CPU reports now distinguish sheet-local
connectivity from project-wide electrical connectivity and include a channel
probe checked against the exhaustive contour reference. BGA/high-pin-count and
curved tracks remain owned by #251–#253 and #259–#262; straight-track fixtures do
not qualify unsupported geometry or routing features.

Collect process memory without adding an allocator to production Rust:

```sh
python3 scripts/collect-performance-resources.py target/release/examples/board_perf tools/performance/fixtures/reference-v1.json /tmp/native-env.json /tmp/reference-resources --iterations 100 --allocations
```

The output directory must not exist. The collector records executable/artifact
hashes, commands, raw timings, 10 ms Linux RSS samples and observed process high
water where available. `--allocations` additionally uses an installed Valgrind
for a separate one-iteration whole-process allocation count. Its timings have
profiler overhead and are explicitly named `profiled-timings-not-a-baseline.json`.
Missing process counters remain unavailable, and missing profiler totals fail
collection rather than turning into zeros. These measurements include startup,
decoding, correctness oracles, fonts and reporting; they are not isolated index
allocation costs. Collection is bounded by a deadline and terminates only its
own child on failure. Preserve both successful evidence and failure logs.

## Idle and presentation diagnostics

Native replay can hold the same window between complete traces:

```sh
target/release/examples/board_perf_window tools/performance/fixtures/reference-v1.json /tmp/wayland-env.json 2 /tmp/idle-report.json --idle-seconds 28800 --preserve-on-stall
```

The report records the requested idle interval and actual total elapsed time.
A short hold verifies the mechanism; it does not establish eight-hour endurance.
Use `--preserve-on-stall` for an endurance investigation: the independent watchdog
reports failure and the diagnostic PID while preserving the process for stacks
and protocol investigation. Even if it later recovers, it cannot write a passing
report. End only that diagnostic after collecting evidence. Automated smoke runs
should omit this flag so failure exits promptly.
Diagnostic logs timestamp injected actions and their checked app state. For an
isolated Wayland run, `WAYLAND_DEBUG=client` also records protocol frame callbacks,
configure/visibility traffic and surface commits. Retain those logs with the
exact executable hash and environment, explaining clock alignment before comparing
protocol timestamps with the harness's monotonic milliseconds. Do not enable
protocol logging on an unrelated user's editing instance.

Follow the workspace's freeze record for occlusion, workspace switching,
lock/unlock, display sleep/wake and monitor transitions. Record each transition
and whether painting and File/resize input recover. Replay intentionally owns
its inputs and synthetic project; an ordinary editor endurance session is also
needed to qualify normal manual interaction. Preserve a stalled process and
collect its stacks and traces before recovery. The harness does not claim to fix
the recorded Wayland stall.

Feature-dependent queue lengths, coalescing, stale completions, index updates,
ERC progress and walkaround/shove commits must be instrumented with #243–#247,
#47 and the corresponding feature PRs. This infrastructure supplies reusable
artifacts, comparisons, rendered replay and resource collection; it cannot
measure implementations that do not exist. Record Windows/macOS coverage as
available or unavailable instead of inventing cross-platform results.

## Checker job measurements

For #243's first checker slice, the CPU diagnostic also accepts `checks`:

```sh
target/release/examples/board_perf checks /tmp/channels-v1.json /tmp/environment.json 100 /tmp/checker-jobs.json
```

The browser CPU page selects the same diagnostic with `&checks=1`. The
`pcbcraft-checker-performance/1` report records capture, stage/batch service costs,
engine total time, queue polling and cancellation acknowledgement, with raw
samples and report-equivalence assertions. A work batch considers at most 256
contact/clearance/courtyard pairs or outline items. Footprint projection uses at
most 32 placements per batch; remaining membership, dimension and finalization
phases yield at phase boundaries and retain supported model/report bounds. Browser
execution yields between batches after a nominal two-millisecond allowance;
this is a cooperative scheduling target, not a hard upper bound on a linear
phase or a geometric predicate. Retain measured overruns rather than reporting
that allowance as achieved latency.

Each view retains one running check and at most one replaceable pending snapshot.
Native execution has a process-wide limit of two snapshot workers shared by checking and copper preparation; excess starts
fail with a capacity error. The live MCP endpoint retains at most one deferred
check and refuses another until the slot retires. Canceling revokes publication
immediately; worker acknowledgement occurs after the current batch. Neither
closing a view nor application teardown joins a checker thread.

These new metrics are checker CPU/queue boundaries, separate from the established
CPU and rendered reports. Compare the existing reports with their existing
comparators, and retain the new report separately. Its synchronous browser
collector does not demonstrate a responsive window; real editor frames and the
rendered replay remain required for interaction claims. The routing continuation adds `board_perf routes ARTIFACT ENVIRONMENT ITERATIONS REPORT`
and `&routes=1` on the browser CPU page. Its separate
`pcbcraft-route-preparation-performance/1` report measures source/request capture,
resumable stages, publication, replacement and cancellation acknowledgement. It
asserts exact saved-byte equivalence with the synchronous shared command, one
revision per commit and stale-source rejection. The ordinary replay remains the
matched before/after measure for pointer reversals, layer transitions, cancellation
and concurrent MCP edits; these diagnostics do not establish presentation latency.

Copper preparation uses the same two-worker process limit and incremental browser
logic as checking. Each view retains one running job, one coalesced desired input,
one ready candidate and one immutable source context. Pad projection survives
pointer replacement while the source remains current. Capturing does not encode
project files. Candidate construction and final shared history publication remain
linear phases with supported model bounds; their measured costs are reported
separately rather than hidden by the browser polling allowance.

The route's source covers session identity, project opening, physical board,
revision and immutable rules/geometry. The host additionally checks view/input
epoch, request generation, active layer and local dimensions. An opaque prepared
edit can publish only once through `Session::commit_copper_edit`; stale edits,
reopening and Undo/Redo revoke it. GUI copper gestures and size properties defer
preparation. Headless callers drain the same engine job synchronously. Live MCP
retains one deferred copper request, rechecks its deadline, endpoint/Assistant
turn and modal ownership before publication, and cancels observed disconnects.
The socket worker samples disconnects at up to 20 ms intervals; no distributed
check can eliminate a disconnect racing an already authorized transaction.

Pending route geometry uses the existing chrome adapter's ghost styling and
progress appears in the board status. Escape and right-click cancel final
preparation; navigation runs before the gesture input gate. Prepared status means
the existing command's contact/lock checks passed, with clearance enforcement
and walkaround/shove still owned by their subsequent issues. ERC jobs remain
with the ERC feature. These checks do not qualify Wayland endurance/presentation
latency.
