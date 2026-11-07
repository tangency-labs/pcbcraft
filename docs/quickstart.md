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
cargo run --locked -p pcbcraft
```

GitHub access is required while the repository is private. The desktop app needs
a working graphical session; a headless shell can run the checks and MCP example
described below. See [Contributing](../CONTRIBUTING.md) for the toolchain and
browser build commands.

On Home, choose **Open Project**, then **Project file or archive…**, and select
`crates/app/tests/fixtures/split/two-resistors/project.pc` from your checkout.
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
`crates/app/tests/fixtures/split/two-resistors.pc.zip`. The browser needs the
archive, because uploading `project.pc` alone does not include its companion
documents. To begin an empty design on either platform, choose **New Project**
on Home. It opens an editable **Untitled** schematic at once. On the desktop, the
first **Save** asks for a project folder, and the project takes that folder's name.

## Make an edit and keep it

1. Drag a resistor on the canvas; its wire endpoint follows. Use **Undo**, then
   **Redo**, to check that the move is one edit. Select a resistor and press
   **E** to change its reference or value, or **R** to rotate it.
2. Press **A** to open Parts and focus search. Search for a capacitor, click the
   result and click the canvas to place it. **R** rotates a placement preview;
   **Esc** cancels it. Parts stays open until you close or hide it. Dragging a
   result directly onto the canvas also places it.
3. Click a pin or wire node to start directly, or press **W** to start on a wire
   segment or empty grid point. The snapped wire cursor rings an existing connection
   target. Click corners and a destination pin. **Space**
   flips the current elbow and **Backspace** removes the last corner. **Enter**
   or a double-click finishes a free end; **Esc** cancels the unfinished wire.
   Finish on an existing wire segment to create a connected branch. Dragging a
   free-end handle onto a pin or segment also connects it; simply crossing a
   wire still leaves the routes separate. Redrawing onto the same net keeps your new
   path when the old alternative has no intermediate pin or branch.
4. Left-drag empty canvas or right-drag to select several objects. Left-to-right
   selects fully enclosed bodies/routes; right-to-left selects touched ones.
   **Shift** adds and **Ctrl** toggles hits. Drag a selected member to move the
   group, or **Delete** to remove it. Each edit has one undo step.
5. Click a wire segment; only that segment turns blue, and **Delete** removes
   it. Use **Undo**, then drag the segment perpendicular to its direction to
   adjust alignment, or hover it and press **G**, then click to finish. Corner
   and free-end handles reshape the adjoining legs; attached pins stay fixed.
   **Esc** cancels a preview. Right-click → **Select Wire** selects the complete
   route for whole-wire movement or deletion. Capture → **Wire edits** offers
   **Slice**, **Break**, **Heal** and **Delete** for the selection; the context
   menu also offers whole-wire deletion. Moves and draws clean up overlapping
   runs, and **Heal** applies that cleanup to existing selected wires. Dropping
   a part with a pin exactly on a wire connects it; a two-pin part placed across
   a straight run replaces the wire between its pins. **Auto-heal wires** in the
   Wire edits menu defaults on. Retracing a draft shortens the preview as you draw;
   Backspace restores the preceding clicked corner. Switch automatic healing off to keep detours and loops while
   drawing, then select a wire and use **Heal** for cleanup when desired.
6. Save your desktop copy, reopen its `project.pc`, and check the edit. In the
   browser, choose **Download**, keep the resulting `.pc.zip`, then reopen that
   download to check it. A download request does not prove the file was saved,
   so the browser deliberately keeps its unsaved marker.
7. With unsaved changes, closing the last desktop editor window, **Quit**, **New Project**
   and **Open** ask **Save**, **Don't Save** or **Cancel**. **Save** carries on
   only once the save succeeds. In the browser, leaving or reloading the page
   shows the browser's own warning while the project is unsaved.

The status bar shows the grid point under the pointer; click its unit, or press
**Ctrl+U**, to read coordinates and the grid in mm instead of mil. The wheel zooms
and a middle-button drag pans by default. Preferences can change
wheel behavior, zoom direction/speed, theme and UI scale. Parts and Browser can
float or collapse; **View** restores a hidden panel. **Help → Keyboard Shortcuts**
shows the current bindings. Keyboard editing shortcuts apply to the canvas when
a text field or dialog is not using the keys.

The PCB grid is hidden by default. **Preferences → Appearance → PCB grid** offers
**Off**, **Dots** and **Lines**, remembered across sessions and shared by all PCB
views and windows. Grid opacity affects visible grids in both workspaces; display
changes leave snapping and each view's spacing unchanged.

To compare views, choose **View → Split**, pan one pane, and select a symbol:
the other pane highlights it while keeping its own selection. **View → Settings**
can also pan other views to reveal a selection. On desktop, **View → New window**
opens another view without moving the existing panes; repeat it for another
window, and use Split inside either window. **Detach** moves the active pane,
while **Dock** in a detached window adds its active pane to the main window.
Right-click a sheet tab to open beside it or close that pane. Closing a detached
window leaves the other windows' layouts and cameras intact. Edits and undo from
any window use the same project history.

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
The engine currently exposes one editable sheet and one legacy board.
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

## Inspect a board beside its schematic

Generate an original example into a directory that does not already exist:

```sh
cargo run --locked -p pcbcraft-engine --example board_workspace -- /tmp/pcbcraft-board-example
```

Open `/tmp/pcbcraft-board-example/board-inspection/project.pc` on desktop, or
upload the generated `board-inspection.pc.zip` in the browser. The example has a
resistor, a two-pin connector, two connections and two generic footprint
placements. It is an inspection fixture, not a fabrication-ready board.

Click the workspace tile and choose **PCB layout** to switch the focused pane.
To keep the schematic visible, click the split icon inside the PCB layout card
(tooltip: **Open PCB layout beside current view**). On desktop, the adjacent
window icon opens the board directly in another native window; repeat it to open
another. Each split pane has an **×** beside its document title that closes only
that view and leaves the other views in place. Select a symbol or a footprint marker and
check the counterpart's amber highlight. Each view keeps its own edit selection,
zoom and position, with optional panning under **View → Settings**.

In the board workspace, Browser picks select and center a footprint. Properties
shows its stored reference, value, definition and unit-aware coordinate readouts;
Layers groups copper in physical stack order and retains visibility by layer ID.
**Layout → Fit** fits the placements, and **Layout → Reveal** brings the current
counterpart highlight into view without changing zoom. Switching back restores
the schematic camera. These actions do not modify project data or undo history.

Only pad centers are represented by the current generic footprint model. Pad
shapes/drills, outline editing, routing, individual pad selection, selection
filters and Flip view still need implementation. Removing the last schematic
unit currently removes its footprint too; reviewed board updates are tracked
separately in #55.

Share the entire project folder or its `.pc.zip` archive, rather than the entry
point alone. Used symbol definitions are pinned so the recipient does not need
your source library. The folder is the Git-friendly working form; opening and
saving a current project without edits preserves its document bytes. The
[storage guide](native-project-storage.md) explains legacy migration and failed
or interrupted saves.

## Try the same project through MCP

Build the two desktop binaries from the repository root:

```sh
cargo build --locked -p pcbcraft -p pcbcraft-cli
```

To inspect a separate, headless copy of the example, configure an MCP client to
launch `target/debug/pcbcraft-cli` with arguments
`mcp --project crates/app/tests/fixtures/split/two-resistors/project.pc` and the
repository as its working directory. Use `.exe` on Windows. Running that command
in a terminal waits for MCP input; it is not an interactive command prompt.
Headless edits change only that process's in-memory session. They do not edit an
open GUI window or save back to the file.

For the new layer/stackup foundation, use `describe_command` with
`pcb.initialize_board_setup` and `pcb.set_board_setup`. On a project with no
footprints, initialize with `copper_layers: 2` (or `4`) and the current
`expected_revision`. Inspection returns `project.board.setup`; submit an edited
copy with the current revision to rename layers or specify `thickness_nm`.
Unspecified thickness stays unknown, and undo restores the previous setup.
The same command accepts project-local custom material definitions, sourced Tg
and Dk/Df reports, and core or ordered prepreg construction. Use the discovered
schema and [material contract](board-materials.md) for the exact fields and units;
material references pin both ID and revision. Updating a retained material
definition requires a new revision and explicit reference changes.
These are headless engine commands for now; there is no board setup dialog or
live-editor permission for them yet. The [representation plan](pcb-representation.md)
describes what follows this foundation.

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
retrying. The live adapter currently permits only `schematic.move_symbol` as an
edit; headless discovery exposes the larger command catalog. Full live command
coverage is tracked in [#61](https://github.com/tangency-labs/pcbcraft/issues/61).

If a step fails, report the checkout commit, platform, exact action and error
through [GitHub issues](https://github.com/tangency-labs/pcbcraft/issues), with a
small shareable project when relevant. The
[contributor guide](../CONTRIBUTING.md) describes where to start investigating.
