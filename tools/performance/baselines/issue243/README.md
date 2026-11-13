# Bounded checker and copper preparation evidence

These are saved development measurements for #243, collected on November 12,
2026. Packaging this directory did not rerun the workloads. The manifest records
stored and uncompressed SHA-256 hashes; `.json.gz` and `.log.gz` files retain their
original bytes, including raw samples, environment/build metadata and historical
local paths. Publication/clean-state fields in the captured verification records
describe when those records were written, before this PR was published.

The command-generated inputs are the existing `reference-v1`, `channels-v1` and
`board-limits-v1` artifacts under `tools/performance/fixtures/`; their exact
uncompressed envelope hashes are in `manifest.json`. No project fixture was edited
or regenerated for the comparisons.

| Evidence | Scope |
| --- | --- |
| `checker/before/`, `checker/after/` | CPU and native/browser rendered before/after reports; captured checker stages, queue polling/cancellation and board-limit smoke samples |
| `routing/before/`, `routing/after/` | Initial 100-sample CPU reports, ten-iteration rendered reports and whole-process resource records |
| `routing/serial/` | Sequential native 100-sample CPU repeat, retained with its transient slowdowns |
| `routing/final/` | Final captured-source manifest/overlay, 100-iteration native/browser job lifecycle diagnostics, targeted ten-iteration CPU reports and native rendered report |
| `checks/` | Final implementation's workspace/harness tests, Clippy, formatting, dependency checks and native/WASM/Trunk release build logs |
| `failure-review.md` | Tested cancellation/authority/failure cases and paths reviewed by code inspection |

The checker capture follows base `739ae2c` with its recorded source overlay; its
verification record identifies the small post-collection changes before `c4f184f`.
Routing's final capture follows `c4f184f` with the source fingerprint/overlay in
`routing/final/`, producing implementation commit `0bbda9b`. The subsequent
`3985f9f` changes verification guidance only. Source snapshots describe the actual
measurement candidates rather than relabeling every report as the final commit.

The native worker limit is shared by checker and copper work: two executing jobs
per process. Each owner retains one running request and at most one replaceable
pending request. GUI routing also retains one ready candidate and a reusable
snapshot pad projection. The lifecycle diagnostics measure capture, batches,
publication, replacement and cancellation acknowledgement. Native cancellation
collection polls at one-millisecond intervals; browser timing resolution can
report small operations as zero. These are service-cost observations, not physical
input-to-visible measurements.

Broad editor replay did not isolate the changed scheduler. Transient slowdowns
also appeared in unchanged UI/tessellation work, so these reports do not establish
that the scheduler caused those tails or that the whole editor became faster.
The final preview handoff removes a duplicate pad projection by queuing the path
already resolved for painting.

Earlier candidate native/browser rendered comparisons passed their identity
checks, with 440 GPU captures in each report. The final native rendered run also
completed 440 captures, but its comparison was rejected for differing observed
focus/visibility transitions; the rejection is retained. The additional final
browser rendered repeat was stopped instead of repeatedly pursuing a clean
comparison. Earlier candidate comparisons are not qualification of that final
rendering change. Raw evidence and limitations are retained in both cases.

The nominal two-millisecond browser polling allowance is a tuning choice, not a
hard batch deadline or an acceptance threshold in #243. Board-limit samples are
smoke checks, not tail baselines. Whole-process RSS includes startup and correctness
work; isolated queue allocations, compositor presentation, physical latency,
Windows/macOS and overnight Wayland endurance remain unqualified. Browser
measurements use headless Chrome/WebGL/SwiftShader rather than WebGPU.
