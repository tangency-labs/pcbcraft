# Try PCBCraft

This guide exercises the schematic editor available today. The
[capability table](capabilities.md) distinguishes working features from the
[MVP plan](https://github.com/tangency-labs/pcbcraft/milestone/1); PCB inspection and
component cross-highlighting are available, while board editing is still being built. The two-resistor
example below is an original development fixture, not the polished MVP demo or
a validated circuit design.

## Build and open an example

With Git and Rust 1.95 or later installed, clone a checkout you can edit:

```sh
git clone https://github.com/tangency-labs/pcbcraft.git
cd pcbcraft
cargo build --locked -p pcbcraft -p pcbcraft-cli
cargo run --locked -p pcbcraft
```

GitHub access is required while the repository is private. The desktop app needs
a working graphical session; a headless shell can run the checks and MCP example
described below. See [Contributing](../CONTRIBUTING.md) for the toolchain and
browser build commands.

The Assistant uses `pcbcraft-cli` beside the desktop app to reach the open
project. Build both binaries in the same profile; for a release build, add
`--release` to both Cargo commands.

If your existing project uses the previous `project.pc` folder or `.pc.zip`
package layout, open it normally. Its first Save upgrades it automatically;
opening it or showing its recent-project preview leaves its files unchanged.
To create a separate converted copy instead, use
`./target/debug/pcbcraft-cli convert /path/to/old-project /path/to/new-project`.
The destination must not exist; a `.pc` destination creates a portable package.
The [conversion guide](native-project-storage.md#convert-a-previous-split-project)
explains supported old inputs and failure behavior. Converted copies and new
projects store each board under its registered `boards/<folder>/board.json` path.
Supported flat `project.json` projects open unchanged and migrate on their first
Save, even without design edits; later unchanged saves produce no diff.
The current format also retains one stable occurrence ID per sheet, stored beside
the board in `instances.json` or in `project.json` for a schematic-only project.
First Save adds these [occurrence records](sheet-occurrence-storage.md) to older
board-folder projects while preserving their existing board paths.
Named nets and explicitly promoted nets use a [sparse persistent registry](net-registry-storage.md)
in those same instance homes. Its first record selects project/17, with
instances/2 for a board; opening or inspecting an older project does not promote nets.

On Home, choose **Open Project**, then **Project file or archive…**, and select
`crates/app/tests/fixtures/split/two-resistors/project.json` from your checkout.
The folder contains two wired resistors and their pinned symbol definition.
Use **File → Save As… → Project folder…** to save your own copy in a new or empty
folder outside the checkout before making changes. This keeps the tracked test
fixture intact. **View → Fit** brings the circuit into view. On desktop, use the
house button to return to Home: **Recent projects** lists successful opens and
saves as compact cards with saved schematic previews, including this copy. The
previews adjust their height to the card width as the window resizes. Clicking
a card reopens it. The list survives
restarts; **×** removes a card and **Clear recent projects** clears the history
without deleting files. Missing files report an error while keeping the current
project open.

For the browser, install Trunk as described in [Contributing](../CONTRIBUTING.md),
then run these commands from the repository root:

```sh
rustup target add wasm32-unknown-unknown
cd apps/pcbcraft-web
trunk serve --locked
```

Open [the local app](http://127.0.0.1:8781) and upload
`crates/app/tests/fixtures/split/two-resistors.pc`. The browser needs the
archive, because uploading `project.json` alone does not include its companion
documents. To begin an empty design on either platform, choose **New Schematic**
or **New PCB** on Home. Each opens an **Untitled** project in the chosen workspace,
with an empty sheet. **New Schematic** starts without a physical board; **New PCB**
creates its one board explicitly. Opening the PCB workspace from a schematic-only
project shows **Create board**; merely switching workspaces does not create it.
The compact cards wrap as the window narrows. Empty PCBs start at a closer drawing
scale, which **Fit board** restores until the board contains geometry; saved views
retain their own zoom and position.
On the desktop, the
first **Save** asks for a project folder, and the project takes that folder's name.

## Name nets and place power symbols

Choose **Capture → Ground** to start placing GND directly. Press **P** to open
the **Parts Catalog** at its Power library. Choose GND, VCC, **VCC (bar)** or
another drawing, edit its **Value** in the preview
if needed, then choose **Place** and click a pin, wire or grid point for its
anchor, or drag the chosen power entry directly onto the sheet. **L** opens a local label form; **Ctrl+L** opens a global label form,
where **Scope** selects either scope. **R** rotates a placement preview and
**Esc** cancels it. Local names join within a sheet, while global labels and
power rails join matching names across sheets. Power symbols use the same
properties and field tools as other components; their Value supplies the net
name. Hover a symbol and press **E** to edit, **G** to move with wires attached,
**M** to disconnect and move, or **Delete** to remove it. Held dragging also
retains wire contacts, including contacts made by placing onto an existing wire;
Undo reverses the complete edit. Local/global labels retain their label properties.
See [named nets](net-labels.md) for contact rules and current limits.

**VCC (bar)** uses the flat-bar drawing; **VCC** keeps the arrow. Both default to
the same `VCC` net name, which can be changed before placement.

## Make an edit and keep it

1. Drag a resistor on the canvas; its wire endpoint follows. Use **Undo**, then
   **Redo**, to check that the move is one edit. Select a resistor and press
   **E** to change its reference or value, **R** to rotate it, or **X**/**Y** to
   mirror it horizontally/vertically about its center. Hover and press
   **G** to grab a symbol with its wires attached, then click to place it.
   **M** picks up
   the selected symbol while leaving its wiring in place; with nothing selected,
   it picks the symbol under the pointer. Click to place it elsewhere, or **Esc**
   to cancel. Right-click → **Disconnect and Move** performs the same action. A replacement placed at the old pins reconnects
   to the preserved wire ends.
2. Press **A** to open the **Parts Catalog** and focus search. The bundled
   **Basic components** and **Power** libraries load through the same native source path
   as added libraries. Sources marks them **Built-in** and keeps their registrations
   read-only; external sources remain removable. Select a library,
   search for a capacitor and click the result to inspect its preview. Choose
   **Place**, then click the canvas to place it. **R** rotates the placement
   preview and **Esc** cancels it. **Repeat placement** keeps placing copies with
   the same rotation and is enabled by default for new settings. Toggle it in the
   catalog or the active tool's right-click menu, then press **Esc** to stop.
   Double-clicking a result or pressing **Enter**
   also starts placement; dragging a result onto the canvas places it directly.
   Overlapping drawings show an amber count badge with references on hover.
   Placement remains allowed; finish placement, then right-click → **Select symbol**
   to choose a hidden symbol before moving it.
   **Manage libraries…** in the catalog or **Libraries → Sources** adds native
   library folders on desktop or uploads a portable library bundle on either
   platform. Try the original [Workshop source example](native-library-sources.md),
   then select Workshop in the catalog. Placed symbols keep their pinned
   definition and license attribution after the source is removed. The
   [native part example](../examples/native-parts/README.md) also offers a generic
   resistor with a 0603 package and an optional synthetic manufacturer part.
   Choose **Unassigned** to defer the package, or select the manufacturer record
   to place the resolved combination. A placed native part's **Footprint** action
   opens its package, manufacturer and typed requirement form; changes use one
   undo step and flag a retained MPN when its specifications no longer meet the
   requirements. These records travel with folder and `.pc` saves.
   **Dock** switches the large form to a compact catalog beneath Browser; its
   header's **Expand** control returns to the large form with the same library,
   search and selection. The compact catalog stays open during placement and can
   be floated or docked at either edge from its **…** menu.
3. Click an unconnected pin or free wire end to start directly, or hover a pin,
   wire or grid point and press **W** to start there immediately. Connected nodes
   use W or **Capture → Wire**, which waits for a first click. The green wire cursor
   rings an existing connection target. Hold **Ctrl** to draw off-grid; releasing it
   restores snapping, while pins and wires remain connection targets. Click corners
   and a destination pin. **Space**
   flips the current elbow and **Backspace** removes the last corner. **Enter**
   or a double-click finishes a free end. The wire tool stays ready for a new route;
   **Esc** exits it or cancels an unfinished wire.
   Finish on an existing wire segment to create a connected branch. Dragging a
   free-end handle onto a pin or segment also connects it; simply crossing a
   wire still leaves the routes separate. Redrawing onto the same net keeps your new
   path when the old alternative has no intermediate pin or branch.
   For an intentionally unused pin or open wire end, hover its lead or endpoint
   and press **Q**, or use **Capture → No connect** and click several targets.
   The X snaps to each attachment. Press **Esc**, select a placed X, then press
   **Delete** to remove only the marker. Dragging a wire-end X stretches its wire;
   **Esc** cancels the drag. Right-click **Delete** also removes the marker. **Analyze → Electrical menu → No-connect check** inspects
   this intent and any saved wiring conflicts. Remove the X before adding a wire;
   marked endpoints block new connections, including with **W**.
4. Left-drag empty canvas or right-drag to select several objects. Left-to-right
   selects fully enclosed bodies/routes; right-to-left selects touched ones.
   **Shift** adds and **Ctrl** toggles hits. Drag a selected member to move the
   group, or **Delete** to remove it. Hover a symbol and press **G** to pick it up
   with attached wires, or **M** to leave its old wiring in place. Press **R** while
   moving to rotate the preview, then click to finish; **Esc** cancels both changes.
   Each edit has one undo step.
   **Ctrl+C**, **Ctrl+X** and **Ctrl+V** copy/cut/paste through the system
   clipboard, including between split panes, windows and running instances.
   Paste previews at the pointer; **R** rotates, a click places, and **Esc** cancels.
   **Ctrl+Shift+V** opens [Paste Options](clipboard.md) for references, named-net
   isolation or original-position placement. Use **⌘** instead of Ctrl on macOS.
   Copying a sheet body includes its child subtree. For a same-project Cut,
   Paste Options requires **Reuse original unparented sheets** to be enabled
   explicitly; leaving it unchecked creates independent copies.
5. Click a wire segment; only that segment turns blue, and **Delete** removes
   it. Use **Undo**, then drag the segment perpendicular to its direction to
   adjust alignment, or hover it and press **G**, then click to finish. Corner
   and free-end handles reshape the adjoining legs; attached pins stay fixed.
   **Esc** cancels a preview. Right-click → **Select Wire** selects the complete
   route for whole-wire movement or deletion. **M** with one wire or segment selected
   picks up the whole route; click to place, or press **Esc** or right-click to cancel.
   **M** also moves a selection of several symbols, wires or labels together,
   preserving their relative positions and attached no-connect markers.
   M disconnects contacts to unselected objects and leaves their wiring in place;
   **G** or dragging the group keeps those connections and stretches attached wires.
   Capture → **Modify** menu offers
   **Slice**, **Heal** and **Delete** for the selection. The context menu also offers
   **Break** to release the selected leg's attachments, including pins and shared junctions,
   whether one or both ends are connected. An already-free end stays free. The detached
   straight piece moves freely in both directions; its original contacts remain disconnected
   until pulled clear, after which dropping back onto them reconnects normally. Moves and draws clean up overlapping
   runs, and **Heal** applies that cleanup to existing selected wires. Dropping
   a part with a pin exactly on a wire connects it; a two-pin part placed across
   a straight run replaces the wire between its pins. A filled dot marks a branch
   connection; unrelated crossing wires have no dot. **Auto-heal wires** in
   **Preferences → Schematic** (also in the Modify menu) defaults on. Retracing a draft shortens the preview as you draw;
   Backspace restores the preceding clicked corner. Switch automatic healing off to keep detours and loops while
   drawing, then select a wire and use **Heal** for cleanup when desired.
6. Save your desktop copy, reopen its `project.json`, and check the edit. In the
   browser, choose **Download**, keep the resulting `.pc`, then reopen that
   download to check it. A download request does not prove the file was saved,
   so the browser deliberately keeps its unsaved marker.
7. With unsaved changes, closing the last desktop editor window, **Quit**, **New Project**
   and **Open** ask **Save**, **Don't Save** or **Cancel**. **Save** carries on
   only once the save succeeds. In the browser, leaving or reloading the page
   shows the browser's own warning while the project is unsaved.

The status bar shows the grid point under the pointer; click its unit, or press
**Ctrl+U**, to read coordinates and the grid in mm instead of mil. The wheel zooms
smoothly around the pointer and a middle-button drag pans by default. Pinch
gestures follow your fingers directly, and grid density fades between scales.
Preferences can change wheel behavior, zoom direction/speed, theme and UI scale.
Browser and Net Inspector
use collapsible cards over the canvas; **View** restores a hidden card and opens
the Parts Catalog in its last presentation. **Help → Keyboard Shortcuts**
shows the current bindings. Keyboard editing shortcuts apply to the canvas when
a text field or dialog is not using the keys.

PCB routing is under **Layout → Route**, and **Draw → Line / Rectangle** uses
the active layer. Copper shapes are conductive linework; technical-layer graphics
retain a physical stroke. The layer, Track / line width and via dimensions stay
visible across PCB tabs, with alignment actions in **Layout → Modify**.

The PCB grid is hidden by default. **Preferences → Appearance → PCB grid** offers
**Off**, **Dots** and **Lines**, remembered across sessions and shared by all PCB
views and windows. Grid opacity affects visible grids in both workspaces; display
changes leave snapping and each view's spacing unchanged. Schematic and PCB views
remember their snap spacing separately, so changing one editor's grid does not
replace the other's startup choice.

To compare views, choose **View → Split**, pan one pane, and select a symbol:
the other pane highlights it while keeping its own selection. **View → Settings**
can also pan other views to reveal a selection. On desktop, **View → New window**
opens another view without moving the existing panes; repeat it for another
window, and use Split inside either window. **Detach** moves the active pane,
while **Dock** in a detached window adds its active pane to the main window.
Right-click a sheet tab to open beside it or close that pane. Closing a detached
window leaves the other windows' layouts and cameras intact. Edits and undo from
any window use the same project history.

On desktop, drag a split pane's document header over another pane in the same
window to move it into that position. Two panes exchange places; with more panes,
the intervening panes shift while the existing dividers and cameras stay intact.
The destination highlights **Move pane here** before release. Moving outside the
client highlights **Open in new window**; release there or over that row to detach.
The native window is created on release, without an outside-client preview.
To return, click **Dock** at the left
of the detached pane's header. During a drag, **Dock in main window** and other
window targets appear beside the header where you grabbed it, so a window
extending beyond the monitor does not hide them at its far edge. The selected
row and destination split region highlight before release. Where window positions
are available, dropping directly over an exposed pane in another window also
docks beside it. **Esc** cancels a drag; right-clicking the header also offers
**Dock in main window**. All targets belong to the same running instance;
separately launched copies keep their own project sessions.

On Linux, drag the logo, project title or empty space in the top bar to move a
window; double-click those areas to maximize or restore it. Drag an edge or
corner to resize. These controls also work while a project dialog is open,
without enabling the editing controls behind it.

The project name stays separate from the top **Views** arrangement tab. Its tooltip
explains the arrangement and its right-click actions. Use the
**+** beside it to name a setup, starting with the current views and windows or a
single view. Right-click any arrangement tab for **Rename…**, **Duplicate…** or
**Delete…**. Renaming or deleting an inactive tab leaves the current setup in
place. Naming opens a dialog in the window you are using;
**Cancel** or **Esc** discards the draft. Each arrangement remembers
its panes, document navigation, cameras, panels and desktop windows locally; reopening
the saved project restores the last active arrangement. Selecting a sheet in
Browser or a workspace changes what the focused pane shows, without adding tabs,
switching arrangements or moving views elsewhere. A single view has no lower tab
strip; split panes each show one document header whose **×** closes the pane.
The engine supports up to 64 sheets and zero or one physical board. Right-click
**Sheets** in the Browser and choose **New Sheet…**, or use
**Sheets → New sheet**; sheet rows also offer
**Rename Sheet…** and **Delete Sheet…**. Creating a sheet opens it in that pane.
The breadcrumb uses the actual sheet name and its position in the sheet count.
Deletion asks for confirmation and removes its symbols, wires and associated
footprints together; Undo restores them. Other views of a deleted sheet become
unavailable until it is restored or another document is opened.

To add a subsheet, press **S** or choose **Sheets → Subsheet**, click its parent canvas and give the child a name,
or choose an existing unparented sheet in **Child sheet**. Double-click inside
the placed rectangle to enter it; **Alt+Backspace** returns to its parent.
The name appears above the outline. Drag the body/name to move the sheet, or
select it and drag a corner handle to resize; **Esc** cancels an active gesture.
Place a hierarchical label with **H** or **Sheets → Hierarchical label**, enter its name and direction, then
connect its pointed anchor with **W**. A matching sheet pin appears automatically
on the parent symbol. Names and directions stay synchronized when you edit the
label with **E** or its context menu. The Browser shows the same nesting in
schematic and PCB workspaces. See [hierarchical sheets](hierarchical-sheets.md).
See [arrangements](board-workspace-plan.md#named-arrangements-and-reopening-a-project)
for storage and monitor-restoration limits.

Close the original window while another editor remains: the surviving window
keeps its views and takes over project dialogs. Closing the last editor window
prompts for unsaved changes; **Cancel** keeps that window open. **File → Quit**
from any editor asks once before closing the whole project. Browser builds keep
panes inside one tab. Schematic is currently the editable workspace; PCB layout
inspects existing placements and highlights their schematic counterparts.

Desktop Close releases the addressed native window, including the original
window. A surviving editor keeps its native identity and position even when both
windows show the same document. There is no hidden original editor or automatic
switch away from Wayland. Native close/reopen verification currently covers Linux
Wayland and X11; Windows and macOS still need platform qualification.

## Inspect and edit component properties

Typed part values accept engineering prefixes from atto through exa, such as
`1 fF` or `1 PHz`, within the [documented precision limits](part-schema.md).

Select a symbol to use the compact **Properties** card. Edit reference, value,
rotation or its **X/Y** position and choose **Apply changes**, or open **More properties** for the
expanded form. **E** opens the same form. **OK** applies the supported properties
as one undo step, with attached wires following a position change. Coordinates
accept explicit units such as `100 mil` or `2.54 mm`; a bare number uses the
display unit. **Reset changes** discards the compact draft, and **Cancel** discards the
expanded draft without changing the project. The Pin functions and
Library tabs show the exact definition retained by the project. The **Footprint**
control opens the package chooser; apply pending component edits before changing
packages. **View → Properties** controls this window's inspector.

## Inspect a board beside its schematic

Generate an original example into a directory that does not already exist:

```sh
cargo run --locked -p pcbcraft-engine --example board_workspace -- /tmp/pcbcraft-board-example
```

Open `/tmp/pcbcraft-board-example/board-inspection/project.json` on desktop, or
upload the generated `board-inspection.pc` in the browser. The example has a
resistor, a two-pin connector, two connections and two generic footprint
placements. It is an inspection fixture, not a fabrication-ready board.

Click the workspace tile and choose **PCB layout** to switch the focused pane.
Drag from empty canvas to enclose footprints in a selection window, or drag
right-to-left to select footprints the box crosses; Shift adds and Ctrl/Command toggles.
To keep the schematic visible, click the split icon inside the PCB layout card
(tooltip: **Open PCB layout beside current view**). On desktop, the adjacent
window icon opens the board directly in another native window; repeat it to open
another. Each split pane has an **×** beside its document title that closes only
that view and leaves the other views in place. Select a symbol or a footprint marker and
check the counterpart's amber highlight. Each view keeps its own edit selection,
zoom and position, with optional panning under **View → Settings**.

Both workspaces use the same **Browser**, with **Sheets** and **Boards** under the
project. Ordinary clicks stay in the current workspace: document rows from the
other workspace expand or collapse for inspection. To switch from a Browser row,
right-click and choose **Open in Schematic** or **Open in PCB layout**. Object
rows select and center within their workspace; their **Show in…** context action
explicitly opens another workspace. The workspace picker remains available.
In the board workspace, Properties shows its stored reference, value, definition
and unit-aware coordinate readouts.

Use **Layout → Board → Board setup**, or **Board setup…** in Layers, to edit the board.
Choose any even copper count from 2 through 64; 10- and 12-layer boards are included.
**Board editor layers** edits names and unused optional layer membership.
**Physical stackup** edits copper/dielectric thicknesses, tolerance and core/prepreg
construction, with a laminate total and a cross-section aligned to the table rows. **Fit unlocked dielectrics**
fits a target thickness while retaining copper and locked gaps. Values accept mm
or mil, and blanks remain unspecified.

**Materials** offers generic family choices and custom materials. **Add material**
starts with basic identity fields; **Technical properties** expands sourced Tg/Dk/Df
characterization. For an existing material, **Technical details → Add specification…**
stages a new immutable revision. Save it to the draft, then select that exact revision
for a dielectric. **Apply board setup**
commits one undoable edit, while Cancel discards the draft. Existing placements and
package assignments are retained; populated-board copper changes explicitly update
through-hole spans while keeping front copper identity. Occupied-layer removal and
count reductions that would lose authored stratum data are rejected. Routing,
impedance calculations and manufacturing rules remain future work.

Left-click a footprint to select it, then press **Delete** to remove its board
placement. Multiple selected footprints are removed together. The **Layout →
Modify menu → Delete** action does the same thing. Schematic components and physical
package assignments stay available; **Place on board** can place the assigned
package again. Undo restores the original placement IDs and geometry.

The Layout ribbon's **Active layer** selector shows the drawing target, also named
in the status bar. In Layers, click a full row to activate it; the arrow and
highlight follow that choice. The eyes at the right toggle visibility
independently, and
a hidden active layer is labeled **(hidden)** in the selector and status bar.
**Show all** reveals every layer; **Isolate active** reveals only the active layer.
Layers groups copper in physical stack order and retains visibility by layer ID.
Click a layer swatch to change its color; the form supports a picker, **#RRGGBB**,
Apply, Cancel and Reset to default. The swatch and supported geometry use the
same color, including pad metal, Board Outline and technical graphics.
The selected ribbon layer and its menu rows show matching color patches.
**+** / **−** cycle through picker order (including hidden/technical layers);
**PgUp** / **PgDn** select front/back copper. These keys change the active layer
without changing visibility. Plated lands take the active visible copper color, while front-only surface
pads stay on front copper. Colors are shared local settings for this board,
separate from its geometry, saved engineering files and document undo.
**Fit** in the ribbon header fits the placements, and **Analyze → Cross-probe → Reveal schematic** brings the current
counterpart highlight into view without changing zoom. Switching back restores
the schematic camera. These actions do not modify project data or undo history.

The **Selection filters** card below Properties limits which supported object
types can be selected. **All items** toggles them together and **Reset** enables
them all. Filters are independent for each view, are retained in local saved
arrangements, and leave the project unchanged; unavailable object types are disabled.
Drag the title of Selection filters, Layers or Properties to float or dock the
card, or use its **…** menu for **Float**, **Dock left** and **Dock right**.

To edit an existing footprint, hover it and press **M**, move the pointer, then
click to place it, or drag the footprint with the left mouse button. **R** rotates
90° counterclockwise and **Shift+R** clockwise, including during a move; **Esc**
or a right-click cancels the preview. Move/Rotate are also in the Layout ribbon
for a single selected footprint. The origin snaps to the view's grid even when
that grid is hidden. Undo once to reverse a completed move and all rotations
made during that move; both the schematic and PCB views share that history.
Save and reopen the folder/archive to check the stored position and orientation.
A concurrent edit or a change of focused view cancels unfinished input.

The [Dual LED demo](../examples/README.md) contains physical SMD packages with
rounded rectangular copper pads, body outlines, courtyards and silkscreen. Older generic
placements retain pad-center markers; opening them does not assign a package.
Board-wide text height, stroke width and pad clearance are in **Board setup →
Silkscreen**, where **Apply board setup** applies them together with any staged layer
or stackup edits. A new setup starts from a two-layer 1.6 mm FR-4 draft with
1 oz copper. **Physical stackup → Starting stackup** offers other two-layer presets,
and **Materials** offers generic material families or custom definitions. **Profiles**
imports or exports a portable `.pcsetup.json`; imported values stay in the draft until
**Apply board setup**. General copper-clearance and routing rule editors remain planned.

You can assign footprints while the project is still schematic-only. To choose
a package for an unplaced component, right-click its schematic symbol
and choose **Assign Footprint…**. For a starter resistor, choose **Resistor 0402**,
map terminal 1 to pad 1 and terminal 2 to pad 2, then choose **Assign** and
**Close**. This pins the package without creating a PCB or placement; folder and
archive saves retain it offline. Creating your first board later adopts these
choices for Update PCB. In PCB layout,
open **Layout → Place → Footprint**, select the component, enter X/Y and
choose **Place on board**. Assignment and placement are separate undo steps;
save/reopen retains the package and mapping offline. The chooser also supports
assigning an unassigned component directly from PCB layout. On a board with
recorded accepted state, changing a placed package records a proposal for the
next reviewed update. See the
[footprint contract](footprint-library.md#assigning-and-placing-a-package) for
mapping constraints and source limits.

Click a placed reference designator to select its text and open **Properties**.
The owning footprint highlights in lighter layer colors, and a dashed leader
connects the lettering to its footprint. Remaining airwires also appear. Drag the
lettering, or press **M** and click, to move it; **R** rotates the text and
**Delete** hides it. Properties offers local printed words, the bundled fonts,
size/stroke, rotation, alignment, visibility and offsets. Clear **Use component
reference** to print other words without changing the component's identity.
**Rename…** explicitly changes the shared component reference, including the
schematic. **Reset silkscreen** restores inherited text, appearance and position;
select the footprint body and **Edit silkscreen text** to recover hidden text.
To set a library default position, open the footprint in **Footprint Editor**,
select **Definition**, and edit the reference offsets in Properties.

To forward a captured schematic, choose **Capture → PCB → Update PCB**; from PCB
layout use **Layout → Board → Update PCB**. **Create board** explicitly adds
the first PCB when needed. Existing footprints without recorded accepted state
first require **Use current state**, which adopts today's schematic, packages and
connections as the starting point rather than reconstructing earlier history.
Resolve missing footprint choices in **Changes to Be Applied**, review the report, then
choose **Update PCB**. The PCB opens with new footprints together on the cursor;
click to place the group, **R**/**Shift+R** to rotate or **Esc**/right-click to cancel.
Cancellation retains accepted assignments; reopen Update PCB and choose
**Place footprints** to resume. Package assignment returns directly to the review.
Compatible pad mappings are preselected, including named terminals. The starter
LED defaults to terminal 1 (Anode) → pad 2 and terminal 2 (Cathode) → pad 1 from
revision-qualified library data. Saved package mappings take precedence, and
you can change any default before assigning. The report includes full mapping and connection changes;
Show filters control visibility while totals always count every warning/error.
Turn off **Place new footprints after updating** to keep the updated report open
and place later. **Copy report** includes all message categories.
**Close** leaves an unapplied review unchanged, while board creation/current-state
adoption remain separate edits. External changes or another client's preview
require **Review again**; a reopened/replaced project requires a fresh dialog.

In **Layers → Board**, select **Board Outline**, then choose **Draw → Line**
or **Rectangle**. Click successive vertices for a line path; click its first vertex
to close it, or press **Enter** to finish an open path. Rectangle uses two opposite
corner clicks. Points snap to the current grid and nearby existing edge vertices.
**Esc** discards the unfinished path, then exits the tool on a second press.

With the tool exited, click an edge to select it. Drag a vertex to reshape the
path, drag a segment or press **M** to move the whole drawing, and use **Delete**
to remove it. Each completed edit shares project Undo. Fit includes edge geometry
even on an empty board; the layer's visibility eye only affects this view.
Saving retains drawing and layer identities without moving footprints or creating
a copper stackup. Open paths are editing geometry, and this slice does not check
closure across drawings, edge clearance or manufacturing validity.

To route a track, activate a copper layer and press **X**. The crosshair follows
the pointer, and the target marker shows nearby grid points and copper contacts.
The preview keeps a lighter shade of the layer color while background preparation
is pending and shows the default clearance around it. Starting on a trace brightens its copper and dims the rest
of the board while you route. Hold **Ctrl** to bypass the grid. Click a pad,
track end or via to finish on copper, or press **Enter** after placing the last
corner in free space. Right-click opens the route menu, which includes **Cancel**.
Zoom in with the wheel to inspect copper, then use **Fit board** to frame the board again.

Use **Analyze → Rules** to save board limits and preferred routing dimensions.
Clear **Use built-in defaults** to author values, then Apply; checking it restores
the built-in values. New routes use those preferences, while existing copper
keeps its dimensions. See [board rules](board-rules.md) for overrides and sources.

Run **Analyze → DRC** to check the board. The check shows its current stage while
pan and zoom remain available. **Cancel check**, **Esc** or leaving the view stops
unfinished work; **Run again** replaces it with a check of the current revision.
Edits and project reopening invalidate unfinished reports.

Pad clearance is measured against the
resolved circular, oval, rounded-rectangle or rectangular copper contour, including
rotation and board side. A gap exactly equal to the selected clearance passes;
clearance enforcement while routing remains separate. DRC also reports existing
copper islands that join different intended nets, including direct pad and through-via
contact at zero clearance. Once board updates are initialized, the check uses accepted
connectivity until you review and accept a later schematic change.

After reviewed updates are initialized, removing the last
schematic unit retains its footprint until an update accepts the removal. A
legacy board retains its prior deletion behavior until explicit current-state
adoption, so establish that starting point before editing its schematic.

Share the entire project folder or its `.pc` archive, rather than the entry
point alone. Used symbol definitions are pinned so the recipient does not need
your source library. The folder is the Git-friendly working form; opening and
saving a current project without edits preserves its document bytes. The
[storage guide](native-project-storage.md) explains the compatibility break and failed
or interrupted saves.

Selected tracks and vias brighten in their copper colors, like footprint pads,
and have a **Lock** choice in Properties. **Locked** keeps
them selectable while protecting geometry and sizes from edits; unlock them
before moving, reshaping or deleting them. Copy/duplicate retain locks, and Cut
refuses protected copper. See the [lock policy](pcb-layout-workflow.md#fixed-tracks-and-vias)
for mixed selections and indirect footprint/segment changes.

## Try the same project through MCP

For repeatable UI demonstrations in the native app, the live connection can
[locate controls and send ordinary input](ui-automation.md). Start with
`ui.handles` for the intended window, send a named `ui.click`, and wait for its
completion ticket through `ui.inspect` before continuing.

Build the two desktop binaries from the repository root:

```sh
cargo build --locked -p pcbcraft -p pcbcraft-cli
```

To inspect a separate, headless copy of the example, configure an MCP client to
launch `target/debug/pcbcraft-cli` with arguments
`mcp --project crates/app/tests/fixtures/split/two-resistors/project.json` and the
repository as its working directory. Use `.exe` on Windows. Running that command
in a terminal waits for MCP input; it is not an interactive command prompt.
Headless edits change only that process's in-memory session. They do not edit an
open GUI window or save back to the file.

For the new layer/stackup foundation, use `describe_command` with
`pcb.initialize_board_setup` and `pcb.set_board_setup`. On a project with no
footprints, initialize with any even `copper_layers` from 2 through 64 and the current
`expected_revision`. With existing footprints, also pass
`resolve_existing_footprints: true` to explicitly adopt front/all-copper pad
resolution. Inspection returns `project.board.setup`; submit an edited
copy with the current revision to rename layers or specify `thickness_nm`.
Unspecified thickness stays unknown, and undo restores the previous setup.
The same command accepts project-local custom material definitions, sourced Tg
and Dk/Df reports, and core or ordered prepreg construction. Use the discovered
schema and [material contract](board-materials.md) for the exact fields and units;
material references pin both ID and revision. Updating a retained material
definition requires a new revision and explicit reference changes.
The Board setup dialog uses these same commands; live and headless MCP share them.
The [representation plan](pcb-representation.md) describes the remaining work.

For the open desktop window instead, choose **Help → Connect AI assistants… →
Enable local connection** and copy its configuration into your MCP client. If
the CLI is not on PATH, replace the configuration's command with the absolute
path to the binary you just built. Close the setup dialog before making an edit.
The copied credential is valid for that enabled connection; keep it out of bug
reports and shared projects. **Disable connection** revokes it.

In either session, call `inspect_project` to obtain the current revision and a
symbol ID, and `describe_command` with `{"command":"schematic.move_symbol"}`
to see the accepted parameters. Pass those inspected values to `execute`:

```json
{
  "command": "schematic.move_symbol",
  "params": {
    "symbol": "<symbol ID from inspection>",
    "x_nm": 0,
    "y_nm": 5080000,
    "expected_revision": 1
  }
}
```

Replace the symbol ID placeholder and example revision `1` with the values from
inspection. Keep the revision as a JSON number, without quotes.
In the live editor the resistor and attached wire move, the project becomes
unsaved, and GUI **Undo**/**Redo** use the same history. Inspect after each step
to verify the result. If another edit changed the revision, inspect again before
retrying. The live editor runs the same project commands as a headless session.

For multiple-sheet projects, take the desired sheet UUID from `inspect_project`
and include it as `sheet` in schematic edits and footprint placement. Use
`sheet.create`, `sheet.rename` and `sheet.delete` for lifecycle changes, with the
current `expected_revision`. Each inspected sheet includes its own schematic;
omitting `sheet` is accepted only when one sheet exists. `view.open_sheet` takes
`view` and `sheet` to navigate a live editor pane without editing the project.

The live editor also offers window commands, including
`window.set_catalog_mode` with a window ID, `mode` of `expanded` or `compact`,
and an optional boolean `repeat`.
It uses the same Dock/Expand handler as the catalog and leaves the project unchanged.
Call `inspect_editor` for its window/view IDs, then split the window and switch
to the light theme:

```json
{"command": "view.split", "params": {"view": 0, "split": true}}
```

```json
{"command": "app.set_preferences", "params": {"theme": "light"}}
```

The window splits and changes palette, but the project's revision, unsaved state
and undo history stay as they were. A headless session refuses these commands
and says they need a live editor.

If a step fails, report the checkout commit, platform, exact action and error
through [GitHub issues](https://github.com/tangency-labs/pcbcraft/issues), with a
small shareable project when relevant. The
[contributor guide](../CONTRIBUTING.md) describes where to start investigating.

## Try a symbol or footprint definition

Choose **Symbol Editor** or **Footprint Editor** from **Workspace** beside the
ribbon. Each has its own Draw,
Library and View ribbon tabs, canvas, Browser and Properties. Switch back to
**Schematic** or **PCB layout** through the same picker; the definition's history
and pending fields remain available when you return. Select a primitive
on the canvas or in Browser, edit its Properties and choose **Apply**.
**More properties…** expands the same draft; lengths accept mm, mil or inches.
All four editors share wheel/pinch zoom up to 102400%; **View → Fit**
frames the definition in the canvas clear of the panels.
The picker’s split and window icons open a definition beside the current pane
or in its own native window. Within either editor, **View → Split** creates a
second pane, and **Detach / Dock** moves the focused pane between windows.
Closing a pane retains its definition and unfinished Properties fields; the
last window still asks about unsaved work.

For a footprint, change its pad dimensions and try Undo/Redo in the app bar.
To make a plated hole, set Layers to All copper and enter a positive drill smaller
than the land. Corner radius is zero for rectangular/circular pads.

On desktop, **Parts Library → New → Library** creates a writable native folder.
Create a symbol or footprint and publish it to that collection first, or choose an
existing populated writable collection. Select a published item and choose **Edit**, then change its
geometry in the matching editor. **Library → Publish…** (Ctrl/Cmd+S) adds an
immutable revision of the same item. New drafts inherit the selected writable collection and bind to it after their
first successful publication. Publish shows the destination and offers **New
library…** in its selector. Choosing another collection switches to **Publish
copy**; **Save as → New item** makes a derivative in the same collection.
The original stays unchanged, and the editor follows the new copy after saving.
**Edit copy** also opens an independent draft under a new identity;
publishing an older base requires confirmation and does not merge newer changes.
Publishing makes the item discoverable without updating placed project copies.

**Save Library Copy** keeps unfinished work outside the catalog and chooses a new `.pcitem.json` filename; an
existing file is never overwritten. Reopen it through the editor’s **Library → Open**, or
through **File → Open Library Item…** inside a definition editor.
The browser downloads the same editable format and retains its leave warning.
**File → Export Native Definition…** produces the existing standalone definition
schema, while the editor file retains authoring IDs and provenance. To start from
a placed item, return to the project, select one symbol or footprint and use
**Libraries → Edit copy**. To create another definition, use **Library → New**
inside its editor. Forking and authoring leave the source and placed copies unchanged. Applying the resulting
definition to a project is follow-up work; see the
[editor notes](properties-and-library-editors.md#authoring-basics) for scope.

## Inspect accepted board connections

With typed quantities captured by a board update, deleting a part from the
schematic keeps its SI-formatted value in PCB Properties and Browser while
its footprint awaits reviewed removal. Older accepted snapshots retain their
stored value text until a real board update captures typed history.

When a board has an accepted update baseline, the PCB canvas shows thin airwires
between its mapped physical pads. **Layers → Ratsnest** hides or shows them for
that view, and the status bar reports airwires and missing endpoints. Schematic
changes affect those connections after a reviewed board update accepts them;
footprint moves and rotations update the displayed geometry immediately. These
lines show logical connections and do not establish routed copper or fabrication
readiness; see [accepted board connections](board-ratsnest.md).

### Copying PCB groups

Select a group with a directional marquee, or use Ctrl/Command+A for visible
supported objects. Layout → Clipboard and the PCB context menu provide system
Copy, Cut, Paste and Duplicate; keyboard shortcuts match the schematic. Copied
footprints duplicate their schematic counterparts too, so PCB Paste Options
shows the destination sheet and schematic anchor for each source circuit.
Same-project Cut/Paste can reuse the original unplaced components. Review the
[clipboard guide](clipboard.md) for layer mapping, named nets and relocation.

Ctrl+Shift+X (macOS: Command+Shift+X) starts the standalone via tool. On native
non-Latin layouts, it uses physical X when the logical character has no key
mapping; Windows Shift+Delete remains Cut.

With a configured board and a closed Board Outline, **Manufacture → Fabrication → Outputs**
saves a fabrication ZIP without editing the design. Output names remain unique
when layer names collide after sanitization; the [editor guide](editor-guide.md)
describes the generated files and desktop save behavior.

### Browse libraries

Choose **Parts Library** in the workspace picker, or **Libraries → Sources**. Select a
library and Type in Browser, search its items, and select a row for Preview.
**Add library** opens a folder or bundle; desktop bundles install into a new
folder. Remove leaves files intact, while Delete moves an eligible folder to
system trash after confirmation. In the browser, added libraries need reopening
after a reload. Projects keep their pinned copies. See the
[native source guide](native-library-sources.md) for limits and source diagnostics.

## Create a reusable part

Open **Parts Library**, choose **New → Generic part**, and select an existing
symbol. Add a package, choose its footprint and assign each pad to a terminal or
to Unconnected. Publish to a writable native library, then find the resolved
record in the Parts Catalog to place it. A published generic part with a package
can also receive an orderable manufacturer/MPN record through **Add orderable
part**. These forms live in the Library workspace; drawing changes still use the
Symbol and Footprint Editors. Unpublished part documents last for the current
session, so publish finished work before closing. The [source guide](native-library-sources.md#generic-and-orderable-part-documents)
covers exact revisions, typed values and recovery.

Generic library parts do not require a symbol. In Parts Library, use **Add
footprint…** to author a footprint-only part, or add the symbol independently with
Browse. When both are present, complete the pin mapping before publishing.
Schematic placement requires a symbol; the current placement command does not
create board-owned symbol-less instances.
