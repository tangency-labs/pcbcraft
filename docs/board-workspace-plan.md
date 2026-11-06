# PCB workspace and board foundations

The first PCB milestone should support a small editable board in both a split
window and separate desktop windows. Both arrangements have equal priority.
They share one open project, command path, undo history and save lifecycle,
while each view keeps its own camera and editing context. Cross-probing connects
the views through explicit component and terminal identities.

This is the proposed architecture and acceptance plan for
[#54](https://github.com/tangency-labs/pcbcraft/issues/54), with editor hosting in
[#64](https://github.com/tangency-labs/pcbcraft/issues/64) and cross-probing in
[#65](https://github.com/tangency-labs/pcbcraft/issues/65). Schematic views now
share one session through **View → Split**, **New window**, **Detach** and **Dock**.
A view registry and per-window split trees replace the original fixed pair.
Multiple detached windows can coexist with a split, and detached windows can
contain splits too. Each host retains its focused view and stationary panels;
each view retains its document binding, camera, grid and edit selection. Sheet
tabs display project and sheet paths with chevrons, never runtime view numbers.
Tab menus can open beside, open a new window, detach, dock or close a specific view.

The native hosts borrow the same application on the UI thread, retaining one
file controller, command history and service owner. Closing a secondary window
dismisses its views without changing other layouts; closing the root uses one
project-level **Save / Don’t Save / Cancel** decision. Layout transitions expire
old input without editing the document, and closed view/host IDs are never reused.
Highlighting and optional selection following reach compatible views across all
hosts while preserving edit selection, zoom and focus. Board rendering,
schematic-to-board cross-probing and live-MCP presentation actions remain
unimplemented. The engine still exposes one sheet, so this hosting slice does not
yet offer a different workspace or sheet as the target of a new pane.

## Existing foundation

At `b028afa555bba85e2ad479d83c268aaf6e2bce8b`, the engine separates components
from schematic placements and supports a generic two-pad footprint with explicit
pin-to-pad lookup. The [storage format](native-project-storage.md) separates
`project.pc`, sheets, `board.json` and pinned symbol definitions while retaining
one runtime project. General footprint definitions and the board canvas remain
to be built.

The desktop Assistant already opens a secondary native viewport with access to
the same session. Editor focus, canvas input, repainting and close behavior need
their own qualification. At that baseline the app owned one schematic camera and
selection. Those fields now belong to explicit view state,
with persistent project/sheet addresses and input invalidation across focus
changes, history changes and file reopening.

PCBcraft Next's
[probe contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/docs/sheet-workflows.md#hooks-for-later-pcb-cross-probing)
and [workspace proposal](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/docs/specs/workspace-ui.md)
are behavior references; its board renderer and coordinated editor views remain
unfinished. Any implementation reuse needs its own provenance review under
[AGENTS.md](../AGENTS.md).

Travis's FigCraft top and bottom layout exports establish Browser and grouped
Layers on the left, Properties and selection filters on the right, and a
flip-view control. These visual references live in his local workspace under
`designs/exports/workspaces-2026-10-30/`; they are not repository assets or
electrical test fixtures. Camera side, visible layers and the active editing
layer are separate states. Viewing the underside does not move a footprint to
the opposite side of the board.

## Project and view ownership

| Owner | State and responsibilities |
| --- | --- |
| Open project session | Authoritative schematic and board data, components, definitions and mappings, revision, document commands, undo and redo |
| Project controller | Save and dirty state, project replacement and close decisions, live MCP attachment and coordinated selection |
| View registry | Views addressed by stable runtime IDs, independent of their workspace target or host |
| Editor view | Stable runtime view ID, schematic or PCB target, camera, active layer, visibility and selection filters, local tool draft |
| Window | Stable host ID, pane layout, focused view, ribbon and panels anchored to that window |
| Split pane | View placement and sizing, sheet-path tabs, keyboard and pointer focus |

Moving a view between a pane and a native window retains its target and camera
without opening another copy of the project. Different projects may have
different sessions; two views of this project use the same authority. View
layout and navigation stay outside tracked electrical documents and electrical
undo history.

### Workspace selection and placement

The workspace picker should be the common entry point for changing what a view
shows and where it opens. Selecting **PCB layout** switches the focused view in
place. The same workspace row should offer **Open beside** and desktop-only
**Open in new window** as explicit alternatives, so either arrangement is
available directly from a single schematic view. Opening a board in another host
must leave the current schematic visible without briefly splitting or switching
it as an intermediate step. These are planned interactions for the board canvas;
the current schematic-only **View → Detach** already works without splitting first.

| Choice | Result |
| --- | --- |
| Select Schematic or PCB layout | Change the focused view's workspace in its current pane or window |
| Split / Open beside | Choose a workspace and document for a new pane beside the focused view; duplicating the current target is an explicit choice |
| Open in new window | Create another view of the requested workspace in a new native window, leaving existing views in place |
| Detach an existing view | Move that view to its own window with its target, camera and selection intact |
| Dock an existing view | Move that view into a chosen window and pane position |
| Close a secondary window | Dismiss its views without changing the layout, cameras or selections in other windows |

Splitting must not force another view of the active workspace. The target chooser
can open the schematic beside the board, another sheet beside the current sheet,
or a second view of the same document. Opening beside or in a new window creates
a view; focusing or moving an existing view is a separate action. A matching
workspace elsewhere must not be silently moved or prevent another window from
opening. Multiple detached windows and a split can coexist, and a detached window
can itself contain a split. A new view of the same document starts at the source
view's zoom and position; a different document uses its own navigation state.

Switching a view back to a workspace restores that view's prior camera, layers
and filters for the workspace. A switch cancels unfinished input; it does not
replace the project or reset another view. Tabs identify sheet paths or the board,
while the focused view determines its window's ribbon and panels. Detach/Dock
should also be available from a view's tab menu so the target is explicit when
several views are visible.

Keep workspace target and host placement separate in the presentation handlers:
opening a target in the current view, beside it or in another window is distinct
from moving an existing view. GUI and live-MCP callers must use those same
handlers under #61. Browser builds offer in-place switching and Open beside;
native window actions are omitted.

### Hosting foundation and next #64 step

The registry and per-window layouts are implemented for schematic views. Layout
nodes refer to stable view IDs; the view's document binding is independent of
its placement. A render slot is exchanged synchronously while each host paints,
without changing another host's focus or creating a project copy. Closing a host
removes its views and invalidates delayed input, including pending fields dialogs.

Interaction tests exercise a main split with two detached windows, a split inside
one of those windows, docking into an existing split, shared edits/history,
highlight/follow fan-out, and closing one host without collapsing another.
Whole split arrangements survive a Split off/on cycle. Native platform
qualification remains separate from those input/render-callback tests.

The next workspace step is to add the board view state and renderer, then wire
workspace/document target choices into the shared placement handlers. The picker
interactions above are still planned: current Split/Open beside/New window actions
create another view of the addressed schematic. Acceptance must include schematic
beside PCB, plus another detached board view, before #64 is complete. Live-MCP
presentation discovery and dispatch remain under #61.

The focused view determines where editing shortcuts and contextual controls act.
Text fields and dialogs retain keyboard ownership. A tool gesture captures its
view and project revision; focus changes, undo, project replacement or a
conflicting edit must never commit it into another target. Document edits are
serialized through the existing session, and all affected views repaint.

Following [#61](https://github.com/tangency-labs/pcbcraft/issues/61), presentation
actions use the same GUI and live-MCP handlers. Inspecting, selecting, fitting,
splitting or moving a view must not dirty the electrical document, change saved
bytes, advance its revision or add a document undo step. Live requests identify
their target view explicitly. Headless sessions report that window actions need
a live editor rather than silently succeeding.

The proposed initial desktop policy has one primary project window and
detachable secondary editor windows in the same process. Closing a secondary
window dismisses its views; only an explicit Dock moves them into another host.
This supersedes the original #64 proposal to return a closed view to the primary
window, following user review of the unexpected split. Closing the primary
window uses the project's save or discard decision once and closes its secondary views.
Root-window promotion and independent-process editing can follow separately.
Browser acceptance covers switching and splitting inside one tab; separate
browser windows need another coordination design.

## Correspondence and cross-probing

Resolve correspondence through component, logical terminal, footprint and
physical pad identities. Reference text, sheet names and coordinates cannot
establish the link. Coordinate target addresses with
[#40](https://github.com/tangency-labs/pcbcraft/issues/40) and
[#41](https://github.com/tangency-labs/pcbcraft/issues/41), while allowing the first
visible fixture to use one sheet and single-unit components. Repeated sheets
and several symbol units belonging to one physical component must not change
the meaning of a component target.

Footprint assignment and board placement are separate facts. Pin the selected
definition, source revision and validated terminal-to-pad mapping so the project
reopens offline. Start with an original two-pad definition, while representing
missing or unsupported mappings explicitly. General mappings cannot be inferred
from the current hard-coded two-pad case or matching pin and pad numbers.

A user selection projects highlights into counterpart views without changing
their focus. By default cameras stay put; **View → Settings** allows all views
to follow selections, other views to follow primary, or primary to follow other views. These
roles remain stable across focus changes. Following pans only when the selected
bounds extend outside the usable canvas, preserving zoom; an oversized group is
centered without zooming out. A reveal is consumed once, and subsequent manual
navigation does not trigger another. Reveal counterpart across sheets remains
planned. Only the focused view’s actual edit selection receives destructive
commands. Receiving a highlight must not emit another selection event.

With more than two views, highlights project into every compatible visible target
without changing its edit selection. Pending follow requests address source and
destination views by stable ID, independently of which window hosts them. The
current global policies distinguish the original primary view from all others;
closing that view transfers the role to the main window’s focused view. Explicit
per-view follow links remain a refinement for mixed workspaces. Highlight-only is
the default, and closing or rebinding either endpoint invalidates pending navigation.

For the current same-sheet views, the latest explicit selection supplies the
counterpart highlight, with amber ink distinguishing it from local blue edit
selection. Focus changes retain the source without emitting a selection event.
Both view bindings must match the current project opening and sheet; deletion
prunes the live source selection, and reopening resets it. This handles symbols
and wire identities on one sheet, without claiming board correspondence or
project-wide net identity. Pending navigation captures both view input addresses
and the policy at selection. Revision changes, reopening, focus changes, modal
input, Home, a collapsed split, and changed preferences discard it. Target host
geometry is supplied before consuming the request, so selection coordination
does not depend on native-window coordinates. The native host supplies independent
input streams and invalidates gestures on focus/host changes. A PCB target still
needs component/pad correspondence and its own geometry adapter.

Distinguish unassigned footprints, assigned but unplaced footprints, unresolved
mappings and targets removed since selection. Represent one-to-many matches or
reject unsupported mappings explicitly. Net highlights use membership from a
specific revision and invalidate after connectivity changes; names or list
positions cannot preserve identity through arbitrary net splits and merges.
Reopening a project must invalidate stale runtime targets even when its saved
identity is unchanged.

## Reviewed board updates

Cross-probing does not apply schematic changes to the board.
[#55](https://github.com/tangency-labs/pcbcraft/issues/55) owns a reviewed update
that previews additions, removals, replacements and connectivity changes,
preserves existing placement and board-local objects, rejects stale plans and
commits an accepted change in one undo step.

The current `Session::delete_symbol` removes the linked footprint when it removes
the last schematic unit. Before exposing that action in a board workflow,
define how a pending removal retains the footprint and its correspondence until
review, including save, reload and undo. An accepted board baseline or another
explicit pending state may provide that representation; the storage schema is
still to be designed. Shared reference/value fields may update live, while
footprint removal and remapping require review.

## Delivery sequence

Use #54 to coordinate these deliverables, with small implementation PRs. The
order describes acceptance dependencies rather than requiring whole backlog
issues to finish before the next step can start.

| Step | Issue | Reviewable result |
| --- | --- | --- |
| 1 | #64, with #28 and #55 contracts | Separate project ownership from view state, replace the fixed pair with a view registry and per-window layouts, and prove a split plus multiple detached views; settle target addresses, assignment versus placement and pending board removals. |
| 2 | [#28](https://github.com/tangency-labs/pcbcraft/issues/28) | Pin an original footprint definition and mapping; assign before or during layout; reopen offline and reject incompatible mappings. |
| 3 | #54, with [#50](https://github.com/tangency-labs/pcbcraft/issues/50) and [#53](https://github.com/tangency-labs/pcbcraft/issues/53) | Board canvas, simple closed outline, basic layers, footprint placement/move/rotation, Properties, grid and selection controls, undo and persistence. |
| 4 | #64 and #65 | The same schematic and board views work in a split and in separate native windows, with independent cameras and bidirectional highlighting. |
| 5 | #55 | Preview and atomically apply a bounded schematic-to-board update; show unrouted connections as ratsnest lines with current membership. |
| 6 | #61 and #54 acceptance | Perform board edits through live MCP in the open project, then verify both views, history and save/reopen. |

View extraction can begin with schematic views; final acceptance uses the board
canvas. Live MCP work can progress alongside board commands. General hierarchy,
external library services, routing (#56), zones (#57), DRC (#58), manufacturing
(#59), 3D and KiCad interchange are not gates for the initial board workspace.
The project-local variant work in #28 remains part of library management but
does not block the first original footprint fixture.

The split host builds on the Browser, Parts and ribbon changes merged in
[#62](https://github.com/tangency-labs/pcbcraft/pull/62), together with #63's Home
startup. Native hosting should preserve these contextual controls when moving
the same view between a pane and a window.

## First acceptance project

Create an original resistor and two-pin connector on one schematic sheet, with
two explicit connections and compatible footprints on a rectangular two-layer
board. This is a behavior fixture, not a validated electrical reference design.

- Place, move and rotate a footprint through shared engine commands. Each completed gesture is one undo step; cancellation makes no edit.
- Switch workspaces, split schematic and board, and open at least two additional native editor windows without moving the existing views. Exercise a split inside a detached window, another view of the same board, explicit Detach/Dock, and secondary Close without an implicit split elsewhere. Preserve independent cameras and the same open project throughout.
- Select a component, pin, footprint and pad from either side. Highlight the exact counterpart without focus theft or selection loops; cameras stay put by default, with optional directed selection following that preserves zoom. Reveal counterpart navigates explicitly.
- Rename a reference and change placement without breaking correspondence. Exercise an unassigned component and reject an incompatible mapping without partial mutation.
- Undo and redo from either editor, then inspect through live MCP. Both views and MCP report the same revision and electrical state.
- Add and remove a schematic part. Preview board consequences, reject a stale preview, apply the current preview once, and undo it once. Unchanged footprints keep their placement.
- Save and reopen with the source library unavailable. Definitions, mappings, outline and placements survive; a no-op save produces no tracked-file diff.
- Exercise text input, a modal dialog, a second edit during a gesture, secondary-window close, project replacement and primary-window close with unsaved changes. No stale gesture or late request changes a replacement project.
- Run presentation actions through GUI and live MCP and compare view state. Confirm that the document revision, saved data, dirty state and history remain unchanged.
- Verify the board canvas and split arrangement in the browser. Record the desktop platforms on which separate native windows were actually qualified.

Use the existing engine, app, chrome and MCP boundaries, original fixtures and
the source rules in AGENTS.md. Any reuse from PCBcraft Next needs a bounded
source review; its recorded KiCad-source exposure and affected-work pause are
not resolved by this plan. Implementation checks include the repository's
standard checks, rendered interaction review and real native-window tests.
Follow the separate failure-path review for shared state, concurrency and I/O,
distinguishing tested failures from cases reviewed by inspection.
