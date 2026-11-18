# Interactive walkaround verification

This is a bounded integration check for #246, based on main `33b4470`. The saved
reference artifact and the decompressed committed `channels-v1.json.gz` were
reused without changing project or trace bytes. `source-hashes.json` identifies
the implementation; the final replay-only adjustment followed the service runs.

Native reports use the development profile with debug information, on the
hardware/toolchain named in each report. Power state and background load were
uncontrolled. Three samples per native service case establish correctness and
observed service costs; they do not establish p95/p99 targets or a shipping speedup.
The synchronous before/after reports reproduce the same outcomes, probe counts
and point counts. The batched runner additionally requires exact equality with
the synchronous plans, exhaustive validation of new copper, one atomic revision
per reached route, and retirement of coalesced/canceled work.

The largest observed planning batch, atomic publication and cancellation
acknowledgement costs, in milliseconds, were:

| Fixture | Planning batch max | Atomic publish max | Cancel acknowledgement max |
| --- | ---: | ---: | ---: |
| reference-v1 | 0.309 | 0.048 | 0.027 |
| channels-v1 | 1.390 | 0.107 | 0.041 |

The native window and headless Chrome/SwiftShader browser each completed one
44-capture rendered trace. Both retained the layer-transition, cancellation,
byte-preservation, intervening shared edit and final inactive-tool assertions.
Native delivery included authenticated loopback MCP. The browser also ran the
shared planning-job diagnostic; its timer resolution and software rendering are
recorded in the report rather than treated as a hardware baseline.

The first native after-run failed with `layer key did not change the route layer`.
The old replay asserted an immediate layer change, while the editor now waits for
full path/via validation. The replay retains the accepted action without repeating
input, waits with a deadline, then requires the same final layer and cancellation
results. CPU settle passes are aggregated into one sample per iteration. The
rendered input policy explicitly records validated layer completion. Therefore
the old and new rendered reports have different interval definitions and are not
a matched latency comparison. The original failure log is retained in local setup
evidence. No failed/incomplete run is counted as a passing report.

These are CPU service and GPU-readback observations, not physical input-to-visible
preview, compositor presentation or endurance qualification. No portable latency
budget is certified. Full #246 workload/platform qualification remains open,
including denser editor traces and current filled-zone integration with #232.
No verification screenshots, generated native demo or temporary renderer helper
are committed here.
