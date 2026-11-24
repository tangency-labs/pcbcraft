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


The branch was then rebased onto main at 3841189 after PR #395 became
conflicting. Capability documentation retains both the new pour controls and
main's layer-role/base-copper descriptions. Both native decoders and the new
project constructor retain main's part-group data alongside the derived cache.
A shared-command regression verifies part-group records/memberships survive
pour/remove, native-document worker transport and unchanged Save/Open bytes.
After that rebase, the full workspace suite passed 2,172 tests (12 ignored),
with workspace all-target Clippy, native/browser builds and the same actual
browser remove/repour pixel oracle passing. No new timing replay was added;
the earlier timing sample retains its stated source and measurement limits.

Main advanced again during verification, and GitHub still reported a conflict.
The second rebase uses 1128e3e, retaining recovery observers and whole-library
export. The conflict was confined to capability documentation. A focused
regression checks that both shared command entry points leave pour/remove out
of recovery edit notifications, preserve native bytes/revision, and still
report a subsequent authored edit. The earlier full workspace and browser
results above describe the first rebase; the second rebase is checked with
affected engine/storage/app suites and current native/browser builds, without
another timing replay.

On that second base, 797 engine tests, 82 storage tests (one ignored) and
1,057 app tests (ten ignored) passed, along with workspace all-target Clippy
and the native build. Browser packaging initially refused an inherited
NO_COLOR=1 setting before compilation; the corrected invocation removes it.
The corrected offline package also exposes the cached wasm-bindgen 0.2.130
and wasm-opt version 123 tools on PATH. Packaging then passed, and the
[second-base browser oracle](latest-main-browser.json) passed actual menu
remove/repour pixel assertions, two successful worker completions and zero page
errors. The failed environment/tool-discovery invocations remain in the local
verification record; they were not Rust compilation or application failures.
