# Issue #302 evidence

Local implementation `d6ce94a271e6197c33903f5e322058c4dc86117d` applies assigned rules to contact defaults,
changed copper, snapshot queries/overlays and both DRC paths. The
[consumer contract](../../../../docs/formats/net-class-consumers.md) defines identities,
conflicts, provenance and limits. [Verification](verification.json) records checks;
[the failure-path review](failure-review.md) separates exercised cases from code review.

The native suite passed 1,844 tests with 11 ignored, combining the initial
unaffected-crate results with the corrected engine/remaining-crate run and all
workspace doctests. Clippy, formatting, 14 harness tests, Python/Node validators,
WASI board/class/registry tests, browser compile, desktop/CLI builds and dependency
checks passed. `checks.zip` retains development failures, corrected runs and the
final-check driver. The old first-board test assumed immediate blanket refusal;
it now drains the prepared job and verifies atomic layer rejection. The new Undo
test permits the existing monotonic numeric ID counter while comparing restored intent.

## CPU observations

The exact retained reference/channel artifacts were measured before and after
with 20 and 10 samples respectively on the same shared host and release profile.
Both [reference](performance/reference-comparison.md) and
[channel](performance/channels-comparison.md) comparators accepted their metadata.
No noisy result was discarded or followed by a larger replay. Raw reports and
source/build/environment hashes are in `performance/`; gzip fixtures decompress
to the exact recorded input hashes.

| Boundary (p95 ms) | Reference before | Reference after | Channels before | Channels after |
| --- | ---: | ---: | ---: | ---: |
| clearance.prepare | 0.195159 | 0.238009 | 3.132468 | 3.180109 |
| clearance.pairs | 0.082786 | 0.074110 | 2.478185 | 2.531636 |
| clearance.probe.crowded | 0.005100 | 0.005951 | 0.024707 | 0.019797 |
| drc.synchronous | 0.214615 | 0.263136 | 4.880166 | 4.343553 |
| command.move_tracks | 0.099137 | 0.115197 | 2.047402 | 1.992058 |
| replay.shared-edit.ui | 0.940975 | 0.946666 | 6.127999 | 5.585215 |

The reference preparation/DRC observations rose by roughly 0.04–0.05 ms, while
channel preparation was roughly flat and DRC was faster. Other frame/tessellation
results vary; these shared-host samples do not isolate a general speed change.
The channel shared-edit CPU frame still exceeds 4 ms, so this is not a #237
latency qualification. Legacy unmeasured strings in the matched report describe
boundaries omitted by that runner; existing background jobs are implemented.

The assigned derivative was generated from the exact reference archive using
`net_classes.create` and `nets.set_classes` in [this generator](performance/assign-fixture.rs).
It adds no hand-edited project data. Its ten candidate-only samples measured
contact policy at about 0.115 ms p95 and complete classified route preparation at
about 0.309 ms p95. The omitted width was committed as 0.5 mm, with provenance and
exhaustive query/probe oracles checked. Batch maxima are retained separately. The
pre-consumer revision refuses this operation, so these are absolute observations,
not before/after speedup claims. They do not qualify large-class capacity.

## Visual and source limits

The [dark](visual/class-route-dark.png) and
[compact light](visual/class-route-light-compact.png) captures show the existing
Craft width controls and routing halo using class dimensions. The temporary
qualified renderer and exact source metadata are recorded under `visual/`.
Removing the test pointer cleared hover in the first attempt; those images were
rejected and the corrected capture retained hover. Later production changes were
bounded pad-contact preparation and equivalent Option plumbing, with app tests
passing; the presentation implementation stayed the same.

These are headless GPU functional images. CPU replay, WASI execution and browser
compilation do not establish native-window/browser rendered latency, physical
input-to-visible latency, memory/allocation behavior or Wayland endurance.
No class editor, routing clearance avoidance, pours, impedance or new storage
vocabulary is included. The work remains local and unpublished.
