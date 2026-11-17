# Zone-fill verification for #232 / PR #395

The navigation pair compares two explicitly recorded uncommitted states after
`43b5083`, before and after automatic fill/canvas integration. Source hashes for
the four selected boundaries are retained beside the reports; these are not
clean-commit measurements. Both reports retain environment, raw samples, geometry
and the same production-packed project SHA-256. The copied fixture was authored
through shared commands for #231 and is preserved unchanged here.

The saved zone/origin fixture has no closed Board Outline. The after report
therefore correctly records unavailable fill. This comparison covers read-only
navigation in that state, not display of filled copper or refill throughput.
Eight warmup frames precede ten samples per measured operation. CPU UI and
tessellation timings exclude GPU submission/presentation; host power, thermal
state and concurrent activity were not controlled. Lower observations establish
no causal speedup. The stopping condition was one matched pair; no rerun was
performed to improve its timing result. See [the comparison](navigation-comparison.md).

`packaged-browser.json` records eight success/failure cases in the packaged
Chrome worker host before the final host-status and accepted-net-short changes.
The host glue exercised there is unchanged in `14c3a65`; `browser-ui-final.json`
records that commit's rebuilt package opening the command-built three-pad demo,
starting one actual automatic worker and receiving a complete fill. The worker
sample includes module startup; its foreground heartbeat demonstrates scheduling,
not input/presentation latency. Timeout testing accelerated the 30-second host
timer to 50 ms. A null-argument exception was injected at the worker entry point;
a real SolidCore abort/stack exhaustion was not reproduced.

The final engine tests separately cover host failure/stale rejection, current
analytic DRC including accepted-net shorts, track-move/refill and deterministic
native reopening. Headless native GPU captures were inspected in both themes.
Native window interaction, realistic-board serialization/decode costs, large
native kernel-call cancellation latency and Wayland endurance remain unmeasured.
Assigned-via integration (#369), routing integration (#248), class pour intent
(#304) and zone output (#233) remain separate qualification/work.


The [explicit pour/remove follow-up](pour-controls/README.md) records physical
unfill/resume, board-edge acceptance, current authority, the completion-repaint
fix and its separately bounded checks/navigation sample.

The later [supplied-demo fill-delay investigation](fill-speed/README.md) isolates
calculation from capture/connectivity reads and compares the original and
prepared-operand paths under the same optimized profile. It records the separate
development-build effect and retains the single-board/three-sample limits.
