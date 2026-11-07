# Editor guide

For a first session, start with the [quickstart](quickstart.md). This guide
covers the desktop and browser controls in more detail; the
[capability table](capabilities.md) records current limits.

## Home and recent projects

The app starts on **Home**, with compact **New Schematic**, **New PCB** and
**Open Project** cards. New PCB opens an empty Untitled project in PCB layout;
New Schematic opens its schematic. Both use the usual unsaved-changes prompt
before replacing an open project. On desktop,
**Recent projects** lists the last 12 successfully opened or saved projects, newest
first, as cards with a rendering of the saved schematic, its name and path. Click
a card to reopen it; unsaved work gets the
usual Save/Don’t Save/Cancel question. Remove a card with **×**, or choose
**Clear recent projects**, without deleting any files. History is kept between
launches in application settings. A missing or unreadable project reports an error
and stays in the list until removed. Empty and unavailable previews have
placeholders; previews load in the background without opening or changing a project. Browser uploads have no reusable local path,
so recent projects are desktop-only. Home does not automatically reopen a project.

## Preferences and grid

The app bar shows the project name and an unsaved marker beside **Undo**, **Redo**, and **Save** (**Download** in the
browser). The gear (or **Ctrl+,**) opens **Preferences**, with
pages for Appearance (theme, interface scale, PCB grid, grid opacity, units), Navigation (whether the wheel
zooms or pans, zoom direction, speed and anchor), Views (selection following),
a list of Shortcuts and, on the desktop, the AI page for the Assistant.
The PCB canvas starts with its grid hidden; **Appearance → PCB grid** chooses
**Off**, **Dots** or **Lines** for all PCB views and windows. This choice is remembered
between sessions and leaves snapping unchanged. **Grid opacity** fades visible grids
from 100% down to hidden at 0%, with **Reset** to go back; parts still snap to the
same points. The opacity applies across views and windows and is kept with your
preferences, not in the project.
The right end of the bottom status bar shows the grid point under the pointer (**X**, **Y**,
with Y increasing down), the **Grid** button and the display unit. Click the unit, or press
**Ctrl+U** (**⌘U** on macOS), to switch between **mil** and **mm**; the Appearance page
chooses it too. Switching only changes how lengths read: geometry, grid spacing and the
project stay exactly as they were, with no undo step. 100 mil is exactly 2.54 mm.
The **Grid** button sets the focused pane's snap spacing:
100, 50 (the default, where symbol pins sit), 25, 10 or 5 mil, or any other spacing
typed with or without a unit (`20`, `20 mil`, `0.5 mm`, from 0.01 mm to 1000 mil). A bare
number is read in the display unit, and a typed unit overrides it.
**N** steps the focused pane finer and **Shift+N** coarser. A new spacing never moves
or rescales anything already drawn, and pins stay reachable at any spacing because
wires attach to them by clicking. When zooming out thins the dots, the button says
how far apart the drawn dots are (for example *Grid 10 mil · dots 50 mil*). Each pane
keeps its own spacing through focus changes, splitting and reopening files; the last
one chosen is kept with your preferences, not in the project.
Search finds any setting; changes show at once, and **Cancel** puts them back. The house button beside the brand opens
**Home**, with New Project, Open, Save, Preferences and About in its rail. Press
**Esc** or choose **Back to** the project to return to the schematic, which stays
open with its history while Home is shown. See [VISION.md](../VISION.md) for the
project's direction.

## Tools and parts

The schematic ribbon follows PCBcraft Next: **Capture**, **Analyze**, **Libraries**,
**Manage** and **View**. Capture holds drawing and editing tools, including subsheets and hierarchical
labels. Analyze holds checks and reports, Libraries holds source management,
Manage holds independent sheet creation and project operations, and View holds
panels and editor layout. Unavailable tools are disabled;
compact windows keep them in the group caption menus. **Fit** and **Net Inspector**
are also available beside the tabs; View contains their full controls.

Choose **Capture → Symbol** or press **A** to open the **Parts** browser and focus
its search. It lists schematic symbols from
[`libraries/starter`](../libraries/starter): a resistor, LED, capacitor, two-pin
connector, 4-pin header and NPN transistor. See [docs/symbol-library.md](symbol-library.md) for the file
format. Type to search, choose Small, Medium, Large or List, and click a part (or
press **Enter** for the first match) to place it on the grid, or drag parts
straight onto the sheet. Press **R** to rotate a click-to-place preview before placing it,
or **Esc** to leave search focus or cancel placing; the browser stays visible.
New resistors, capacitors, LEDs and two-pin connectors use shorter leads from
revision 2 of the starter symbols; saved
revision 1 parts retain their original geometry.

The **Parts library** floats beside the Browser without resizing or panning the
canvas. Drag its title to move it; releasing near either edge snaps it there.
The **…** menu also offers **Float**, **Dock left** and **Dock right**. A short
dock uses a scrollable list, and returning to a taller card restores the chosen
preview size. **Esc** cancels a panel drag before leaving search or cancelling
placement. Close the library with its **×** button or **View → Parts**; reopening
keeps its search, preview size and position. Choosing a part leaves the card open
for repeated placement. **Fit** and Browser selections keep their targets clear of both floating cards.
The header chevron collapses Parts to its title bar and expands it again, keeping
the search, preview size and position. **A** also expands it and focuses search.

## Moving symbols

Press **M** with a symbol selected, or hover a symbol with no selection, to pick
it up while leaving its wiring in place. Click to place it elsewhere, or press
**Esc** or right-click to cancel the preview. **Disconnect and Move** in the
symbol's context menu performs the same action.
The move and disconnection share one undo step; placing a replacement at the vacated
pins connects it to the preserved wire ends. Ordinary symbol dragging keeps wires attached.

## Drawing and editing wires

Click an unconnected symbol pin or free wire end to start directly. Press **W** to
start immediately at the pointer, including on a connected pin, junction, wire segment
or empty grid point. **Capture → Wire** instead waits for the first canvas click.
The green crosshair marks the drawing point; a ring marks an existing connection
target. Hold **Ctrl** to bypass the placement grid while drawing; releasing it restores
snapping. Pins and wire connections still use their exact positions. Ordinary clicks on connected nodes retain selection behavior.
Click to add corners, **Space** to flip the current
elbow, and **Backspace** to remove the last corner. Click a destination pin or endpoint
to finish, or press **Enter** or double-click to finish on empty space. **Esc** or
right-click → **Cancel** discards the draft. Click a completed wire segment to select it;
only that segment turns blue, and **Delete** removes it. Drag a segment perpendicular to its
direction to adjust alignment, or hover it and press **G**, then click to finish. Attached pins
and junctions stay fixed while the adjoining legs adjust. **Esc** cancels the preview.
Right-click → **Select Wire** selects the whole route for dragging or deletion; box and
modifier selections continue to select whole wires. Crossings stay disconnected.
Select a wire segment, then use Capture → **Modify** for **Slice**, **Heal**
or **Delete** in the group caption menu.
Right-click a wire for these edits and whole-wire selection/deletion:
**Slice** splits it at the grid point under the pointer and keeps it connected, so a new wire
can start from the slice point. **Break**, available in this context menu, detaches the
leg under the pointer from neighboring legs, pins and shared junctions at either end.
It works with one or both ends attached; an already-free end stays free. The detached
straight piece moves in both directions when dragged, carrying its ends with it.
Its old contacts stay disconnected during departure. Once pulled clear,
dropping it back onto those contacts reconnects them, including within the same drag.
Ordinary drags of connected segments still anchor pins and shared junctions.
**Delete Segment** removes just that leg,
and **Delete Wire** the whole route. Hovering a row previews what it changes. An unconnected wire end shows a small square, a
point where three or more wires meet shows a dot, and two wires meeting end to end read as one
line. Agents use `schematic.slice_wire`, `schematic.break_wire` and
`schematic.delete_wire_segment` with a point on the wire.
`schematic.move_wire_segment` takes `wire`, `at`, `to` and `expected_revision`;
`to` moves the point at `at` in both axes for a detached straight wire, or perpendicular
to the segment when its ends are anchored. It uses the same
geometry as the canvas preview and commits one undo step. Optional `connect_ends` names
up to two of that wire’s degree-one junction IDs to join to exact stationary pin or wire contacts;
ordinary geometry-only commands omit it. These commands are available in
headless MCP and through the live desktop connection.

Selecting a segment also shows square handles on its wire’s corners and free ends. Drag a handle
to reshape it in place: a straight segment slides across its direction, a corner drags the
two legs that meet there, and a free end or junction drags the end leg of every wire attached
to it. The wire's other ends stay put, gaining an orthogonal leg where needed; pins never leave
their parts. Dropping a free end or junction on a highlighted pin, endpoint or wire segment connects
every wire at that junction to the target, using its exact position even on a different grid.
A segment target creates a branch automatically. Ambiguous contacts are refused.
Moving or placing a part onto a wire connects its touching pins; placing a two-pin part
across one straight run removes the span between its pins, inserting it without a short.
Moving a whole wire onto a part also connects an exactly touching endpoint to its pin.
Drags move by the grid's snapped travel, so an off-grid wire retains its offset until attached. Release to commit one undoable edit;
**Esc**, releasing outside the sheet, or an edit made meanwhile cancels it. Multi-item
selections and explicit **Select Wire** selections still move whole routes. Agents use `schematic.move_wire_segment`,
`schematic.move_wire_corner` and `schematic.move_junction`. For an explicit connection, use
`schematic.attach_junction` with `junction`, a pin/junction reference or
`{"wire":"wire-N","at":{"x_nm":0,"y_nm":0}}` as `to`, and `expected_revision`.
The same wire/point target works with `schematic.draw_wire`.

Completed draws and moves heal positive-length collinear overlaps, doubled-back legs,
duplicate strokes and redundant straight-through junctions in the affected routes.
When dragging folds a stepped route back over itself, obsolete split corners disappear
instead of becoming dangling stubs; original free ends, pins and branches stay connected.
When a route returns to an already connected straight wire through a U-shaped detour,
healing slides the junction to the nearer intersection without passing another branch
or moving a pin. Self-crossing returns also shorten. When a new route reconnects to
its existing net, cleanup keeps the new drawing gesture and removes the older
alternative only when it has no intermediate pin or branch. Manual cleanup and
geometry-only moves retain the shorter-route policy.
While drawing, retracing the draft shortens its preview immediately, including a
reversal that runs past an earlier corner. Backspace restores the previous clicked
corner, and finishing keeps the shortened route in one undo step.
The Modify menu's **Auto-heal wires** toggle defaults on and is remembered as a
user preference. Turn it off to keep detours and loops during editing, then use **Heal**
on selected wires when you want cleanup. Commands that draw, place, move or attach
support the optional `auto_heal` boolean with the same default; `schematic.heal_wires`
always performs cleanup.
Crossings and nearby geometry remain disconnected. **Heal** applies the same cleanup
to selected wires and connects old endpoints that exactly touch a pin; agents use
`schematic.heal_wires` with `items` and `expected_revision`. Healing is part of the
initiating edit's undo step, and opening a file never performs cleanup. Ambiguous pin
contacts, unsupported connections and overly complex edits are rejected as a whole. This
local cleanup does not route around obstacles or resolve conflicting net names
(named nets are separate work).

## Symbol properties and selection

The schematic **Properties** card shows the focused selection's reference,
value, orientation and footprint. **Apply changes** commits a draft in one undo
step. **More properties** or **E** opens the expanded General, Pin functions and
Library form; **Cancel** leaves the project unchanged. **View → Properties**
shows or hides the inspector for that window. Footprint choice opens a responsive
package browser with a physical preview, explicit terminal mapping and expandable
source/license details. Library-definition authoring remains separate from changing
an instance's properties.

Press **R** over a symbol to rotate it, or select it first. With a symbol selected,
**E** opens its expanded properties and **Delete** removes it with its wires.
Right-click a symbol for the same actions, and press **Esc** to clear the selection.
Reference and value belong to the component; position and rotation belong to
its placed unit. Native formats 2 and 3 record these separately, and opening a format 1
project migrates it on the next save. Authored wire routes use format 3. Footprints link to the component.

With no placement or wiring tool active, **left-drag empty canvas** or **right-drag**
to select a group. Left-to-right draws a solid **Window** box and selects fully
enclosed symbol bodies and wire paths; right-to-left draws a dashed **Crossing**
box and selects touched objects. The box shows the hit count. **Shift** adds to
selection and **Ctrl** toggles items, with either boxes or individual clicks.
Drag a selected member to move the group, or press **Delete** to remove it;
each operation is one undo step. Connected pins on unselected components stay
attached. **Esc** cancels a drag or clears selection, and a stationary right-click
still opens the context menu. Selection alone does not modify the project.

Scroll to zoom or drag with the middle mouse button to pan. Use **Fit** to bring
the circuit into view and **Nets** to open the Net Inspector.

## Project Browser and sheets

The **Browser** floats over the left of the canvas and shows the project as
**Sheets → Main → Symbols / Nets** and **Boards → Board → Footprints** under the
project, using the same tree in schematic and PCB workspaces. Click a sheet or
board to open it in the focused pane; its arrow unfolds it without navigating.
Unfold **Symbols**, **Nets** or **Footprints** to list a document's objects, and
click an object to open its document, select it and center the view on it. A net
row selects one of its wires. With one view open, the document path and count
sit directly on the canvas beside the Browser, without a tab strip.
Split views have one closable header above each canvas showing its document path, such as
**Power controller › Main**. Click the Browser's title to fold it. Browsing never changes the
project, and the canvas keeps its full size; **Fit** keeps the drawing clear of
the Browser. The ribbon's **View** tab shows or hides the Browser, Parts and Net
Inspector panels, and fits the view. Each ribbon group's caption, such as
**Place ⌄**, opens a menu of the same actions.

A project supports up to 64 sheets, either at the project root or nested under a parent. Right-click **Sheets** or a sheet row
to create a sheet, or use **Manage → Sheets → New sheet**; a sheet row also offers **Rename Sheet…** and **Delete Sheet…**.
Deleting a sheet removes its symbols, wires and associated board footprints in
one undo step, and the project always retains at least one sheet. Each pane keeps
its sheet target, camera and selection independently.

Use **S** or **Capture → Sheets → Subsheet** to place a hierarchical sheet symbol,
then choose a new child or an existing unparented sheet. Double-click inside the
symbol to enter its child; **Alt+Backspace** returns to the parent. The sheet name
sits above the outline; drag either to move it, or select it and drag a corner
handle to resize. Attached wires follow, and each gesture is one undo step. **H** or
**Capture → Sheets → Hierarchical label** places a hierarchical label inside a child. Its matching pin
appears on the parent symbol automatically, with the same name and direction;
**W** connects wires to either end. The Browser nests child sheets beneath their
parent. Each child has one parent, and repeated names on different sheets do not
connect them. See [hierarchical sheets](hierarchical-sheets.md) for editing
and deletion behavior.

## Views and windows

**View → Split** opens another view of the current schematic in that window.
Each pane keeps its camera, grid and edit selection while sharing the project's
commands, undo history and saving. Click a pane or its header to activate it;
the header underline marks that window's focused view. Browser, Parts and Net
Inspector stay in place and target that view. Selecting a sheet in Browser or
switching workspace changes that pane without adding document tabs.

On the desktop, **View → New window** opens another view without moving an
existing one. Multiple native windows and splits can coexist, and each detached
window can have its own split. **Detach** moves the active pane to a new window;
when moving the only primary pane, another view remains at the same zoom and
position. **Dock** in a detached window moves its focused pane beside the main
window's focused pane, preserving the other panes and windows. Closing a native
window while another remains dismisses only its views, without a save prompt or
an implicit split elsewhere. Each window uses the same title-bar style and has
its own panels.

Right-click a split pane's header for **Open beside**, **Open in new window**, **Detach
view**, **Dock in main window** or **Close view**, as applicable. These actions
target that pane, including its remembered document navigation. Dividers resize
panes, with narrow areas stacking vertically.
Opening another view copies the source camera and grid without copying its edit
selection. Turning Split off keeps the focused pane; turning it back on restores
the window's remembered split arrangement, seeding restored cameras from the
focused view. Layout and focus changes cancel unfinished gestures.

Selecting symbols or wires highlights their counterparts in every other visible
view in amber; blue remains each view's own edit selection. Highlights follow the
latest selection without taking focus. **View → Settings → Follow selection**
offers highlight only (the default), all views, other views following primary,
or primary following other views. Following reveals out-of-view items once while
preserving zoom. Primary is the original view even if detached; closing it
transfers that role to the main window's focused view. Delete acts only on the
focused view's own selection, and clearing the source selection clears its
highlights.

Closing any editor window leaves the others open. If the original window closes,
a surviving window takes over project dialogs and the Assistant without moving
its panes or resetting its camera. Closing the last editor window, or choosing
**File → Quit** from any window, uses one **Save / Don’t Save / Cancel** decision
for the project. Canceling or failing a save leaves the project open.

The top **Views** tab represents a named arrangement of the project's panes and
windows. Use **+** to add one, or right-click a tab to rename, duplicate or delete
it. Arrangements restore when a saved project is reopened on the same computer;
they live in user-local app settings and do not change or travel with project
files. Unsaved projects keep arrangements only for the session until their first
successful save. See the [arrangement persistence details](board-workspace-plan.md#named-arrangements-and-reopening-a-project).

## PCB layout

Browser builds support panes within one tab; native window actions are desktop
only. The workspace tile can switch to PCB layout, open it beside
the schematic or create a board window directly. Physical SMD placements render rounded rectangular copper pads, body outlines,
courtyards and silkscreen, with component counterpart highlights, grouped Layers
and read-only Properties. Legacy placements retain pad-center markers. The editor
supports up to 64 flat or hierarchical schematic sheets sharing one board. Existing footprints can be moved by left-dragging or by
hovering/selecting one and pressing **M**, then clicking to place it. **R** rotates
90° counterclockwise and **Shift+R** clockwise, including during a move. **Esc**
or a right-click cancels a move; releasing a drag over a panel also cancels it.
The Layout ribbon offers **Move** and **Rotate** for a single selection, and
Properties reports the saved orientation. Moves snap the footprint origin to the
view's grid, independently of grid visibility; each completed move and its
preview rotations form one undo step. A conflicting edit, focus/workspace change
or project reopen cancels the draft. The [starter footprints](footprint-library.md) include 0402/0603
resistors and capacitors, 0402 LEDs, SOT-23 and a two-pin plated header, pinned
into projects with explicit terminal mappings. New views hide fabrication and
courtyard graphics by default; Layers can show them. **Properties → Silkscreen**
edits physical text height, stroke width and pad clearance in one undoable board
change. Defaults are 1 mm, 0.15 mm and 0.25 mm respectively; lengths accept mil/mm
input and save with the project. **Assign Footprint…** in a schematic symbol's
context menu, or **Footprints** in the PCB Layout ribbon, opens package selection
and explicit terminal-to-pad mapping. Assigning pins the package without placing
it. In PCB layout, **Place on board** places an assigned component at unit-aware
X/Y coordinates, as a separate undo step; it then uses the ordinary M/R tools.
Changing an already placed package awaits reviewed board updates. Native-folder
registration, arcs, slots and routing remain planned.
For a board boundary, select **Board Edge** in the Layers card's Board group and
use **Layout → Draw → Line** or **Rectangle**. Line accepts successive clicked
vertices, closes when you click the first vertex, and finishes an open path with
Enter. Rectangle uses two opposite corners. Both snap to the grid and nearby edge
vertices, and Esc cancels unfinished geometry before exiting the tool. After
exiting, click an edge to select it, drag a vertex to reshape it, drag a segment
or press M to move the drawing, and Delete to remove it. These edits share project
Undo; Fit and layer visibility include the edge geometry. A Board Edge layer is
available independently of copper setup, including on older populated boards.
Open paths remain valid editing state; no closure, clearance or manufacturing
validation is implemented yet.
Live MCP reaches the transform command and these view, workspace and window
controls too (see the [MCP guide](mcp.md#live-mcp-in-the-desktop-editor)). See the
[board inspection walkthrough](quickstart.md#inspect-a-board-beside-its-schematic).

## Opening and saving projects

Open `project.pc`, its project folder, or a `.pc.zip` archive from **File → Open…**. Opening from PCB layout binds the incoming project's board before restoring
its saved views, including split panes and detached windows.
Desktop projects default to a folder containing separate project, schematic,
board and saved symbol documents. **Save** keeps the opened form, and
**File → Save As…** offers **Project folder…** and **Packed project (.pc.zip)…**.
Commit the folder to Git; use the archive to share the complete project.
Ordinary hand-zipped project folders open too, including those without a manifest.
The browser uploads archives and legacy files and downloads `.pc.zip` archives.
The [native storage notes](native-project-storage.md) describe the formats,
file safety and performance checks.
