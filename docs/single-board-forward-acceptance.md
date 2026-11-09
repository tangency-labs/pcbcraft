# Single-board forward-update acceptance

The command-driven regression in
[`crates/storage/tests/board_forward_update.rs`](../crates/storage/tests/board_forward_update.rs)
exercises one continuous schematic-to-PCB workflow across the engine and real
native storage APIs. It runs separately for two flat sheets joined by explicit
global labels and a parent/child hierarchy joined through a sheet pin and child
label. These are supported single-use circuits; repeated circuit instances and
multiple boards remain outside this acceptance case. A second scenario checks
physical membership independently of population flags across reviewed updates
and offline reopen.

Run it from the repository root:

```sh
cargo test --locked -p pcbcraft-storage --test board_forward_update
```

Every authored circuit, package choice, placement, graphic and update is created
through the shared revision-checked commands. The test chooses original pinned
packages from the production catalog and generates folder/archive contents with
the production encoder. It does not hand-edit project documents or depend on an
installed external library. Files live in temporary directories and are removed
after the test.

| Phase | Automated acceptance |
| --- | --- |
| Capture and reopen | A new project is schematic-only, with no board target or `board.json`; both sheet organizations survive folder and archive reopen. |
| First update | Explicit board creation, incomplete package choices, a blocker report, refused Apply, Cancel, a fresh valid preview and first Apply use the shared commands. Preview and refusal retain the exact immutable snapshot; Cancel preserves authored bytes. |
| Physical layout | Accepted parts start unplaced, are placed explicitly, and resolve board-qualified pin/pad correspondence in both directions. Moving/rotating one package and adding a Board Outline graphic establish the layout that later updates must preserve. |
| Pending changes | Add a part, change a placed package and terminal mapping, change cross-sheet wiring and remove a schematic part. Accepted airwires and existing physical geometry remain unchanged while removal and desired package intent are pending. |
| Offline review | Pending intent and retained footprints survive actual folder/archive reopen, with pinned geometry available and no runtime preview token restored. |
| Reviewed Apply | One Apply adopts the reviewed package, mapping and connectivity, removes the retained footprint, leaves the new part unplaced and preserves existing placement IDs/poses and board graphics. |
| Failure paths | Reusing a preview after an authored edit and choosing a duplicate pad mapping both fail without replacing the current snapshot. |
| History and no-op behavior | Undo restores the complete pending update, Redo restores the accepted result, and an unchanged preview/Apply preserves bytes, snapshot identity and revision. |
| Physical membership | An excluded, unplaced middle terminal preserves the connection between eligible parts without requiring a package. DNP parts still need packages and stay physically present. Clearing board exclusion introduces a package blocker, and assignment/Apply permits later placement without moving existing footprints. |
| Native no-diff saves | Reopened folder documents match exactly; a second save stages no payload or journal. Archive reopen reproduces the same canonical documents and archive bytes. |

The test exercises real filesystem folder publication and native archive encoding
and reopening. It does not simulate an interrupted save, operating-system process
restart or desktop archive publication failure. Coverage of those failure paths
needs separate evidence. It also does not establish that GUI review controls,
window focus, live transport or browser file handling have been exercised by a
person.

For desktop/browser acceptance, follow the same lifecycle through the actual
review UI: create a schematic-only project, create its board, resolve package
blockers, cancel and Apply, place parts, then make and review a package, connection
and removal change. Check the pending state after reopening before Apply, and
check Undo/Redo afterward. In split or detached views, an edit must expire an older
review; cancelling a move or review must leave authored state unchanged. The
ratsnest toggle belongs to each view and must remain reachable at compact sizes.
Record the tested build, platform and actual actions separately from the automated
result; this document does not claim that manual acceptance run is complete.

A passing run exercises a placeable, updatable single physical board through the
engine and storage APIs, including accepted connection membership and physical
pad addresses. The ratsnest is a deterministic display approximation, not routed
copper. Track/via routing, electrical/design-rule
verification, copper zones, manufacturing outputs and fabrication readiness
remain later work.
