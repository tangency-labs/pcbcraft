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
| Named arrangement | Remembered window/pane/tab configuration and view presentation state for this project |
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

Named arrangements now separate the project's working setups from the documents
shown inside them. The title bar contains the project name and dirty indicator,
followed by arrangement tabs and a single **+** for creation. Every project starts
with **Views**; previously saved arrangement names stay intact. Hovering a tab
explains that it represents the project's workspaces, split panes and windows.
Right-click a tab for **Rename…**, **Duplicate…** or **Delete…**.
Context actions address that tab by ID without activating it first: renaming or
deleting an inactive arrangement leaves the working layout and windows in place.
Duplicating an inactive tab activates a copy of that tab’s saved setup.
These are user names, not fixed editor modes, and all arrangements share the same
project, edits and undo. Naming forms stay in the initiating window, keep invalid
or duplicate names editable, and support Enter, Escape and explicit buttons.
Closing that editor discards its unfinished form.

| UI location | Meaning | Example |
| --- | --- | --- |
| Project name and dirty indicator | The shared project | Controller |
| Top tabs | Named arrangements of its windows and panes | Views, Routing, Review |
| Split pane header | The document currently shown in that pane | Controller › Main, Controller › Board |
| Browser sheet selection | Show that sheet in the focused pane | Main |
| Workspace picker | Open a document here, beside it, or in a native window | PCB layout |

Selecting a Browser sheet changes the focused pane's document while remembering
its prior navigation. It adds no document tabs. A view of that sheet elsewhere
stays independent, and the top arrangement tab
stays selected. The board Browser includes the current schematic sheet so this
navigation works from either workspace. The engine currently exposes one editable
sheet and one legacy board; the typed targets carry sheet and board identity,
but the UI does not imply that hierarchical or multiple-board editing has shipped.
Future sheet headers should show occurrence paths such as **Power › Regulators**.

A split pane has one header for its current document. Its **×** closes the pane,
including its cached document navigation, without deleting a document or closing
other panes. Detach and Dock move the pane with that navigation intact, rather
than uncovering a previously visited document. The main host retains one pane.
Native window Close still dismisses only that window while another survives, and
the final window uses the project's save decision. A single pane keeps the
compact canvas breadcrumb with no header strip, including after workspace changes.

The arrangement covers the whole project working setup, including detached
windows. Selecting a different top tab explicitly restores that setup and can
change the window set. Ordinary document selection and pane focus do not change
other windows. New defaults to the current views and windows, with an explicit
choice to start with one view of the main host’s focused document. Creating a
copy of the current setup and renaming preserve the live host and view identities;
they do not recreate native windows. Each arrangement automatically remembers its
latest layout. Deletion asks for confirmation and is disabled for the only
remaining arrangement; it does not remove electrical data. Cancelled forms leave
the saved collection, live layout and project history unchanged.

Version 1 records contain window membership, split ratios, cached documents, active
views, focus, stable document targets, both workspace cameras and grids, board
layer visibility/selection, and supported panel visibility, collapse and
placement. Desktop bounds are recorded separately from cameras. Runtime view
and host IDs are freshly allocated on activation. Gestures, selections,
highlights, pending probe requests, modal dialogs and electrical history are
excluded, so restoration cannot grant a delayed edit authority or roll back
project data. Tree-row expansion, provider payloads, view orientation and filters
are not yet persisted. The internal `tabs` record retains its original serialized
name for compatibility; it now stores a pane's document navigation cache and does
not introduce a tab strip in the interface.

Records live under `pcbcraft.project_arrangements` in eframe's user-local storage,
not in the electrical project. Desktop keys combine the project UUID with the
normalized saved path's OS bytes, encoded without lossy Unicode conversion.
Separate checkout paths therefore have separate records. In the browser the
key uses the project UUID, because the file picker provides no durable location;
copies with the same UUID share that browser-local record. Unsaved projects keep
arrangements in memory until a successful first Save. Successful Save As copies
the collection to the new location while retaining the original record; failed
or cancelled saves do not rebind it. Portable export and cross-device sharing
remain separate future features.

Checkpoints capture the outgoing setup before project or arrangement transitions
and after settled frame changes. Storage is flushed after pointer release or a
one-second change interval, and through the normal application save hook.
Restoration follows successful project load; failed Open leaves the existing
arrangement active. Modal dialogs and pending file/project transitions block
arrangement actions. Validation happens before replacing the live host graph,
and stale requests are rejected by project-opening and view input identities.

The store is bounded to 8 MiB and 128 projects, with up to 32 arrangements per
project, 16 windows and 128 views per snapshot. Invalid or newer records are
preserved instead of being overwritten by fallback state; missing documents
remain identifiable, closeable placeholders that cannot authorize edits. Size
and validation failures leave the project usable. Eframe's storage API does not
return write/flush errors, so disk or browser-storage failure reporting is still
a limitation rather than a confirmed durability guarantee.

Native restoration constrains size to the reported monitor and rejects positions
it cannot establish as reachable, leaving those windows at their OS-selected
location. Egui does not expose a full monitor map here, so exact placement across
negative-coordinate or secondary monitors is not yet qualified. Wayland does not
provide window positions through this integration, so the current bounds record
cannot capture its native window geometry; the view arrangement and cameras still
restore, while the compositor chooses window placement. Browser-native
window creation is unsupported. If a record containing native hosts reaches the
browser, it is preserved and a usable local layout is opened; flattening that
record into browser panes without losing desktop placement remains future work.
Unknown plugin schemas likewise require a future versioned provider contract;
restoring arrangements neither installs plugins nor grants capabilities.

The shared presentation handlers own navigation and layout intent independently
of widgets. They are suitable for a future live MCP adapter under #61, but this
slice does not expose new MCP tools, and a headless session cannot restore native
windows. Acceptance checks cover mixed arrangements, local document navigation,
independent cameras, tab close/detach, production save/reopen, cancelled/failed
file operations, missing targets, malformed/newer records, stale input, and
unchanged electrical bytes and revision. Native-window verification on Linux
X11 with software rendering exercised three
hosts, switching to a one-host arrangement and back, and restoration of a moved
child's bounds. The real X11 app also retained the second window's native ID,
position and size after closing the original with both displaying Main, then
handled Cancel and a PCB workspace switch. The isolated Wayland lifecycle test
uses real native windows, while the full-app automation there did not complete.
The arrangement-control rework was also exercised in the real X11 app: Rename
and New kept both native IDs and window bounds, and Rename continued to work in
the surviving editor after the original closed. UI regression tests cover form
cancellation, invalid/duplicate names, compact controls, detached form ownership
and closing the form’s owner. The failure-path review also traced project
replacement, queued actions, layout limits and last-window save handling.
Windows, macOS and physical multi-monitor placement are unqualified.

Backend selection remains unchanged, including native Wayland. The local
[eframe lifecycle patch](../vendor/eframe/PATCH.md) separates its logical root
viewport from the native window. Closing the original editor releases that
window's native resources and continues driving the shared session through the
remaining editors, which retain their own host, viewport and OS window IDs.
No editor contents move between windows, and no hidden original window remains.
Selecting an arrangement can recreate a root window when its layout needs one.
Native regression runs on isolated Wayland and X11 displays verify original
window destruction, survivor identity/rendering, root recreation and shutdown.
Headless application tests additionally cover matching/mixed documents, tabs,
cameras and the last-window save/cancel policy. Windows and macOS use the same
lifecycle path but have not been verified on those operating systems.

### Workspace selection and placement

The controls below describe the current inspection implementation. Opening a
different workspace or a detached window does not require splitting first.

The workspace picker should be the common entry point for changing what a view
shows and where it opens. Selecting **PCB layout** switches the focused pane to
the board. Each available workspace card contains a split icon and a desktop-only window
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
runtime retention as well as the named-arrangement persistence described above.

The [representation foundation](pcb-representation.md) defines layer identity,
physical stackup, units and the proposed shared net/rule contracts before the
board renderer makes those relationships visible. The inspection slice adds a
board-addressed renderer, direct workspace/placement choices, a footprint Browser,
grouped Layers and read-only Properties following the FigCraft exports. PCB
views have a dedicated near-black canvas, including with light application chrome,
and a Layout ribbon independent of schematic Capture. The PCB grid is hidden by
default; Preferences → Appearance offers Off, Dots and Lines across all PCB views
and windows, independently of snap spacing. Copper
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
position, layout, camera and selection. The original native window is released;
the framework's logical root continues to coordinate shared services and
immediate viewports without an OS surface. Closing the last editor, or
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
- Name and switch arrangements, then close and reopen the project. Restore the last active arrangement's document navigation, splits, independent cameras, panels and supported windows from user-local state without changing electrical files. Exercise unavailable documents/providers and a changed monitor configuration without losing the saved arrangement.
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
