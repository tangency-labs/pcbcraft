# Try PCBCraft

This guide exercises the schematic editor available today. The
[capability table](capabilities.md) distinguishes working features from the
[MVP plan](https://github.com/tangency-labs/pcbcraft/milestone/1); PCB layout and
visual schematic/board cross-probing are still being built. The two-resistor
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
fixture intact. **View → Fit** brings the circuit into view.

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
3. Press **W**, click a pin, then click corners and a destination pin. **Space**
   flips the current elbow and **Backspace** removes the last corner. **Enter**
   or a double-click finishes a free end; **Esc** cancels the unfinished wire.
   A crossing does not create a connection, and attaching to the middle of an
   existing wire is not automatic yet. Right-click a wire and choose **Slice**
   to create a connected endpoint there, then start another wire from it.
4. Left-drag empty canvas or right-drag to select several objects. Left-to-right
   selects fully enclosed bodies/routes; right-to-left selects touched ones.
   **Shift** adds and **Ctrl** toggles hits. Drag a selected member to move the
   group, or **Delete** to remove it. Each edit has one undo step.
5. Click a wire segment; only that segment turns blue, and **Delete** removes
   it. Use **Undo**, then drag the segment perpendicular to its direction to
   adjust alignment, or hover it and press **G**, then click to finish. Corner
   and free-end handles reshape the adjoining legs; attached pins stay fixed.
   **Esc** cancels a preview. Right-click → **Select Wire** selects the complete
   route for whole-wire movement or deletion. **Slice**, **Break**,
   **Delete Segment** and **Delete Wire** remain available in that menu.
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

To compare views, choose **View → Split**, pan one pane, and select a symbol:
the other pane highlights it while keeping its own selection. **View → Settings**
can also pan other views to reveal a selection. On desktop, **View → New window**
opens another view without moving the existing panes; repeat it for another
window, and use Split inside either window. **Detach** moves the active pane,
while **Dock** in a detached window adds its active pane to the main window.
Right-click a sheet tab to open beside it or close that pane. Closing a detached
window leaves the other windows' layouts and cameras intact. Edits and undo from
any window use the same project history.

Close the original window while another editor remains: the surviving window
keeps its views and takes over project dialogs. Closing the last editor window
prompts for unsaved changes; **Cancel** keeps that window open. **File → Quit**
from any editor asks once before closing the whole project. Browser builds keep panes inside one tab. Schematic is currently
the only editable workspace, so these actions show additional views of its one
sheet; schematic/PCB workspace choices come with the board editor.

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
