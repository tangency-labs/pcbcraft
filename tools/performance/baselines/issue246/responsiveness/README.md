# Routing feedback and starting-net preservation

This follow-up to #246 investigates @travle1's report that the optimized local
route tool still trailed the pointer. The saved reference and decompressed
channels artifacts were reused byte for byte in shipping release builds on the
host/toolchain recorded in each report. Three repetitions per fixture each move
the pointer to 24 distinct clear targets at 60 Hz. The diagnostic inspects actual
preview copper in each input frame, uses the production native worker queue, and
requires unchanged authored bytes after Escape.

| Fixture | Before: current preview/input frames | Responsiveness fix | Final, including starting-net fix | Final UI p50 / max (ms) |
| --- | ---: | ---: | ---: | ---: |
| reference | 0 / 72 | 72 / 72 | 72 / 72 | 1.057 / 1.387 |
| channels | 0 / 72 | 72 / 72 | 72 / 72 | 2.712 / 3.048 |

Previously, completed work was polled after painting, while the next moving
pointer could cancel its generation before consuming it. Consuming read-only
completions before paint removes that handoff delay. A warm query can now finish
up to eight search suspension units in the input frame; longer searches retain
the same continuation in the worker. Cold projection and contended locks do not
wait on the UI thread. Final actions still run after input with existing source
and full-path validation, so cancellation can revoke publication.

This moves a small amount of work onto the input frame: before UI p50 was 0.822
ms for reference and 2.333 ms for channels. The final observations are about
0.23/0.38 ms higher. The raw intermediate observations are also retained rather
than selected away. The runner always performs at least one extra observation
frame after movement stops, even when the final endpoint was already current;
that counter's minimum of one does not mean an extra delayed frame. These small,
correlated samples on a shared host do not qualify p95/p99, GPU presentation,
physical input latency, Windows/macOS or Wayland endurance.

While manually testing the responsiveness build, @travle1 reported that a route
from an unconnected resistor pad could cross the named net of its other pad.
The regression reproduced a three-probe reached path into the named pad. Probe
contact unions were granting a same-net exemption after encountering the foreign
net. The final solver grants that exemption only from the original starting
contact and retains that contact through fixed vertices and layer/via changes.
Full-path contact unions still enforce shorts and class rules. Engine tests cover
both directions, previous-path hints and a detour around named copper; real input
covers rejected clicks, preserved history, cancellation and finishing a legal
prefix. Existing valid same-net, class and through-via routes remain covered.
No named net, registry record or saved format is added for unused terminals.

`source.json` records executable/diff identities and exact final runtime hashes.
The retained measured runner and overlays distinguish the intermediate measured
source from the final source. The only final runner change is Clippy's equivalent
`or_default()` spelling. No verification images, generated project or executable
are checked in. The earlier interactive evidence remains historical, with its
original source hashes and limitations.

## Preview and commit trace

The companion trace retains the exact reference project's native bytes and picks
the first saved airwire with conductive copper at both ends. Airwire 0 starts on
a legacy center-only marker, which cannot lend physical membership. Its blocked
solver outcome remains recorded and asserted; successful publication assertions
use the next supported connection. This selection examines pad geometry, not
solver success or timing. No native project file is edited or regenerated.

Three traces per renderer wait for a fully validated preview, then click a changed
final target and await one committed revision. Each new route must reduce the
saved airwires, pass full DRC for its new copper, reopen exactly, undo to the
original bytes in one step and redo to the committed bytes. These correctness
checks run after the commit interval. Each renderer produced 18 nonblank captures.

| Renderer / development profile | Validated preview to UI, raw ms | Click to publication UI, raw ms |
| --- | --- | --- |
| Native Wayland, RTX 4070 Ti | 35.59, 23.33, 24.14 | 56.05, 55.86, 56.84 |
| Chrome WebGL, SwiftShader | 111.90, 34.40, 32.60 | 78.00, 75.20, 73.20 |
| Isolated X11, llvmpipe | 1013.83, 1007.59, 1008.34 | 3008.58, 3007.50, 3007.56 |

The isolated X11 run exposed second-long waits between UI validation passes;
a separate short Wayland run exercised the normal desktop renderer and retained
both series. This does not establish the lower-level cause of the X11 pacing.
The first isolated attempt's 60-second whole-trace deadline was too short for its
observed readbacks; it produced no passing report. Its logs remain in local setup.
Browser startup also failed once because the on-disk temporary path exceeded the
Unix socket limit; shortening that disposable path allowed startup without changing
product code. No incomplete report is counted as a passing sample.

These observations satisfy the scoped requirement to record native/browser
preview and commit intervals; they do not qualify #237's proposed p95/p99 targets.
The validated-preview boundary also includes full candidate preparation, rather
than timing only the first drawable legal plan. The observed UI preview values
exceed 16.7 ms, while compositor presentation remains unmeasured. GPU readbacks,
retained in full, include capture/transfer overhead: Wayland commit readback was
114.13–117.48 ms and browser commit readback was 193.90–207.20 ms. Three development
samples on one host cannot establish shipping tails, capacity, hardware browser
performance or endurance. Those broader tracker targets remain open.

The earlier 44-capture native/browser reversal, layer, cancellation and shared-edit
traces are also retained for the final product code. Filled-zone routing remains
with #232 integration; #246 explicitly permits its initial supported-copper slice.
No verification photos are included.
