# PCBCraft

Electronics design that feels like the tools you already know, built so a person and an agent can work on the same board, at the same time, with the same commands.

Start with the [quickstart](docs/quickstart.md) for a small editable example,
the [capability table](docs/capabilities.md) for current limits, or
[CONTRIBUTING.md](CONTRIBUTING.md) to build and work on an issue.

**Status: just started.** Schematic editing works on desktop and in the browser.
The app starts on **Home**, where you can create or open a project. It does not
yet reopen the last project automatically. The app bar shows the project name and an
unsaved marker beside **Undo**, **Redo**, and **Save** (**Download** in the
browser). The gear (or **Ctrl+,**) opens **Preferences**, with
pages for Appearance (theme, interface scale, grid opacity, units), Navigation (whether the wheel
zooms or pans, zoom direction, speed and anchor), Views (selection following),
a list of Shortcuts and, on the desktop, the AI page for the Assistant.
**Grid opacity** fades the canvas grid
from 100% down to hidden at 0%, with **Reset** to go back; parts still snap to the
same points. The opacity applies to both split panes and is kept with your
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
open with its history while Home is shown. See [VISION.md](VISION.md) for the
project's direction.

Choose **Capture → Symbol** or press **A** to open the **Parts** browser and focus
its search. It lists schematic symbols from
[`libraries/starter`](libraries/starter): a resistor, LED, capacitor, two-pin
connector and 4-pin header. See [docs/symbol-library.md](docs/symbol-library.md) for the file
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

Choose **Capture → Wire** or press **W**, then click a pin, an existing wire endpoint
or empty grid space to start. Click to add corners, **Space** to flip the current
elbow, and **Backspace** to remove the last corner. Click a destination pin or endpoint
to finish, or press **Enter** or double-click to finish on empty space. **Esc** or
right-click → **Cancel** discards the draft. Click a completed wire segment to select it;
only that segment turns blue, and **Delete** removes it. Drag a segment perpendicular to its
direction to adjust alignment, or hover it and press **G**, then click to finish. Attached pins
and junctions stay fixed while the adjoining legs adjust. **Esc** cancels the preview.
Right-click → **Select Wire** selects the whole route for dragging or deletion; box and
modifier selections continue to select whole wires. Crossings stay disconnected.
Right-click a wire for these edits:
**Slice** splits it at the grid point under the pointer and keeps it connected, so a new wire
can start from the slice point. **Break** separates the leg under the pointer from the legs beside it. **Delete Segment** removes just that leg, and **Delete Wire** the whole
route. Hovering a row previews what it changes. An unconnected wire end shows a small square, a
point where three or more wires meet shows a dot, and two wires meeting end to end read as one
line. Agents use `schematic.slice_wire`, `schematic.break_wire` and
`schematic.delete_wire_segment` with a point on the wire.
`schematic.move_wire_segment` takes `wire`, `at`, `to` and `expected_revision`;
`to` moves the point at `at` perpendicular to the segment. It uses the same
geometry as the canvas preview and commits one undo step. These commands are available in
headless MCP; the live desktop connection still exposes only symbol movement.

Selecting a segment also shows square handles on its wire’s corners and free ends. Drag a handle
to reshape it in place: a straight segment slides across its direction, a corner drags the
two legs that meet there, and a free end or junction drags the end leg of every wire attached
to it. The wire's other ends stay put, gaining an orthogonal leg where needed; pins never leave
their parts, and connections don't change. The drag moves by the grid's snapped travel, so an
off-grid wire stays off the grid by the same amount. Release to commit one undoable edit;
**Esc**, releasing outside the sheet, or an edit made meanwhile cancels it. Multi-item
selections and explicit **Select Wire** selections still move whole routes. Agents use `schematic.move_wire_segment`,
`schematic.move_wire_corner` and `schematic.move_junction`. Overlapping or doubled-back runs
are not yet merged automatically.

Press **R** over a symbol to rotate it, or select it first. With a symbol selected,
**E** edits its reference and value and **Delete** removes it with its wires.
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

The **Browser** floats over the left of the canvas and shows the project as
**Project → Sheets → Main → Symbols / Nets**. With one view open, the sheet path
and count sit directly on the canvas beside the Browser, without a tab strip.
Split views have a tab above each canvas showing its sheet path, such as
**Power controller › Main**. Arrows fold rows; unfold **Symbols** or **Nets** to list the sheet's
items, and click one to select it and center the view on it. A net row selects
one of its wires. Click the Browser's title to fold it. Browsing never changes the
project, and the canvas keeps its full size; **Fit** keeps the drawing clear of
the Browser. The ribbon's **View** tab shows or hides the Browser, Parts and Net
Inspector panels, and fits the view. Each ribbon group's caption, such as
**Place ⌄**, opens a menu of the same actions. A project has one sheet so far.

**View → Split** opens two views of that schematic. Click a pane or its sheet tab
to activate it; the tab underline marks the active view. Browser, Parts and Net
Inspector keep their workspace positions while their actions target that view.
Both tabs show the same sheet path when they display the same sheet. Each view
keeps its own camera and selection, including box and modifier selections, while edits, undo,
redo and saving use the same project. Changing panes cancels unfinished gestures.
Selecting symbols or wires highlights their counterparts in the other pane in
amber; blue remains that pane's own edit selection. Highlights follow the latest
selection, including Browser picks and box selections, without changing focus.
**View → Settings → Follow selection** controls automatic panning: highlight only
(the default), both views, secondary follows primary, or primary follows secondary.
Following reveals out-of-view items once per selection while keeping zoom; it
does not repeatedly override manual panning. Primary is the left/top pane and
secondary the right/bottom pane, regardless of focus. Switching tabs alone keeps
the highlight; Delete acts only on the focused pane’s own selection. Clearing the
source selection clears its highlight.
Drag the divider to resize the panes; narrow windows stack them vertically.
Opening Split copies the active view’s current zoom and position into the other
pane. Turning Split off keeps the active view; reopening copies its camera again
while retaining each view’s separate edit selection.
The arrangement lasts for the app session and does not change project files.
Detached editor windows and schematic-to-board cross-probing remain planned.

Open `project.pc`, its project folder, or a `.pc.zip` archive from **File → Open…**.
Desktop projects default to a folder containing separate project, schematic,
board and saved symbol documents. **Save** keeps the opened form, and
**File → Save As…** offers **Project folder…** and **Packed project (.pc.zip)…**.
Commit the folder to Git; use the archive to share the complete project.
Ordinary hand-zipped project folders open too, including those without a manifest.
The browser uploads archives and legacy files and downloads `.pc.zip` archives.
The [native storage notes](docs/native-project-storage.md) describe the formats,
file safety and performance checks.

## Run it

Use Rust 1.95 or later and start in the repository root. To launch the desktop app:

```sh
cargo run --locked -p pcbcraft
```

For the browser, install [Trunk](https://trunkrs.dev) and run:

```sh
rustup target add wasm32-unknown-unknown
cd apps/pcbcraft-web
trunk serve --locked
```

Then open [the local web app](http://127.0.0.1:8781).

The [Dual LED editing demo](examples/README.md) includes a small wired schematic,
a shareable archive and a walkthrough for editing and inspecting pin/pad mappings.

## Assistant in the desktop editor

Open **Assistant** from the chat icon next to Preferences or press
**Ctrl+Shift+A** (**Cmd+Shift+A** on Mac). Drag the title to switch sides,
drag the inside edge to resize, or use **Pop out Assistant** to move the chat
into its own window. **Dock Assistant** returns it to the workspace.
Minimize folds the panel into the bottom-right strip. Close hides the panel
and keeps the conversation available when you reopen it.

Open **Preferences → AI** from Home or the gear in the app bar to test
sign-in, choose the default agent, and set where and when Assistant opens.
**Add agent…** saves a named Codex
or Claude Code profile. Choose **Installed agent** to use the standard
executable on PATH or **Custom command** to use a compatible executable at
another location. Both **Add agent…** and **AI settings…** are available
from the Assistant's agent dropdown.

For an installed agent, put the Codex CLI or Claude Code on your PATH. For a
custom command, set the executable path in its profile. Choose the agent and
select **Connect**.
The header shows **Ready** when PCBcraft can use the agent's saved login.
If the agent asks you to sign in, choose **Sign in** and complete the browser
flow. You can also sign in with `codex login` or `claude auth login --claudeai`
in a terminal, then choose **I've signed in** or **Retry** in PCBcraft.

Open a schematic and ask a question or request a wire or a symbol move.
**Send** shares the schematic and recent conversation with the selected
agent. Review the specific edit, then choose **Apply** or **Discard**.
**Undo** reverts an applied edit. **Stop** cancels a request in progress.
If the schematic changed since the request, ask again before applying.

PCBcraft remembers the agent choice and reuses the CLI sign-in across
projects. Switching projects or agents starts a fresh conversation.

## Live MCP in the desktop editor

Build the desktop and CLI binaries with `cargo build --locked -p pcbcraft -p pcbcraft-cli`.
Open a project in the desktop app and choose **Help (?) → Connect AI assistants…
→ Enable local connection**. In a narrow window, use the same row on **Home**. Copy the configuration into your MCP client. If `pcbcraft-cli`
isn't on your PATH, set its command to the full path of the binary in
`target/debug`. Close the setup dialog before editing. The copied configuration
grants access to this window until you disable the connection or close the app.

For a quick check, open `crates/app/tests/fixtures/split/two-resistors/project.pc`. Use
`inspect_project` to get R1's symbol ID and the current revision. Call `execute`
with `schematic.move_symbol` and pass the ID as `symbol`, coordinates as integer
`x_nm` and `y_nm`, and the inspected revision as `expected_revision`. Move R1 to
`(0, 5_080_000)` and watch the attached wire follow it. Undo and redo in the
editor, then inspect again to confirm the position. The edit stays unsaved
until you save the project.

## Headless MCP

To start the MCP server from the repository root, run
`cargo run --locked -p pcbcraft-cli -- mcp`. To connect an MCP client, point it
at `pcbcraft-cli mcp`. Pass
`--project path/to/project.pc` (or a folder or `.pc.zip`) after `mcp` to start with a saved snapshot. The
headless session is separate from the app window. Send `inspect: false` with
`execute` to return just the resulting revision. Use `inspect_project` to read
the project, or `cross_probe` to look up a linked terminal or pad.

An MCP client can use `schematic.connect_pins` to join two terminals from
`inspect_project`. Describe the command for its input shape, and send the current
project revision with the two endpoints. Project inspection returns the wires,
connected terminal groups and unconnected pins. `schematic.draw_wire` accepts
terminal references, `{"junction":"junction-N"}` references from inspection, or
new free points as `{"x_nm":0,"y_nm":0}`, plus an ordered `bends` array and
`expected_revision`. Every leg must be horizontal or vertical. One complete route
is one undo step; `schematic.delete_wire` removes an inspected wire ID.
`schematic.place_symbol` also accepts an optional `rotation` of 0, 90, 180 or 270
counterclockwise degrees, defaulting to 0.

`schematic.move_items` and `schematic.delete_items` perform the same atomic group
edits as the editor. Both accept `items: {"symbols": ["symbol-2"], "wires": []}`
and `expected_revision`; moving also takes `dx_nm` and `dy_nm`. Obtain IDs from
inspection. Missing IDs, stale revisions and invalid geometry reject the complete
operation. Selecting a group and the live rectangle remain window state.

## Where things go

| Path | What it is |
|---|---|
| `crates/engine` | The session and the command set. The schematic and board model goes here. No UI. |
| `crates/mcp` | The MCP server, a thin layer over the engine's commands. |
| `crates/craft-control` | Bounded line framing and authenticated local transport. |
| `crates/app` | The window, shared by desktop and browser. Menus run engine commands. |
| `crates/chrome` | Theme, fonts, menus, bars and dialogs in the Craft style. Stands in for the shared Craft UI crate. |
| `apps/` | The desktop, command line and browser programs. They only start things. |

To add a feature: add a command to `crates/engine/src/lib.rs`, then call it from a menu or tool. MCP's `list_commands` and `execute` pick it up on their own.

The [PCB workspace plan](docs/board-workspace-plan.md) describes the proposed
first board milestone, including split and detached editor views, cross-probing,
footprint mapping and reviewed schematic-to-board updates.

## Checks

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo deny --locked check licenses sources bans
```

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE) and [ATTRIBUTION.md](ATTRIBUTION.md).
