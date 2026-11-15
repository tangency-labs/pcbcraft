# Rules inside Board Setup

Board defaults and Net classes are now pages within the existing Board Setup
navigation, using shared editor content and the existing commands. Each page save
retains pending edits on other pages; physical Apply also retains Setup when rule
edits remain. Page-local revert controls discard only their own draft. Source
window ownership applies to both embedded and standalone editors, including when
the main window has switched to the library workspace.

The [verification](verification.json) records 1,870 workspace unit/integration
tests passing with 11 ignored, plus one documentation test and 15 opt-in harness
tests. Formatting, workspace/harness Clippy, WASM compilation, release web packaging
and native app/CLI builds passed. The initial workspace command reached a stale
shared-cache dependency error during doc compilation; targeted package refresh and
a documentation-only rerun passed. [Check attempts](checks.json) retain that failure
rather than reporting the original command as successful.

GPU readbacks were inspected in wide dark and compact light layouts. The packaged
browser smoke flow saved a class while layer/default drafts remained pending,
returned to inspect both drafts, and resized the open form before using compact
navigation. [Browser metadata](browser.json) records Chrome/SwiftShader, viewport
sizes and errors; screenshots show retained fields and the containing Setup form.
The [failure review](failure-review.md) separates tested paths from code inspection.
Raw logs and all captures remain under `setup/issue303/inline-setup-review`.

This adds no performance qualification and does not certify native compositor or
Wayland endurance behavior. Existing timing evidence keeps its original scope.
The user's desktop app was not restarted, and no push or PR was created.
