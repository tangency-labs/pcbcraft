# Preview and apply schematic annotation

Use `annotate.preview` to review reference designators without editing the
schematic. It is a shared project command available in headless and live MCP,
including a read-only headless session, where `annotate.apply` is refused.
The command does not need a PCB. Its query result is returned even with
`inspect: false`:

```json
{"command":"annotate.preview","params":{},"inspect":false}
```

Pass `sheet` with the sheet UUID from project inspection to limit the report to
that sheet. Omitting it covers the whole project, independent of the active
view. The optional `expected_revision` rejects a preview of a more recent
project state. The reply contains `revision` and `annotation_preview`, with
`rows` and `errors`, plus an opaque `token` for applying that specific proposal
in the same session. The numeric `revision` describes the previewed state and
cannot authorize Apply.

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
clear redo or change any board link.

## Apply the reviewed proposal

Call `annotate.apply` with the `token` from the latest preview in the same
session. No scope or reference overrides are accepted; the token already binds
that proposal and its revision. A new preview replaces the previous token. A
successful apply consumes the token, even when there are no renames. Opening a
project again discards outstanding tokens. Desktop recovery records the edit's
resulting documents, not the token.

Apply refuses a proposal with errors or a revision that changed even if Undo
has since restored the same design. A stale-token refusal carries
`details.current_revision` and `details.part_instances`, with IDs for parts
added, removed, renamed, locked or unlocked, or with moved, rotated, mirrored,
added or removed units. An empty list means no such differences remain; it does
not make the token valid. Preview again before applying.

All proposed renames publish as one edit and one undo step; empty proposals do
not change the revision or history. Part instance IDs, placements and native
file formats are unchanged. Internal reference numbers follow the proposed
numbers when available. If an unchanged part has that internal number under a
manually edited reference, Apply assigns the renamed part a free internal
number rather than changing the unchanged part. There is no automatic save.
Duplicate repair, renumbering already numbered parts and a graphical review
control are not implemented here.
