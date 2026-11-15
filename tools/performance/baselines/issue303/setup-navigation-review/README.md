# Board Setup access to design rules

Board Setup now exposes **Design rules → Board defaults / Net classes** in its
wide rail and compact selector, opening the same existing editors as Analyze →
Rules. No extra rule store or editor was added. Pending setup fields, material
edits and form-owned file jobs keep the setup draft open until Apply or Cancel;
stale source authority cannot hand off to a newly focused view. Untouched setup
suggestions are not applied by navigation.

All 1,866 workspace tests pass (11 ignored), including three new real-control
regressions at 1200/700-point widths for initial/existing setup, shared saves,
Undo, retained valid/invalid fields and stale revisions. Formatting, workspace and
harness Clippy, WASM check/package, native app/CLI builds and 15 harness tests pass.
[Verification](verification.json) distinguishes tests and code-reviewed gates.

The wide dark rail, light compact selector and editor destinations were inspected
using native GPU readback. Browser screenshots independently verify both shared
editor destinations and the dark compact menu. [Browser verification](browser-verification.json)
retains the limitations of interrupted or rejected coordinate automation; only
its identified screenshots count as successful states. Full logs, raw captures
and helpers remain under `setup/issue303/setup-navigation-review`.

The change reuses qualified Craft navigation/menus and existing forms without new
assets, dependencies, saved data or resolution policy. No new performance,
compositor or endurance qualification is claimed. The user's running native app
and session were preserved.
