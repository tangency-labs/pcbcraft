# PCBCraft

Electronics design that feels like the tools you already know, built so a person and an agent can work on the same board, at the same time, with the same commands.

Start with the [quickstart](docs/quickstart.md) for a small editable example,
the [capability table](docs/capabilities.md) for current limits, or
[CONTRIBUTING.md](CONTRIBUTING.md) to build and work on an issue.

**Status: just started.** Schematic editing works on desktop and in the browser.
The app starts on **Home**, where you can create or open a project. On desktop,
**Recent projects** lists the last 12 successfully opened or saved projects, newest
first, as cards with a rendering of the saved schematic, its name and path. Click
a card to reopen it; unsaved work gets the
usual Save/Don’t Save/Cancel question. Remove a card with **×**, or choose
**Clear recent projects**, without deleting any files. History is kept between
launches in application settings. A missing or unreadable project reports an error
and stays in the list until removed. Empty and unavailable previews have
placeholders; previews load in the background without opening or changing a project. Browser uploads have no reusable local path,
so recent projects are desktop-only. Home does not automatically reopen a project.

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

Click an unconnected symbol pin or free wire end to start directly. Press **W** to
start immediately at the pointer, including on a connected pin, junction, wire segment
or empty grid point. **Capture → Wire** instead waits for the first canvas click.
The green snapped crosshair marks the drawing point; a ring marks an existing
connection target. Ordinary clicks on connected nodes retain selection behavior.
Click to add corners, **Space** to flip the current
elbow, and **Backspace** to remove the last corner. Click a destination pin or endpoint
to finish, or press **Enter** or double-click to finish on empty space. **Esc** or
right-click → **Cancel** discards the draft. Click a completed wire segment to select it;
only that segment turns blue, and **Delete** removes it. Drag a segment perpendicular to its
direction to adjust alignment, or hover it and press **G**, then click to finish. Attached pins
and junctions stay fixed while the adjoining legs adjust. **Esc** cancels the preview.
Right-click → **Select Wire** selects the whole route for dragging or deletion; box and
modifier selections continue to select whole wires. Crossings stay disconnected.
Select a wire segment, then use Capture → **Wire edits** for **Slice**, **Break**,
**Heal** or **Delete**. The group caption opens the same actions as a menu.
Right-click a wire for these edits and whole-wire selection/deletion:
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
The Wire edits menu's **Auto-heal wires** toggle defaults on and is remembered as a
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
Split views have one closable header above each canvas showing its document path, such as
**Power controller › Main**. Arrows fold rows; unfold **Symbols** or **Nets** to list the sheet's
items, and click one to select it and center the view on it. A net row selects
one of its wires. Click the Browser's title to fold it. Browsing never changes the
project, and the canvas keeps its full size; **Fit** keeps the drawing clear of
the Browser. The ribbon's **View** tab shows or hides the Browser, Parts and Net
Inspector panels, and fits the view. Each ribbon group's caption, such as
**Place ⌄**, opens a menu of the same actions. A project has one sheet so far.

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
successful save. See the [arrangement persistence details](docs/board-workspace-plan.md#named-arrangements-and-reopening-a-project).

Browser builds support panes within one tab; native window actions are desktop
only. The workspace tile can switch to PCB layout, open it beside
the schematic or create a board window directly. Existing placements render as
pad-center markers with component counterpart highlights, grouped Layers and
read-only Properties. The editor still supports one schematic sheet and one
board; board editing and live-MCP view/window controls remain planned. See the
[board inspection walkthrough](docs/quickstart.md#inspect-a-board-beside-its-schematic).

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
The [PCB representation plan](docs/pcb-representation.md) records layer/stackup
ownership, #75's unit contract and the proposed shared net/rule model. Its first
engine supports validated setup commands, custom materials, sourced Tg/Dk/Df
reports, core/prepreg plies, undo and native persistence. The
[material contract](docs/board-materials.md) documents these fields; the stackup
UI, impedance estimator, editable board canvas and rule resolver remain follow-up work.

## Checks

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo deny --locked check licenses sources bans
```

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE) and [ATTRIBUTION.md](ATTRIBUTION.md).
