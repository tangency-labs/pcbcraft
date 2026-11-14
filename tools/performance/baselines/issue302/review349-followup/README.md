# Layer-change via follow-up for PR #349

CodeRabbit's fourth finding concerned unresolved via preferences at track pickup:
a later layer change could submit retained sizes as explicit values. The draft
now captures its source-contact policy alongside its existing revision authority.
V, PageUp/PageDown and the route menu share the same pre-mutation via resolution.
Unresolved or hard-invalid sizes return an error without changing the path, layer
or project. Track-only routing still works, and an explicit valid ribbon pair
can resolve the via choice before a new pickup.

The real-input regression checks V, layer keys, the route menu, unchanged snapshot
authority, completion of a plain track with conflicting via preferences, and two
actual layer-change vias committed with an explicit valid pair. It fails with the
old module restored. The initial regression setup changed ribbon values while a
draft was active; existing focus routing cancels such a draft, so the corrected
test restarts pickup after ribbon changes. That setup correction changed no
product focus behavior. All failed/corrected logs are retained in `checks.zip`.

The affected app suite passed 959 tests (nine ignored), along with 15 harness
tests, workspace/feature Clippy, formatting, browser app/harness compilation and
native builds. Engine/MCP code and dependencies were unchanged, retaining the
previous workspace/WASI/dependency results. Shared immutable policy metadata adds
no snapshot retention, graph lookup during pointer motion, worker or blocking
operation. No new rendering, timing or endurance qualification is claimed.
