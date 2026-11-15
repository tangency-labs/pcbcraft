# Constraint illustrations and shared Rules layout

Board Rules now uses the same padded 800-point modal, bounded values, aligned
rows and fixed footer as class authoring and effective net rules. Built-in fields
remain read-only while labels and illustrations retain their ordinary contrast.
The geometry column marks copper spacing, track width, copper-to-edge spacing,
radial annular ring, outside via diameter and drill diameter. Hover and accessible
image descriptions identify each span; drawings are illustrative rather than
scaled to the current value.

All 1,863 workspace tests pass (11 ignored), along with workspace Clippy,
formatting, WASM check/packaging, native app/CLI builds and 15 harness tests.
Real-control regressions cover separated geometry/field bounds at 1200, 700 and
400-point widths, disabled built-in inputs, exact unit/reset behavior, history,
revision ownership and lower-field reveal. The existing command/resolution and
save contracts are unchanged.

Native GPU readbacks were inspected in dark wide and light compact layouts.
The packaged browser additionally exercised an existing class, Board defaults,
a dimensional draft, scrolling, an open-form resize, the final field and Cancel.
It reported no page errors. [Browser metadata](browser.json), [source hashes](source.json)
and [verification](verification.json) identify those boundaries; raw captures,
complete logs and helpers remain under `setup/issue303/geometry-review`.

The electronics illustrations are original PCBcraft geometry composed beside
qualified Craft controls. No external artwork, dependency or toolkit revision
changed. This follow-up adds no new timing, compositor or endurance claim; the
earlier performance evidence retains its original scope. The user's running
native app/session was preserved.
