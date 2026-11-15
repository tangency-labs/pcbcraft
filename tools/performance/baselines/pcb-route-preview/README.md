# PCB startup and routing preview verification

These retained records cover runtime commit `e8d8c7c` on base `a5c72a9`.
Packaging adds no runtime changes and repeats no measurements. The manifest
identifies the saved fixture and original bytes of each record. The original
`verification.json.gz` is a local checkpoint taken before publication; its
authorization and publication fields describe that earlier state. Source hashes
identify the code tested independently of later evidence-only commits.

The new empty-board zoom and pending-preview regressions fail before their fixes
and pass afterward. The app suite passed 897 tests with nine ignored, along with
app Clippy, formatting, the WebAssembly check, dependency policy and the desktop
build. Submission workspace Clippy passed with warnings denied, and all 1,693 workspace
tests passed with ten ignored; their logs are retained separately. Large outline and
clipboard fixtures now choose an explicit overview scale; they do not test the
startup zoom. The new zoom regression checks creation, independent workspace
cameras, saved restoration, ordinary edits, empty Fit and a second new board.
The preview regression checks multiple pointer changes on Top and Bottom copper
in both themes, before and after preparation, and exact cancellation state.

Each native replay completed one saved reference iteration and 44 GPU readbacks
on isolated X11/Xvnc with an explicitly selected lavapipe adapter. The comparator
accepted the pair. The raw reports, environment, build logs, source hashes and
comparison remain available here; one iteration provides integration context,
not statistical latency or performance qualification. The changed initial camera
scale intentionally changes projected positions. GPU readback does not establish
compositor presentation or input-to-photon latency, and this run does not cover
the recorded Wayland endurance failure.

The four PNGs show the pending draft before and after the fix in wide dark and
compact light layouts. They use the actual app renderer from a temporary source
copy; capture code and dependency features are absent from the implementation.
The helper's pointer is synthetic. Additional local captures include empty boards
and settled preparation, but those can retain the preceding painted status;
the focused shape regressions establish the pending/prepared color behavior.

The WebAssembly diagnostic built before the fix and the shipping browser crate
was checked afterward. A packaged browser replay was unavailable because the
wasm-bindgen CLI was absent. Browser execution, Windows/macOS execution and native
presentation/endurance qualification remain unmeasured.

The separate failure-path review traced fresh camera slots through new binding,
open/reopen and saved-state restoration; populated Fit retains its original
projection while empty bounds use the new scale. Routing changes only the paint
color choice. Candidate capture, worker coalescing, authority/revision checks,
publication, cancellation and status retain their existing behavior. The tests
above exercise the camera and color paths; those unchanged worker paths were
reviewed by reading rather than requalified with new measurements.
