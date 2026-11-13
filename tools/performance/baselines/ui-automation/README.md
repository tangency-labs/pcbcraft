# UI automation verification

These existing records cover runtime commit `d28a3ab` on base `de281fd`.
Packaging adds no runtime changes and reruns no measurements. The manifest
records the canonical fixture hash and the original bytes of each gzip record;
`verification.json.gz` is the local checkpoint taken before publication, not
current PR status. Its source hashes identify the code exercised by the checks.

The workspace suite passed 1,642 tests with nine ignored. Formatting, workspace
Clippy, the WebAssembly check, dependency policy and desktop/CLI builds passed.
The opt-in native smoke passed separately on isolated X11/Xvnc with DRI3 disabled
and an explicitly selected lavapipe adapter. It exercised the real Wire button,
File menu, rename field and confirmation through authenticated MCP, and checked
the resulting project name. The headless regressions also cover drag/Undo,
interruption, stale geometry, queue bounds, cancellation during an in-flight
button pass, disconnects and detached-window lookup. See `failure-review.json.gz`
for the separate failure-path review and its test-versus-reading distinction.

The Chrome WebGL/SwiftShader before and after replays each completed one reference
iteration and 44 captures. Their raw reports and comparator output are retained;
one iteration supplies integration context and cannot establish tail latency or
attribute a performance regression. The native after replay failed during warmup
and produced no after report, so no native comparison is available. It was not
repeated. The earlier NVIDIA/DRI3 server crash and the corrected software baseline
are both retained rather than relabeled as a successful hardware result.

Native model/input checks passed, while a captured completion milestone still
showed the typed modal after an accepted UI pass and a visual hold. This evidence
does not qualify compositor presentation or recording reliability. No physical
input-to-photon, Wayland endurance, Windows or macOS claim is made. Input is
limited to the original native window and split panes; detached windows expose
lookup only, and specialized widgets may lack names. The user-facing contract is
in [UI automation](../../../../docs/ui-automation.md).

PR #325 review fixes add viewport-scoped collection and a human-consent boundary
for Assistant controls. The `review-*` records identify this follow-up by source
hashes on base `41c2509`; they do not replace the original baseline measurements.
The affected app/chrome suites passed 890 tests with eight ignored, including
regressions for detached Assistant lookup, synthetic approval clicks/Enter, global
permission settings and a pre-opened agent permission popup. Clippy with warnings
denied, formatting and the WebAssembly check passed. No performance or presentation
measurements were repeated.

The subsequent `review-enter*` records cover the outside-diff focused-draft
submission finding on base `870057c`. The new regression fails before the guard
and passes after it, checking that synthetic Enter leaves the draft, prompts,
messages and turn unchanged while human Enter still sends. All six focused
synthetic-input tests and app Clippy passed; these are functional checks without
new performance or presentation measurements.

The `review-third*` records cover connection enable/export consent, idle cancellation,
held physical pointer admission and deterministic disconnect readiness on base
`96643a9`. Three behavior regressions fail before the fixes. The app suite passed
867 tests (eight ignored); the strengthened socket fixture passed separately,
along with final app Clippy, formatting and the WebAssembly check. These records
add no performance or presentation qualification.
