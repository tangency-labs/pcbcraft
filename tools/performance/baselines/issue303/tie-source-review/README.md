# Equal hard-limit source summaries

When a class matches the baseline clearance, Active source now reads
**Matches built-in minimum** while the effective value stays **0.2 mm**.
The tooltip and Contributions and identities retain both sources. Equivalent
board, maximum and class-only ties have corresponding summaries; resolution
and saved data are unchanged.

Seven Rules tests pass, including actual preview/assignment controls and both
retained Active detail entries. Affected app Clippy, formatting, WASM check and
native rebuild pass. The two native GPU readbacks were inspected in dark wide
and light compact layouts. Source hashes and check boundaries are recorded in
[source.json](source.json) and [verification.json](verification.json). Full logs
and raw captures remain under `setup/issue303/tie-source-review`.

This focused text change adds no new timing, packaged-browser interaction or
native compositor qualification. The earlier workspace and performance results
retain their original scope. The user’s running app was preserved.
