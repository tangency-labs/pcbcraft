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

## Net labels and power symbols

**Capture → Ground** starts a GND placement preview directly. **P** opens the
**Parts Catalog** at its Power collection, with GND, VCC, VCC (bar), +3V3, +5V and
other starter drawings. The two VCC choices have distinct drawings and default
to the same electrical net name. Select one to preview it and
edit its net name, then choose **Place**. **Capture → Net label** or **L** opens
a local label form; **Ctrl+L** opens a global label form. The label form has
aligned fields for net name, scope and rotation. Click the anchor position to
place the marker; **R** rotates the preview and **Esc** cancels it. Hover the
marker's name or glyph for **E**, **M**, **R** and **Delete**, or use its context
menu. Drag the drawing or name to move a marker directly; **M** picks it up for
click-to-place movement. **E** opens properties, where **Apply** confirms changes.
Moving a marker keeps its touching wires connected regardless of placement order;
fixed component pins stay in place while the wire stretches to the moved anchor.

Names are case-sensitive. Local labels join matching names on the same sheet;
global labels and power/ground symbols join project-wide. A marker anchored
exactly on a pin or wire contacts it. The Browser, Net Inspector and PCB pad
annotations show the resolved name. [Named nets](net-labels.md) explains scope,
aliases, shared commands and limits.

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

Choose **Capture → Symbol** or press **A** to open the **Parts Catalog** and
focus search. The large form has library navigation, searchable results and a
selected-part preview. **PCBcraft starter** contains the resistor, LED,
capacitor, two-pin connector, four-pin header and NPN transistor from
[`libraries/starter`](../libraries/starter); registered sources appear as
additional libraries. **Power** contains the built-in power and ground drawings.
**Manage libraries…** opens the existing source manager. See the
[native source guide](native-library-sources.md) for loading a library and
[symbol library format](symbol-library.md) for native definitions.

Click a result to inspect its drawing, value and source, then choose **Place**.
Double-clicking a result or pressing **Enter** also starts placement. In the large
form, the catalog closes and leaves a preview on the canvas; click to place it,
**R** to rotate or **Esc** to cancel. A result can also be dragged directly onto
the sheet, including Power entries with their chosen net name. **Cancel**, **Esc** or the header's **×** closes the large form without
changing the project. On narrower hosts, a Library selector replaces its
navigation pane; the smallest forms have Parts and Preview tabs.

**Repeat placement**, enabled by default for new settings, keeps the chosen part or power symbol
active after each placement, preserving its rotation until **Esc**. With it off,
one placement finishes the tool. Each instance has its own undo step, and the
choice is retained when switching catalog presentations or restoring an arrangement.
The active placement's right-click menu also offers **Repeat placement**, **Rotate**
and **Cancel**. Placement commits on the primary press, so motion before releasing
cannot turn a quick placement into a discarded drag.

Right-click empty canvas in the default pointer mode for **Draw wires**, **Place symbol…**,
**Place ground**, **Select all**, **Unselect all**, **Fit schematic** and a **Grid**
submenu with the same spacing presets as the status bar. Opening this
menu preserves the current selection; symbol and wire hits retain their object menus.

**Dock** in the large form switches to a compact catalog, initially docked
beneath Browser. Its header's **Expand** control returns to the large form.
Both presentations keep the same library, query, selection, result density and
custom power name, and reopening uses the last presentation. The compact catalog
stays open after Place or a drag onto the sheet. **A** focuses its search; **Esc**
releases search focus before another Escape cancels a canvas tool.

Drag the compact header to float the catalog or release it near an edge to dock.
Its **…** menu also offers **Float**, **Dock left** and **Dock right**. The chevron
collapses it to its header, and **×** closes it. Returning from the large form
preserves the compact catalog's position. Docking overlays the canvas without
resizing it; **Fit** keeps the drawing clear of the visible cards. Compact mode
and its placement/visibility are retained in saved arrangements; an open large
form and unfinished input gestures are not restored.

New resistors, capacitors, LEDs and two-pin connectors use shorter leads from
revision 2 of the starter symbols; saved revision 1 parts retain their original
geometry.

## Moving symbols

Hover a symbol and press **G**, or choose **Grab (keep connected)** from its context
menu, to move it without holding a mouse button. Attached wires follow the symbol;
press **R** to rotate the preview and click to finish, or use **Esc** or right-click
to cancel. Ordinary symbol dragging keeps the same connected movement and supports **R**.

Press **M** with a symbol selected, or hover a symbol with no selection, to pick
it up while leaving its wiring in place. Click to place it elsewhere, or press
**Esc** or right-click to cancel the preview. **Disconnect and Move** in the
symbol's context menu performs the same action. **R** also rotates this preview;
movement and rotation commit together as one undo step, while cancellation changes neither.
The move and disconnection share one undo step; placing a replacement at the vacated
pins connects it to the preserved wire ends.

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
right-click → **Cancel** discards the draft and exits drawing. Completing a wire clears
its corners and keeps the tool ready for a new start; **Esc** returns to selection.
Click a completed wire segment in selection mode to select it;
only that segment turns blue, and **Delete** removes it. Drag a segment perpendicular to its
direction to adjust alignment, or hover it and press **G**, then click to finish. Attached pins
and junctions stay fixed while the adjoining legs adjust. **Esc** cancels the preview.
Free endpoints, shared junctions and interior corners can be dragged without selecting
the wire first. Hover a handle and press **G**, or right-click the wire and choose
**Grab**, to pick it up without holding a button, then click to place it. Moving a
shared junction adjusts all its attached routes; terminal pins stay on their symbols.
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
**Preferences → Schematic → Auto-heal wires** defaults on and is remembered as a
user preference; the Modify menu offers the same toggle. Turn it off to keep detours and loops during editing, then use **Heal**
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

The **Selection filters** card sits below Properties in schematic and PCB views.
Its checkboxes control symbols (including power/ground), wires, local/global and hierarchical labels, and sheets in the
schematic, and footprints and board graphics in PCB layout. **All items** toggles
the supported categories together; **Reset** enables them all. Categories without
individual selection support are disabled. Filtering does not hide geometry or
change connectivity. Turning a category off clears its edit selection and cancels
unfinished gestures; explicit placement and drawing tools remain available.
Each view retains its filters through workspace changes, splitting and detach/dock,
and saved arrangements keep them in local application settings. The card's
chevron collapses it to its header; **View → Selection filters** hides or shows the
card while retaining its settings.

Selection filters, Layers and Properties have draggable titles and a **…** position
menu with **Float**, **Dock left** and **Dock right**. Drag a title away from its
edge to float the card, or release near either edge to dock it; **Esc** restores
the position from before the drag. Cards docked on the same edge stack beneath
one another, with Browser above left-docked cards. Positions belong to the view,
survive split and detached layouts, and are kept in local project arrangements.
The Symbol and Footprint editors use the same docking controls for Properties.

The schematic **Properties** card shows the focused selection's reference,
value, orientation, X/Y position and footprint. Coordinates accept mm, mil or
inch suffixes; bare numbers use the current display unit. Changing display units
reformats valid drafts without applying or rounding away their stored coordinates.
**Apply changes** commits all fields in one undo
step. **More properties**, **E** over the body or away from text with one symbol selected,
or a double-click on the body opens the expanded form. **General** combines
reference, value, field visibility, datasheet, description, manufacturer,
manufacturer part number and named custom fields;
these belong to the selected component without changing its library definition.
HTTP(S) values have an open-link control beside the input, including custom fields.
**Add link** creates an additional named field for documentation, supplier pages or
other web URLs. Each link opens independently, while **D** opens the saved
datasheet for the hovered component (including its text),
or the single selected component when the pointer is elsewhere. Opening a link
changes neither the project nor undo history; text entry, dialogs and active tools
retain their keyboard input. Local document paths remain plain field values.

Double-click visible reference, value or custom-field text, or hover it and press **E**,
to edit it in place on the canvas. **Enter** applies the string as one undoable edit;
**Escape**, an outside click, focus loss or a changed view/project cancels it. Invalid
input stays in the text box for correction without changing the document. The editor
stays in its originating split pane or native window and holds live mutation commands
until accepted or cancelled.

The **Text appearance** disclosure selects a field and controls its name display,
size, alignment, rotation and local offsets. Drag the reference, value or another visible
field on the canvas to move only its text. Its outline appears on hover; the
preview commits on release, while Escape, focus loss or release outside the
canvas cancels. Moving the body carries its text, and rotating the body rotates
manual offsets while retaining the chosen text orientation. Hover visible field text and
press **R** to rotate that field about its anchor. **R** also rotates a field while
dragging; release commits its position and angle together, and Escape discards both.
**Reset appearance**
restores automatic placement and default appearance for that field.

The attributes beside placement record population and output/simulation exclusions.
Board
exclusion prevents new placement. With reviewed board updates enabled, excluding
an existing placement stages its removal for Apply; otherwise remove its footprint before excluding
its component. BOM and simulation flags are retained intent for their future consumers.
**Exclude from pick-and-place files** records omission from assembly position exports
(reference, coordinates, orientation and board side); PCBcraft retains the flag,
but does not yet generate those files.

**Reset changes** discards the compact draft, and **Cancel** discards the
expanded draft. Attached wires follow coordinate changes through the same contact
and healing rules as symbol movement. Invalid input or ambiguous contacts reject
the entire draft. **View → Properties**
shows or hides the inspector for that window. Footprint choice opens a responsive
package browser focused on that component. From PCB layout, a searchable,
scrolling component list lets you work through the project. The browser includes
a library list, token-based name/package search, pin-count
and mounting filters, and a larger physical 2D preview. Drag the preview to pan,
use the wheel or +/− to zoom, and **Fit** to restore its framing. On narrow windows
the library, results and preview stack in the scrolling body while **Assign** and
**Close** remain visible. Browsing does not change the component; choose each
terminal-to-pad connection or use **Match numbers** when their identifiers agree,
then **Assign**. Source/license details remain available beneath the preview.
The **Library** tab shows the pinned source/revision and opens **Manage libraries**
or **Edit symbol copy**. Copy editing opens an independent definition document;
it does not update the placed symbol. Updating a placed symbol from a library
revision or replacing it with another symbol is not implemented. Apply or cancel
pending field changes before using these actions.

Power and ground markers also appear in the selection Properties card. Edit their
net name, choose Ground, Power arrow or Power bar, and change position/rotation
there or through **More properties**, **E**, double-click or **Properties…** in the
context menu. Their text appearance controls size, visibility and a separate
local offset while preserving the electrical anchor. Drag a visible power/ground
name to move only its text; **M** picks up that name and **R** rotates it when hovered
or while dragging. Double-click its text or press **E** over it to edit the net name
in place; Enter applies and Escape cancels. Full appearance controls remain in Properties.
Dragging the glyph still moves the whole marker and its attached wires. Changing the net name changes
the connected global net; changing only its text appearance leaves the wiring in place.

Press **R** over a symbol to rotate it, or select it first. With a symbol selected,
**E** opens its expanded properties and **Delete** removes it with its wires.
Right-click a symbol for the same actions, and press **Esc** to clear the selection.
New generic resistor and capacitor placements show neutral `R` and `C` values.
Set their actual ratings in Properties; existing projects and pinned library
revisions keep their original values.

Reference and value belong to the component; position, rotation and field appearance belong to
its placed unit. Text sizes scale with the drawing when zooming. Native formats 2 and 3 record these separately, and opening a format 1
project migrates it on the next save. Authored wire routes use format 3. Footprints link to the component.

With no placement or wiring tool active, **left-drag empty canvas** or **right-drag**
to select a group. Left-to-right draws a solid **Window** box and selects fully
enclosed symbol bodies, wire paths and named-net anchors (including power and ground); right-to-left draws a dashed **Crossing**
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
project, using the same tree in schematic and PCB workspaces. Ordinary clicks
never change workspaces: document rows navigate within the current workspace,
or fold a branch belonging to the other workspace. Disclosure arrows only fold.
Right-click a document for **Open in Schematic** or **Open in PCB layout**.
Unfold **Symbols**, **Nets** or **Footprints** to inspect objects. An object click
selects and centers within its workspace; **Show in…** in its context menu opens
it explicitly in another workspace. A net row selects one of its wires. With one view open, the document path and count
sit directly on the canvas beside the Browser, without a tab strip.
Split views have one closable header above each canvas showing its document path, such as
**Power controller › Main**. Click the Browser's title to fold it. Browsing never changes the
project, and the canvas keeps its full size; **Fit** keeps the drawing clear of
the Browser. The ribbon's **View** tab shows or hides the Browser and Net Inspector,
opens the Parts Catalog, and fits the view. The Net Inspector uses the same
collapsible card and tree rows as the Browser, with expandable named nets, their
individual pins and a separate unconnected-pin section. Search filters nets or
pins; inspecting them does not change the project. Each ribbon group's caption, such as
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
the header underline marks that window's focused view. Browser and Net Inspector
stay in place and target that view. The large Parts Catalog captures its opening
view/revision; compact placement captures the current view when the gesture starts.
A stale drag or modal choice cannot edit a different view or newer project state. Selecting a sheet in Browser or
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

On desktop, dragging a header shades its source pane and displays window
destinations. Drop over another pane in the same window to move into that slot;
intervening panes shift, so a two-pane layout exchanges its positions. The full
slot highlights **Move pane here**, retaining divider ratios and cached documents.
Release outside the window or over **Open in new window** to detach, retaining
its remembered documents and camera.
Detached panes retain a draggable header with a leading **Dock** button that
returns the pane to the main window in one click. Drag destinations stay beside
the initial header press, rather than at a possibly off-screen far edge. Drag
onto **Dock in main window** or another window target; its row and destination
split region highlight before release. Direct drops over exposed panes in another
window are also available where the compositor exposes window positions. Moving outside the
client highlights **Open in new window**; the new OS window is created on release.
There is no window preview outside the client. **Esc** cancels. Every target belongs
to the same running instance and project session. Live MCP can address the same destination with `view.dock`, passing the
source `view` and destination `beside` view IDs from `inspect_editor`.
`view.reorder` takes source `view` and destination `target` IDs for in-window movement.
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

The **Layout** tab groups the board tools as **Place** (Footprint, Via), **Outline**
(Line, Rectangle), **Route** (Track, followed by the Active layer, Track width and Via
pad / drill fields), **Modify** (Move, Rotate; Delete in its menu), **Inspect** (DRC,
Reveal) and **Design** (Sync updates the PCB from the schematic; Setup opens Board
setup). Tools that are not built yet, such as Zone, Pair, Tune, Align, Rules, Measure,
Flip view, Outputs and Refill zones, appear disabled in their group's menu and say
"planned". The ribbon scrolls sideways when the window is too narrow for every group.

Left-drag empty canvas or right-drag to select footprints with the same Window/Crossing
box used in the schematic. Only visible footprint geometry contributes to selection;
Shift adds and Ctrl/Command toggles. Escape cancels the box, and Delete removes
the selected footprints in one undo step. Footprint moves and rotation operate on one placement at a time.

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
courtyard graphics by default; Layers can show them. **Board setup → Silkscreen**
stages physical text height, stroke width and pad clearance alongside the layer
and stackup settings. Apply commits them in one undoable board change; Cancel
discards the draft. Defaults are 1 mm, 0.15 mm and 0.25 mm respectively; lengths accept mil/mm
input and save with the project. **Assign Footprint…** in a schematic symbol's
context menu, or **Layout → Place → Footprint** in PCB layout, opens package selection
and explicit terminal-to-pad mapping. Assigning pins the package without placing
it. In PCB layout, **Place on board** places an assigned component at unit-aware
X/Y coordinates, as a separate undo step; it then uses the ordinary M/R tools.
Changing an already placed package awaits reviewed board updates. Native-folder
registration, arcs and slots remain planned.

**Layout → Design → Setup** and **Layers → Board setup…** share a revision-bound
Craft settings form, including on a board with no saved stackup. The first opening
shows a complete editable two-layer 1.6 mm FR-4/1 oz draft; nothing changes until Apply.
Existing setups retain their own dimensions and materials. Board editor layers provides names and optional membership;
Physical stackup provides every even copper count from 2 through 64, finished
thicknesses/tolerances, a laminate total and cross-section, locked-gap fitting,
and core/prepreg construction. Materials offers generic family presets and custom definitions, with sourced Tg/Dk/Df
specifications and explicit pinned revisions behind technical details. The laminate
preview uses copper and dielectric appearances independently of canvas layer colors.
Profiles imports/exports a self-contained `.pcsetup.json` snapshot of stackup,
materials and silkscreen; imported settings remain staged until Apply. Empty values stay unknown and typed units override
the display unit. Apply is one undoable transaction; Cancel and invalid/stale drafts
leave the board unchanged. Existing material pins cannot be rewritten; create a
new revision and explicitly choose it in construction. Populated copper changes
retain physical surface identities and resolve existing front/all-copper bindings;
occupied drawings and authored stratum data are protected against removal.

Left-click footprints to select them, then press **Delete** to remove only their
board placements. **Layout → Modify menu → Delete** provides the same action. Components, schematic wiring and physical
package assignments are retained for re-placement, and undo restores the original
placements. Active drawing tools and movement previews keep keyboard ownership.

The Layout ribbon's **Active layer** selector and status bar identify the drawing
layer. Click a row in Layers to activate it, with an arrow and color swatch
beside its name. The eye at the right toggles visibility independently: a pupil
means visible, a slash means hidden, and its outline is always shown. Hidden
active layers carry a **(hidden)** suffix. **Show all**
and **Isolate active** change only visibility in the focused view, leaving saved
board data and document undo unchanged. Click a layer color swatch to open the
themed color form. Choose a color or enter **#RRGGBB**,
then **Apply color**; **Cancel** discards the draft, and **Reset to default** removes
that layer's override. Colors are local settings shared by every view of this
board, including copper, Board Outline, silkscreen, mask and other technical layers. They survive
renames without changing layer ownership or entering project files and undo.
The ribbon's selected layer and every dropdown row include the same effective
color patch. **+** and **−** cycle through the picker order, wrapping through
technical and hidden layers without changing visibility. **PgUp** selects front
copper and **PgDn** selects back copper when that side exists. Text fields,
menus and dialogs keep their normal keys; switching layers cancels an unfinished
Board Outline gesture or route. The keys are also listed in Keyboard Shortcuts.

For a board boundary, select **Board Outline** in the selector or Layers card and
use **Layout → Outline → Line** or **Rectangle**. Line accepts successive clicked
vertices, closes when you click the first vertex, and finishes an open path with
Enter. Rectangle uses two opposite corners. Both snap to the grid and nearby edge
vertices, and Esc cancels unfinished geometry before exiting the tool. After
exiting, click an edge to select it, drag a vertex to reshape it, drag a segment
or press M to move the drawing, and Delete to remove it. These edits share project
Undo; Fit and layer visibility include the edge geometry. A Board Outline layer is
available independently of copper setup, including on older populated boards.
Open paths remain valid editing state while you draw; **Layout → Inspect → DRC**
reports a missing closed outline and copper outside it or too close to its edge.

To route copper, select a copper layer such as **Top** and press **X** or use
**Layout → Route → Track**. Set **Track width** beside it in the display units
(0.25 mm by default). Click a pad or a grid point to start; each click adds a run with one
45° bend, and **/** flips whether the diagonal comes first. Clicking a pad or a
track end on the same layer finishes the route, as do a double-click and Enter.
Press **V** while routing to drop a through via at the pointer and continue on the
other side: front copper leads to back, any other layer to front, and the view's
active layer follows. The **Via pad / drill** field beside the width field sets the
pad diameter and drill (0.6 mm and 0.3 mm by default). Backspace removes the last via, returning to the
previous layer, or else the last vertex. Esc or right-click drops the unfinished
route, and a second Esc exits the tool. The whole route is one Undo step. When copper
joins two terminals of a net, their airwire disappears. A route that would connect
pads on two different nets is refused with a message, leaving the board unchanged.
**Layout → Inspect → DRC** checks the board without editing it and opens a DRC card
listing each finding. It reports copper of different nets closer than the clearance
(0.2 mm), tracks narrower than 0.15 mm, via annular rings under 0.1 mm, each unrouted
connection, a missing closed Board Outline, copper outside the outline or within 0.3 mm
of its edge, and overlapping footprint courtyards. Copper joined by tracks or on the
same net needs no clearance. Click a finding to center the view on its marker; the
status bar shows the result, and an edit marks it out of date until **Run again**.
These rule values are fixed defaults until net classes and board rules arrive (#77);
zones, hole spacing, silkscreen and solder mask are not checked, so a clean result is
not a manufacturing sign-off.

To stitch layers or start a route from a via, choose **Layout → Place → Via** or press
**Ctrl+Shift+V**. Each click places one through via with the current pad and drill,
snapping to a nearby pad center or track end on any copper layer, otherwise to the grid;
the tool stays armed until Esc or right-click, and each via is its own undo step. A via
that would join pads on two different nets is refused, as is a second via on the same
spot. The tool needs a board setup with at least two copper layers.

Without a tool, click a track or via to select it (Shift adds, Ctrl or ⌘ toggles) and
press Delete to remove it; deleting a via keeps its tracks. A selection box also picks
tracks and vias, with the same Window/Crossing rules as footprints. **U** selects every
track and via joined by copper to the one under the pointer, or to the current copper
selection. With copper selected and no footprint selected, **M** (or **Layout → Modify → Move**)
moves the selection
with the pointer until the next click, and dragging a track or via moves it directly;
both snap to the grid, Esc or right-click cancels, and each completed move is one undo
step. Properties shows the selected tracks and vias, with editable **Width**, **Via
pad** and **Via drill** fields in the display units; a value applies to every selected
item when you release a drag or press Enter. A move or resize whose copper would reach
another net's pad is refused, leaving the board unchanged. The **Tracks** selection
filter turns track picking off. Tracks stay on their layer, so a back-side track does
not reach front-only SMD pads, while a via joins copper on every layer. Reshaping a
route by dragging one segment and blind or buried vias are not implemented yet.
Routing does not check clearance; run **Inspect → DRC** for that. PgUp and PgDn still change layer by cancelling the route.
Live MCP reaches the transform command and these view, workspace and window
controls too (see the [MCP guide](mcp.md#live-mcp-in-the-desktop-editor)). See the
[board inspection walkthrough](quickstart.md#inspect-a-board-beside-its-schematic).

The same picker includes **Symbol Editor** and **Footprint Editor** as independent
authoring workspaces with Draw, Library and View ribbons. Switching retains each
definition's history, camera, selection and pending Properties fields, and returns
to the last document of the selected kind. Their picker rows and View ribbons
support the same split, detach/dock and native-window placements as project views.
A definition can share a split with a schematic or PCB view; multiple views of
one definition share its history while retaining independent navigation and
selection. Closing a pane retains the definition and any unapplied fields.
Definition placements currently last for the session, with the last saved project
arrangement retained while definition panes are open.

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
