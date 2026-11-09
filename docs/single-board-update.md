# Single board update review

The engine command `pcb.initialize_update_baseline` explicitly adopts the current
board's package choices and schematic connectivity as an accepted baseline. This
is the storage and retention foundation for [reviewed board updates](https://github.com/tangency-labs/pcbcraft/issues/55).
The engine also provides preview, Apply and Cancel commands for later schematic
differences, through headless and live MCP. The native Update PCB dialog uses
those same commands, and the [ratsnest](board-ratsnest.md) displays accepted
connections.

## Update dialog

Open **Capture → PCB → Update PCB** from the schematic, or **Layout → Schematic →
Update PCB** from the PCB workspace. Sheet and board Browser rows also offer
**Update PCB from schematic…** in their context menus. Live MCP can open this same
dialog with `app.update_pcb`, addressing the originating `view`; approval still
uses the engine commands below.

Opening or cancelling the review changes no authored data. If the project has no
board, **Create board** explicitly creates its first PCB as a separate undoable
edit. Existing footprints without an accepted baseline require **Use current
state**, which records today's logical/package inputs; it cannot reconstruct an
earlier accepted state. Cancelling a later preview does not undo either action.

The review lists component additions/removals, old/new footprint and mapping
details, changed connection groups and missing package choices. Assignment opens
the existing package chooser and returns to explicit re-review. Apply is disabled
for blockers, unchanged inputs or stale/replaced previews. Re-review stays within
the same opening and original document; reopening or replacing the source requires
closing the form and starting again. Cancel cannot revoke another client's newer
preview.

After **Apply update**, the dialog lists accepted unplaced footprints. **Place…**
opens the existing coordinate-based placement chooser in the originating pane's
PCB workspace. Reopening an up-to-date review offers the same unplaced list, so
placement can continue after saving or closing the dialog. Changing a package
choice from that handoff requires another explicit review and Apply. Updates
never invent overlapping board coordinates or move an unchanged footprint.

## Preview and Apply

`pcb.preview_update` takes `board` and `expected_revision`. It freezes the
current authored input and returns `board_update_preview` with an opaque
`plan_token`, captured source identity, complete component/connectivity changes
and package blockers. It changes no project bytes, revision, dirty state or undo
history. This query returns its requested data even when MCP uses `inspect:false`.
Ordinary project inspection includes a smaller cached-preview summary.
`can_apply` means the blocker list is empty, including for a no-op; native views
use `changes.is_noop()` separately when showing whether anything needs acceptance.

`pcb.apply_update` takes the same board, current `expected_revision` and the
preview's `plan_token`. It revalidates the immutable input, constructs the complete
physical candidate, checks native document budgets and publishes the candidate
with its matching baseline in one undo step. Unchanged footprint IDs, positions,
rotations, setup and board graphics survive. Added parts remain unplaced with
accepted assignments. Missing choices and unsupported mappings prevent Apply
without partial changes. A no-op consumes the token without changing bytes,
history, accepted capture identity or revision.

`pcb.cancel_update` takes `plan_token` and revokes only that runtime preview.
There is one cached plan per session. A new preview replaces its predecessor;
authored edits, undo/redo, opening another snapshot and save-name adoption
invalidate it. UUID capabilities cannot be replayed in another session even when
project IDs and numeric revisions match. Tokens and preview captures are never
saved; real acceptance persists the captured source without forcing a save.

Complete reports are bounded to 8 MiB, below the live reply budget. Larger
change sets are refused explicitly rather than returning a truncated review with
an Apply token. Native UI can borrow `Session::cached_board_update()` for the typed
report, including captured metadata for removed components; it still submits
Apply/Cancel through the shared commands.

A new board with no placements can preview against an empty accepted predecessor.
An existing placed board without a baseline needs explicit adoption through the
initialization command below, because its earlier accepted state is unavailable.

## Initializing accepted state

The command takes the addressed `board` and `expected_revision`, through the same
engine command path as other board edits. Every board-eligible component must have
a supported pinned physical assignment or an existing compatible legacy marker.
Unplaced components excluded from the board need no package. An excluded component
that is already placed is adopted with its actual physical package, so initialization
preserves that predecessor until a reviewed update removes it.
Missing choices, incompatible mappings and stale targets fail without changing
the project. Initialization is one undo step; a repeated invocation is a no-op
and does not accept subsequent schematic changes.

Without an enabled baseline, existing deletion and file-encoding behavior is
preserved. Once enabled, deleting a component through single-symbol, group or
sheet deletion removes its live schematic representation but retains accepted
footprints. Board views render those footprints using historical component
fields when the live source is absent. Inspection reports `pending_removals`,
and each retained footprint exposes its `accepted_component` and
`pending_removal` state. No missing live symbol is fabricated for cross-probing.

Reference/value fields follow the live component while it exists. At deletion,
their last visible values become descriptive display metadata for the retained
footprint. This overlay changes neither accepted terminal/package mappings nor
connectivity or source identity, and is not an authority for reconstructing the
captured source schematic. Other physical edits share the immutable baseline
across undo snapshots; the display overlay copies it only when values differ.

Unchanged accepted assignments can still be placed, moved and rotated. Package
selection in baseline mode stages desired intent without changing accepted
geometry. Pending choices must be applied before placement. Pending footprint
removal is not performed by calling initialization again. Ordinary explicit PCB
footprint deletion remains available
and undoable; accepted history and its package assets remain until reconciled.

## Physical membership and exclusion

This single-board slice forwards all live components except those with
`exclude_from_board`. Population, BOM, simulation and position-file filters do
not change physical membership. Connectivity is resolved across the complete
schematic before excluded terminals are removed from the board input, preserving
connections that meet at an excluded terminal.

With an accepted baseline, excluding a placed component stages a physical removal;
the footprint retains its pose and is marked `pending_removal` until Apply.
Uninitialized legacy boards continue to refuse exclusion of placed components.
Excluded components can retain or change a desired package choice, with the same
live-symbol and pinned-pad validation as other choices. That intent stays saved
with its assets, without becoming a pending physical package change. Apply removes
the accepted physical part while preserving its live excluded choice. Unexcluding
it later proposes an addition using that choice, which remains unplaced after Apply.

## Persisted records

Board format `pcbcraft-board/10` requires a `baseline` object. Earlier board
formats reject that field and retain their previous no-op encoding until baseline
initialization. Setup, silkscreen, graphics, legacy outline, assignments and
placements retain their existing records alongside it.

Board format `pcbcraft-board/11` is used while a desired `assignments` entry
for a board-eligible component differs from accepted history. Excluded-only choices
remain in version 10. The entry is the sole package intent; its mapping
must cover the current live component and exact pinned package. The baseline and
placement retain the accepted package/mapping independently. Inspection reports
`pending_update`. An older reader rejects version 11 instead of equating desired
and physical choices. Apply adopts those choices and returns to version 10 when
no pending package intent remains. Resetting intent to the accepted package is
also a normal desired-assignment edit.

| Baseline field | Meaning |
| --- | --- |
| `format` | `pcbcraft-board-baseline/1` |
| `project`, `board` | Explicit owners; both must match the loaded design |
| `source` | Frozen source `capture_id` UUID and lowercase SHA-256 `content_digest` |
| `components` | Canonical positive numeric ID keys, with historical symbol identity, reference/value, sorted terminal tokens and accepted package |
| `nets` | Canonically ordered accepted component/terminal groups and authored names; these are historical membership, not persistent net IDs |

Each physical package keeps its pinned library/item/revision and exact
terminal-to-pad mapping. A legacy marker has a null library and its explicit
two-pad mapping; it cannot become an unplaced catalog package by implication.
Historical component records need no live schematic placement or installed symbol
library to validate their accepted terminal set. When the exact symbol identity and
revision are still live, their terminal sets must agree. Physical assets remain pinned
while any placement, assignment or accepted record references them.

Proposed inputs may have unresolved package choices so a preview can report
blockers; persisted accepted board records require a package for every component.

The baseline has at most 4096 component records with at most 128 terminal tokens
per component, within a 16 MiB serialized bound and the existing project/document
budgets. Validation rejects duplicate/noncanonical numeric keys, foreign owners,
unknown or multiply assigned terminals, invalid capture addresses and incompatible
package mappings. Named single-terminal groups can retain their display names;
unnamed isolated terminals are not stored as connections.

Source capture hashes the complete canonical pre-Apply bundle, including prior
accepted history and current desired intent. The new capture is attached only
after hashing, so it is not self-referential. Hash documents in sorted path order,
beginning with
`pcbcraft-board-source-bundle/1` followed by a NUL byte. Each document contributes
its UTF-8 path byte length as unsigned 64-bit big-endian, path bytes, document
byte length with the same encoding, and exact document bytes. A fresh capture
UUID distinguishes byte-identical sources captured from different allocation
histories. The source address does not promise that a full historical schematic
snapshot remains available; the retained baseline supplies the information needed
to inspect the accepted physical state.

## Following slices

The engine normalizes all-sheet connectivity to stable component/terminal
addresses without persisting every derived net as a stable ID. The native review,
unplaced-part handoff and ratsnest use that accepted state; routing and
manufacturing readiness remain separate milestones.

Broader design boundaries are recorded in the [circuit instance contract](circuit-instances.md#package-intent-and-physical-acceptance)
and [storage design](multiboard-storage.md). Repeated circuits, multiple boards,
partial-use assignment and general net rules are not implemented by this foundation.
