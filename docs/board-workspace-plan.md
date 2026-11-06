# PCB workspace and board foundations

The first PCB milestone should support a small editable board in both a split
window and separate desktop windows. Both arrangements have equal priority.
They share one open project, command path, undo history and save lifecycle,
while each view keeps its own camera and editing context. Each window has its
own ribbon and contextual panels, determined by its focused pane; focusing a
PCB window does not replace another window's schematic ribbon. A split within
one window shares that window's ribbon, with an explicit active-pane indicator. Cross-probing connects
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
file controller, command history and service owner. Closing any window dismisses
its views while another editor remains; a survivor takes over project controls
without changing its layout. Closing the last editor or explicitly choosing Quit
uses one project-level **Save / Don’t Save / Cancel** decision. Layout transitions expire
old input without editing the document, and closed view/host IDs are never reused.
Highlighting and optional selection following reach compatible views across all
hosts while preserving edit selection, zoom and focus. The first PCB inspection
view now renders existing footprint pad centers, with schematic/board workspace
choices and component counterpart highlights. It shares the same single-sheet,
single-board session; board editing, individual pad/pin selection and live-MCP
presentation actions remain incomplete.

## Existing foundation

At `b028afa555bba85e2ad479d83c268aaf6e2bce8b`, the engine separates components
from schematic placements and supports a generic two-pad footprint with explicit
pin-to-pad lookup. The [storage format](native-project-storage.md) separates
`project.pc`, sheets, `board.json` and pinned symbol definitions while retaining
one runtime project. General footprint definitions and an editable board canvas
remain to be built; the inspection view does not infer pad shapes or drills.

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
| Named arrangement (planned) | Remembered window/pane/tab configuration and view presentation state for this project |
| View registry | Views addressed by stable runtime IDs, independent of their workspace target or host |
| Editor view | Stable runtime view ID, schematic or PCB target, camera, active layer, visibility and selection filters, local tool draft |
| Window | Stable host ID, pane layout, focused view, ribbon and panels anchored to that window |
| Split pane | View placement and sizing, sheet-path tabs, keyboard and pointer focus |

Moving a view between a pane and a native window retains its target and camera
without opening another copy of the project. Different projects may have
different sessions; two views of this project use the same authority. View
layout and navigation stay outside tracked electrical documents and electrical
undo history.

### Named arrangements and reopening a project

Following the November 6 UI discussion, named arrangements are part of the
workspace direction, including restoration when a project reopens. This changes
the earlier recommendation to keep only document tabs: top tabs have a separate
purpose when they select remembered arrangements. The current implementation
still has a project-title pill and one view per pane; arrangement tabs, tab
groups and editor-layout persistence are not implemented. App storage currently
retains preferences and the Assistant's layout only.

The proposed relationship is:

| UI location | Meaning | Example |
| --- | --- | --- |
| Plain project title and dirty indicator | The shared project, independent of the active arrangement | Controller |
| Top tabs | Named arrangements of that project's views | Design, Routing, Review |
| Tabs within a pane | Views of specific documents, with sheet paths or board names | Main, Power › Regulators, Controller PCB |
| Workspace picker | Editor type and placement for a document view | Schematic here, PCB beside, PCB in a new window |

Names such as Design or Routing are user labels, not fixed editor modes. An
arrangement can mix schematic sheets, boards and future provider-backed views.
A board is a peer document to a schematic, not a subview owned by that schematic.
Two views of the same document can have independent cameras while sharing its
data, undo history and correspondence. Multiple boards use their distinct board
IDs; neither an arrangement nor a tab implies a second project session.

The proposed arrangement scope is the whole project working setup, including
its detached windows. Selecting an arrangement explicitly restores that setup;
ordinary pane focus and document-tab clicks never switch arrangements or
reconfigure other windows. Each window keeps its own focused pane, ribbon and
panels, and shows the same active arrangement identity. The existing independent
window-close behavior remains in effect within the active arrangement. This
scope should be demonstrated in the UI before implementing arrangement switching,
because changing arrangements can intentionally affect more than one window.

Within an arrangement, a pane owns an ordered group of document-view tabs and
one active tab. Clicking a tab activates its view in that pane. Its **×** closes
that view, without deleting the document or closing another view of it. Removing
the last tab from a secondary pane collapses that pane; the last window retains
the existing project-level close decision. New, Duplicate and Rename arrangement
actions manage arrangements separately from document-view close controls.
Switching arrangements does not discard project edits or prompt to save them.
An arrangement remembers its latest state automatically; Duplicate creates a
separate starting layout over the same project data. Whether arrangement removal
uses a separate menu action or a top-tab close control remains a UI detail to
review, rather than reusing the lower **×** without a clear meaning.

An arrangement snapshot should capture the following presentation state:

- Stable arrangement identity, name, ordering and the last active arrangement.
- Window membership, pane splits and ratios, tab ordering, active tabs and focus.
- Stable document targets, including project, sheet or board identity, rather
  than display names or runtime view IDs.
- Per-view world-space camera position and zoom, grid state, active layer,
  visible layers and applicable filters or view orientation.
- Per-window panel visibility, collapse and placement, together with usable
  desktop window bounds and display hints.

The distinction between current and future editor capabilities still applies:
record only state a provider actually supports. In-progress tool gestures,
preview edits, modal dialogs, transient highlights, pending cross-probe requests,
transport handles and electrical undo history do not belong in this snapshot.
The project remains the authority for authored content; an arrangement cannot
restore an older version of a board or resurrect deleted geometry.

Use a versioned per-project record in user-local app storage for automatic
restoration. The project UUID identifies the design, while a local location or
copy discriminator keeps two checkouts of that UUID from overwriting one
another's arrangements. Paths are location hints, not document identities.
Unsaved projects keep their arrangement in memory; a successful first Save
associates it with the saved project. A successful Save As copies the current
arrangements to the destination's local record without deleting the original's
record. Cancelled or failed saves do not rebind either record. Portable,
explicitly exported arrangements can be added separately; automatic window and
camera updates must not rewrite `project.pc`, board or sheet files, change their
format versions, or create electrical undo entries. The exact local key and
schema need a bounded implementation review before adoption.

Keep the last successfully stored record until its replacement is written.
A layout-storage failure must leave editing usable and report that arrangement
changes were not retained, independently of whether the electrical project saved.

Save the outgoing arrangement before activation and checkpoint settled layout
changes during use, with a final flush on an accepted project close or switch.
Capture the working setup before native windows are torn down so shutdown does
not replace it with an empty arrangement. A cancelled close keeps the current
setup active. Restore only after the corresponding project has loaded and been
validated; a failed Open leaves the existing project and its arrangement intact.
An arrangement change must respect the owner of an open modal or project
transition. Prepare and validate the replacement before retiring its old hosts;
this is not a last-window project close. Activation failure must leave a usable
arrangement and the shared project intact.
Use fresh runtime host/view identities and invalidate old input when activating
an arrangement. The persistent arrangement and document identities must not
authorize a delayed gesture or request from a previous activation.

Restoration must tolerate changed circumstances. Clamp window bounds to available
displays and keep controls reachable after monitor or scale changes. Keep a
missing document or unavailable provider identifiable instead of silently
opening a different sheet, board or plugin view; permit closing or replacing
that unavailable view. If no usable view remains, offer the ordinary document
chooser. Invalid local state must not prevent opening a valid project, and an
unsupported newer record must not be overwritten by a fallback layout. A
provider's optional state needs a namespaced, versioned, bounded payload, with
unknown state preserved until the user explicitly removes that view. Restoring
layout state must not install or grant capabilities to plugins.

Browser restoration uses the same arrangement concepts within one browser tab,
without assuming automatic file access or permission to create other windows.
After the user reopens the project, unsupported detached hosts need an explicit
in-tab fallback that retains their document views and does not destroy the saved
desktop placement. Cross-device sharing and browser-tab coordination remain
separate from remembering a project's local arrangements.

The next view-management slice should establish arrangement and tab-group
ownership before adding storage, then prove the complete reopen flow through the
shared presentation handlers. These handlers also serve live MCP under #61;
headless sessions must not claim to have restored native windows. Acceptance
includes switching two differently named mixed-workspace arrangements, reopening
the last active arrangement, duplicate document views with independent cameras,
multiple native windows, a removed monitor, missing documents/providers,
malformed or newer local state, failed and cancelled Open/Save, stale queued
input, and unchanged electrical bytes, revision and undo history. Native window
restoration needs actual desktop qualification in addition to state and UI tests.

### Workspace selection and placement

The controls below describe the current inspection implementation. The named
arrangement design above adds tab groups and restoration; it does not require
splitting before opening a different workspace or a detached window.

The workspace picker should be the common entry point for changing what a view
shows and where it opens. Selecting **PCB layout** switches the focused view in
place. Each available workspace card contains a split icon and a desktop-only window
icon, with tooltips naming the target and placement, so either arrangement is
available directly from a single schematic view. Opening a board in another host
must leave the current schematic visible without briefly splitting or switching
it as an intermediate step. These choices now work for the inspection canvas;
**View → Detach** also works without splitting first.

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

The registry and per-window layouts support schematic and PCB inspection views. Layout
nodes refer to stable view IDs; the view's document binding is independent of
its placement. A render slot is exchanged synchronously while each host paints,
without changing another host's focus or creating a project copy. Closing a host
removes its views and invalidates delayed input, including pending fields dialogs.

Interaction tests exercise a main split with two detached windows, a split inside
one of those windows, docking into an existing split, shared edits/history,
highlight/follow fan-out, and closing one host without collapsing another.
Whole split arrangements survive a Split off/on cycle. Native platform
qualification remains separate from those input/render-callback tests. This is
runtime retention, not the named-arrangement or reopen persistence planned above.

The [representation foundation](pcb-representation.md) defines layer identity,
physical stackup, units and the proposed shared net/rule contracts before the
board renderer makes those relationships visible. The inspection slice adds a
board-addressed renderer, direct workspace/placement choices, a footprint Browser,
grouped Layers and read-only Properties following the FigCraft exports. PCB
views have a dedicated near-black canvas and subdued line grid, including with
light application chrome, and a Layout ribbon independent of schematic Capture. Copper
markers use visible layers; active layer, layer visibility and document ownership
remain distinct. The current model has no pad shape, drill, outline or copper
tracks, so the view shows pad-center markers rather than fabrication geometry.
Selection filters, Flip view and the broader layout ribbon remain future work.

The next authored-layout step is #92's pinned footprint definitions and mapping,
then outline and placement tools using shared commands. Before adding a normal
schematic-to-board editing workflow, #55 must replace immediate footprint removal
with reviewed updates. Live-MCP presentation discovery and dispatch remain #61;
this slice does not complete #54, #64 or #65.

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

Desktop windows share one process and one project owner. Closing any editor
window dismisses its views while other editor windows remain; only explicit Dock
moves them into another host. When the main window closes, a surviving native
window takes over project dialogs, Home and the Assistant while retaining its ID,
position, layout, camera and selection. The framework's original root shell stays
hidden to drive the shared services and immediate viewports; it has no editor
views and is not counted as an open project window. Closing the last editor, or
choosing File → Quit from any window, runs one save/discard decision. Cancel or
save failure keeps the last editor available. A save already in progress remains
owned and can finish after the original window closes. This supersedes both the
original redock-on-Close proposal and the interim original-window-closes-project
policy, following user review. Independent-process editing remains separate.

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

The latest explicit schematic or board selection supplies the
counterpart highlight, with amber ink distinguishing it from local blue edit
selection. Focus changes retain the source without emitting a selection event.
Both view bindings must resolve in the current project opening; deletion
prunes the live source selection, and reopening resets it. Same-sheet views share
symbol/wire identities; schematic/board views resolve component-to-placement
correspondence through the engine. Board-to-board highlights also require the
same board ID. Individual pin/pad selection and project-wide net identity remain
future work. Pending navigation captures both view input addresses
and the policy at selection. Revision changes, reopening, focus changes, modal
input, Home, a collapsed split, and changed preferences discard it. Target host
geometry is supplied before consuming the request, so selection coordination
does not depend on native-window coordinates. The native host supplies independent
input streams and invalidates gestures on focus/host changes. Board fitting and
following use resolved board coordinates through a separate geometry adapter.

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
issues to finish before the next step can start. The read-only inspection slice
now proves the mixed-view part of steps 4–5 ahead of authored placement tools.
Manufacturer profiles, impedance calculation and advanced setup dialogs remain
later additions; plugin interfaces and explicit SolidCraft links must remain
possible without making those additions prerequisites for the first editable board.
[#122](https://github.com/tangency-labs/pcbcraft/issues/122) adds the board-identity
requirement described in the [representation plan](pcb-representation.md#board-addresses-and-multiboard-migration).

| Step | Issue | Reviewable result |
| --- | --- | --- |
| 1 | #64, with #28 and #55 contracts | Separate project ownership from view state, replace the fixed pair with a view registry and per-window layouts, and prove a split plus multiple detached views; settle target addresses, assignment versus placement and pending board removals. |
| 2 | #76, #77 and #75 | Establish stable layer IDs and physical stackup with exact lengths, commands and persistence. Specify durable net identity and shared rule resolution before saving class assignments; expose unit-aware setup through a shared parser. See the [representation plan](pcb-representation.md) for implemented versus proposed contracts. |
| 3 | [#92](https://github.com/tangency-labs/pcbcraft/issues/92), with #28 | Pin an original footprint definition and mapping, following #118's symbol/footprint/part separation; assign before or during layout; reopen offline and reject incompatible mappings. |
| 4 | #54, with [#50](https://github.com/tangency-labs/pcbcraft/issues/50) and [#53](https://github.com/tangency-labs/pcbcraft/issues/53) | Board canvas, simple closed outline, basic layers, footprint placement/move/rotation, Properties, grid and selection controls, undo and persistence. |
| 5 | #64 and #65 | The same schematic and board views work in a split and in separate native windows, with independent cameras and bidirectional highlighting. |
| 6 | #55 | Preview and atomically apply a bounded schematic-to-board update; show unrouted connections as ratsnest lines with current membership. |
| 7 | #61 and #54 acceptance | Perform board edits through live MCP in the open project, then verify both views, history and save/reopen. |

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
- Name and switch arrangements, then close and reopen the project. Restore the last active arrangement's document tabs, splits, independent cameras, panels and supported windows from user-local state without changing electrical files. Exercise unavailable documents/providers and a changed monitor configuration without losing the saved arrangement.
- Select a component, pin, footprint and pad from either side. Highlight the exact counterpart without focus theft or selection loops; cameras stay put by default, with optional directed selection following that preserves zoom. Reveal counterpart navigates explicitly.
- Rename a reference and change placement without breaking correspondence. Exercise an unassigned component and reject an incompatible mapping without partial mutation.
- Undo and redo from either editor, then inspect through live MCP. Both views and MCP report the same revision and electrical state.
- Add and remove a schematic part. Preview board consequences, reject a stale preview, apply the current preview once, and undo it once. Unchanged footprints keep their placement.
- Save and reopen with the source library unavailable. Definitions, mappings, outline and placements survive; a no-op save produces no tracked-file diff.
- Exercise text input, a modal dialog, a second edit during a gesture, closing the original window before its peers, project replacement, and last-window close or explicit Quit with unsaved changes. No stale gesture or late request changes a replacement project.
- Run presentation actions through GUI and live MCP and compare view state. Confirm that the document revision, saved data, dirty state and history remain unchanged.
- Verify the board canvas and split arrangement in the browser. Record the desktop platforms on which separate native windows were actually qualified.

Use the existing engine, app, chrome and MCP boundaries, original fixtures and
the source rules in AGENTS.md. Any reuse from PCBcraft Next needs a bounded
source review; its recorded KiCad-source exposure and affected-work pause are
not resolved by this plan. Implementation checks include the repository's
standard checks, rendered interaction review and real native-window tests.
Follow the separate failure-path review for shared state, concurrency and I/O,
distinguishing tested failures from cases reviewed by inspection.
