# Shared net-class editor evidence

The implementation is `23f58ac`, followed by the form resize/choice-width fix
`64831ae`, based on PR #349's merge `6be9dec`. This slice adds the shared Rules
class/assignment workflow; it does not change the saved vocabulary or introduce
per-frame connectivity work. The [workflow](../../../../docs/net-class-editor.md)
and [failure review](failure-review.md) describe behavior and tested boundaries.

Required checks passed: workspace Clippy/tests, browser compilation and packaging,
license/source/ban checks, and the opt-in performance harness and evidence
validators. The initial workspace suite passed 1,857 tests with 11 ignored; after
the modal adapter fix, the affected app/chrome suites passed 965 and 32 tests,
with nine app tests ignored. The unchanged engine/transport results remain valid.
The opt-in harness passed 15 tests. Commands and exits are in [checks.json](checks.json).

The bounded comparison reuses the exact command-generated reference and assigned
reference artifacts from [#302](../issue302/performance/), with 20 and 10 samples.
Both correctness runs passed and the official comparator accepted their identities
and environments. Raw reports and source hashes are compressed here; environments
and comparisons remain readable alongside them. No broad replay was repeated to
obtain a more favorable result.

The maximum reference UI p95 was 1.089 ms before and 1.070 ms after; the assigned
replay was 1.092 ms before and 1.224 ms after. Individual assigned UI boundaries
increased by up to 0.207 ms. These shared-host samples do not isolate the new form,
which did not exist in the base, or establish a cause for the timing differences.
Class route-engine total p95 was 0.317 ms before and 0.308 ms after. Keep the raw
per-boundary results when evaluating this change rather than treating one number
as an overall speed claim. Large class/net/selection authoring latency, allocation,
physical presentation and Wayland endurance are not qualified by these runs.

Native GPU readback inspected wide dark and narrow light class forms. The final
packaged app was exercised in headless Chrome 148 with WebGL2/ANGLE SwiftShader:
existing Power binding clear/restore, effective dimensions, dark/light rendering,
and an open-form resize without pointer input. Browser class creation with explicit
`0.350001 mm` and refreshed mil display was checked before the final bounded-choice
width and resize-paint change. [Browser metadata](visual/browser.json) records those
boundaries and the bundle hashes; screenshots cover authored and effective forms.
This is functional software-rendered evidence, not a compositor latency result.

The retained native render helper only enabled GPU readback in an isolated source
snapshot; it is absent from production dependencies. Source maps, complete logs,
raw native captures and browser artifacts remain under `setup/issue303` in the
shared development workspace. No user-visible desktop app, remote branch or PR
was created by this local implementation task.

The [layout follow-up](layout-review/README.md) supersedes the original form
screenshots after @travle1's local testing feedback, with separate source and
functional validation records.

The [active-source follow-up](active-source-review/README.md) further separates
class contributions, effective provenance and unsaved assignment previews. It
keeps the existing resolution policy and original performance evidence intact.

The [equal-limit follow-up](tie-source-review/README.md) labels matching hard
bounds explicitly while retaining both contributors in the tooltip and Details.

The [constraint illustration follow-up](geometry-review/README.md) brings Board
Rules onto the shared form layout and adds dimension drawings to all Rules pages.

The [Board Setup navigation follow-up](setup-navigation-review/README.md) adds
Design rules destinations for the existing board-default and net-class editors.

The [composition-guidance rebase](composition-rebase-review/README.md) records
unchanged commit replays and the focused draft/input-interface extraction.

The [embedded Board Setup follow-up](inline-setup-review/README.md) replaces those
destinations with retained pages inside Setup and fixes source-window hosting.

The [CodeRabbit follow-up](rabbit-review/README.md) repairs no-board Rules access
and short-window navigation, and adds explicit GUI/MCP parity comparisons.
