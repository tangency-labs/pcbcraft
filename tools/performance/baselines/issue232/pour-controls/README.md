# Explicit pour and remove controls

This follow-up adds actual board-wide copper pouring/removal through the ribbon,
B/Ctrl-or-Command+B and shared GUI/live/headless commands. Zones retain their
assigned net/layer and authored outlines; removal suppresses automatic/checker
refill until another explicit pour. Derived authority revokes old fill/check/route
work without changing native project bytes or history. Numeric edge acceptance
uses an authored 600,000 nm clearance and checks the 2 nm reserve.

The required workspace pass completed 2,131 tests (12 ignored) before the final
app-only completion-repaint fix. Final affected app tests pass 1,056 (10 ignored),
with affected Clippy and native/browser builds. A sandbox app-only run had 40
socket-permission failures; the local-socket-enabled rerun passed them. One WASI
removal regression and dark/light actual-app GPU/menu captures passed.

The packaged browser test first caught a missing final repaint: worker completion
was polled after canvas paint and could leave the pending image until another
input. The final test verifies actual removed/restored pixels after completion;
worker messages alone were not treated as a pass. See
[browser-controls-final.json](browser-controls-final.json) and
[the failure review](failure-review.md). Native-window/manual interaction and
Wayland endurance remain unmeasured.

The one [navigation pair](comparison.md) uses the same command-built closed-edge
three-resistor demo. Both runs preserve native bytes, project geometry and input
assertions, and the after fill is ready. Raw reports include environment and
samples. The before is 89c1ac0; the after is the uncommitted controls immediately
before the final completion-repaint fix, with source hashes retained. It measures
steady warm navigation, with no active worker handoff; it does not qualify that
fix, refill throughput, allocation, presentation or a causal speedup. Initial
parallel verification builds stalled; own overlapping jobs were reduced with the
user's approval. No broader timing rerun was performed to improve the result.
