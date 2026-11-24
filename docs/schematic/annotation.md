# Preview schematic annotation

Use `annotate.preview` to review reference designators without editing the
schematic. It is a shared project command available in headless and live MCP,
including a read-only headless session.
The command does not need a PCB. Its query result is returned even with
`inspect: false`:

```json
{"command":"annotate.preview","params":{},"inspect":false}
```

Pass `sheet` with the sheet UUID from project inspection to limit the report to
that sheet. Omitting it covers the whole project, independent of the active
view. The optional `expected_revision` rejects a preview of a more recent
project state. The reply contains `revision` and `annotation_preview`, with
`rows` and `errors`. No review authority is cached; the revision describes the
state previewed and is not an Apply token.

Each row gives the stable `part_instance` ID, the representative `symbol` ID,
its `sheet` and `occurrence` UUIDs, its center `at` in integer nanometers,
`locked`, and the `current` and `proposed` designators. A part with several
placed units still has one designator and one row. The first in-scope unit is
its representative: sheets follow project order, and units on each sheet
sort by increasing Y, then X, then unit ID. Rows with no proposed change
repeat the current designator, including locked rows.

An unlocked part is eligible only when its displayed designator is exactly its
symbol prefix followed by `?`, such as `R?`. Ordinary placement already gives a
numbered designator; gaps alone do not cause renumbering. The preview suggests
the lowest free positive number for each eligible prefix. It checks the exact
displayed strings, not the internal placement numbers, and reserves locked and
out-of-scope references too. Custom references, such as `R2_A`, stay unchanged
and reserve only their literal text. If no number is available the row stays
unchanged and gets a `no_available_reference` error.

The editor and native file reader already refuse duplicate displayed designators.
If duplicate references reach the planner, `duplicate_reference` reports all
claimants when at least one is in scope; the planner leaves them unchanged.
This is not a way to open an invalid file or to reset duplicates.

Preview does not write files, change the project revision, add an undo entry,
clear redo or change any board link. Applying proposals, duplicate repair,
renumbering already numbered parts and a graphical review control are not
implemented here.
