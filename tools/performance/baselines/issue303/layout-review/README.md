# Rules form layout follow-up

Under @travle1's screenshot feedback, `8d942a1` gives both Rules pages a padded,
800-point maximum reading width, aligned dimensional rows, consistent inherited
fields and readable source names. Conflicts appear before the effective rules;
technical IDs remain in Details and disambiguate duplicate class names. Current
net choices use component references and pin labels as display aliases while
keeping the original typed command targets.

The pinned Craft selectable table introduced another scroll owner and row focus
targets. Real-control testing exposed failed reveal of lower preference fields.
The final chrome adapter composes shared controls in reserved cells under one
vertical scroll owner instead. It uses the shared modal/footer, section treatment,
fields, checkboxes and semantic warning band without new generic control painting.
The width-aware modal retains the previous board setup defaults and schedules a
settling frame when its viewport or requested width changes.

The workspace passed 1,858 tests with 11 ignored, alongside Clippy, native/browser
builds and the opt-in harness checks (15 tests). Existing real-control coverage
includes class/assignment edits, lower-field scrolling at three widths, mixed
units, untouched precision, cancellation, stale source/revision, Undo/Redo,
Update PCB and reopen. Native GPU readback inspected class and conflicting net
forms in dark/light and wide/compact hosts; the packaged browser additionally
covered inspection, open-modal resize, theme/units and wheel scrolling to Details.
[Verification](verification.json) records commands, exits, renderer and limitations.

These are functional layout/control checks, with no presentation or timing claim.
The earlier CPU comparison remains a historical ordinary-editor measurement; this
follow-up adds no new engine or saved format, and no new CPU replay was collected
for an unrelated modal layout. Complete logs, raw readbacks and the isolated test
source remain under `setup/issue303/layout-review` in the development workspace.
