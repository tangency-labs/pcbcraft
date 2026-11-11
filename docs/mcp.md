# AI assistants and MCP

PCBCraft offers an in-app Assistant and a live MCP connection on desktop.
The CLI can also run a separate headless session. Project edits use the
same engine commands and undo history within each session.

## Assistant in the desktop editor

Open **Assistant** from the chat icon next to Preferences or press
**Ctrl+Shift+A** (**Cmd+Shift+A** on Mac). Drag the title to switch sides,
drag the inside edge to resize, or use **Pop out Assistant** to move the chat
into its own window. **Dock Assistant** returns it to the workspace.
Minimize folds the panel into the bottom-right strip. Close hides the panel
and keeps the conversation available when you reopen it.

The Assistant talks to a coding agent you already have, over the
[Agent Client Protocol](https://agentclientprotocol.com) (ACP), as SolidCraft's agent chat does.
PCBCraft finds installed agents by itself: Claude Code (through
`npm install -g @agentclientprotocol/claude-agent-acp`), Codex
(`npm install -g @agentclientprotocol/codex-acp`), Gemini CLI, OpenCode, GitHub Copilot CLI,
Grok and Antigravity. Agents that aren't installed are listed with their install command. Pick an
agent in the dropdown and choose **Connect**; if it needs a sign-in, its own sign-in methods
appear. The agent's own options, such as its model, reasoning effort and mode, appear under the
dropdown when it lists them, and the chosen model is remembered for that agent.

The agent works on the open project through its own live MCP connection, which PCBCraft starts
for the chat and revokes when it disconnects. It runs the same commands as the menus and any MCP
client (see [Live MCP](#live-mcp-in-the-desktop-editor)), so every edit shows at once and is one
**Undo**; the conversation notes each edit. Its own shell and file tools are turned off where
the agent allows it. `pcbcraft-cli` must be beside the PCBCraft program (as in `target/debug`)
for the agent to reach the project. The agent starts it as
`pcbcraft-cli mcp --connect PORT --token-file FILE --wait 300`: the endpoint's credential stays in a
file only you can read, in its own temporary folder apart from the agent's, so it is never in the
agent's session, command line or environment, and the file goes when the chat disconnects.

The permission mode decides what the agent may do without asking. **Auto-allow project edits**
(the default, as in SolidCraft) runs project and view commands at once, while saving, starting
a new project, closing windows and deleting arrangements wait for **Allow**, **Always allow** or **Deny** in the chat.
A call nobody answers within four minutes is declined, and the agent is told it didn't run.
**Ask every time** asks before any command; **Allow everything** asks before none. Reading the
project always runs. Questions the agent asks about its own tools appear in the chat too.
**Stop** (or **Esc** in the chat) ends the answer and denies anything waiting. Tool calls run
only while a message is being answered: after **Stop** or a new chat (switching projects starts
one), a call still on its way is refused. The agent's own questions about using PCBCraft's tools are
answered for it, since the permission mode still applies to each command, but only for this
chat's server, which gets a random name per connection. Turn off **Approve agents' requests to
use PCBCraft's tools automatically** in Preferences → AI to confirm those questions too.

Open **Preferences → AI** from Home or the gear in the app bar to set the permission mode for all
agents or one, choose the default agent, **Test** that an agent starts and answers, forget its
**Always allow** answers, **Refresh** the installed agents, and set where and when the Assistant
opens. **Add agent…** adds another ACP agent by its command line, which runs without a shell, or
a model endpoint: OpenAI-compatible (Ollama, LM Studio, OpenAI, OpenRouter and other presets) or
Anthropic-compatible, with its base URL, model, optional key, request timeout and reasoning
effort. For an endpoint PCBCraft runs the conversation itself and gives the model the same project
tools through the same permission gate. Its key is kept in the system keychain (Keychain on macOS,
Credential Manager on Windows, the Secret Service on Linux), never in PCBCraft's settings, and is
only sent over HTTPS or to this computer, and only to the address it was entered for: changing an
endpoint's address asks for its key again. **Test** lists an endpoint's models and checks that its
model answers.

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
until you save the project. The move command also accepts optional absolute
`rotation` in degrees (0, 90, 180 or 270), applying movement and orientation in
one undo step; omitting it preserves the current orientation. `disconnect: true`
leaves the old wire paths in place while applying the new pose.

`schematic.set_symbol_fields` applies reference, value and optional `rotation`,
`x_nm` and `y_nm` together as one revision-checked undo step, matching the schematic
Properties inspector. Omitted placement fields retain their values. Coordinates
are integer nanometres; GUI unit suffixes are converted before the same command
validation runs. Position changes retain attached wires and use the ordinary
exact-contact rules, with optional `auto_heal`. A failed field or contact check
leaves the whole project and history unchanged. Optional `metadata` replaces the
component's standard/custom fields and attributes; optional `field_layout`
replaces only the addressed unit's text appearance. Omission preserves either
map. Inspection publishes component metadata and each symbol's field layout;
positions remain symbol-local nanometres. `schematic.place_net_label` and
`schematic.set_net_label` accept optional `text` with the same appearance schema;
omission preserves a marker's existing appearance. The strict schemas appear in the shared
command catalog, and dangling field layouts or invalid values reject the entire
transaction.

For a project with several sheets, pass its inspected sheet UUID as `sheet` to
schematic commands and footprint placement. Omission remains compatible with
single-sheet projects; ambiguous and unknown targets are refused. `sheet.create`,
`sheet.rename` and `sheet.delete` use the same project-wide undo history as edits.
Inspection includes each sheet's own schematic, while the legacy top-level
`schematic` is available only for single-sheet projects.

The live connection runs every project command in `list_commands`, such as
`schematic.delete_wire` or `edit.undo`, in the window's own session, so the
editor and the agent share one undo history and the status bar names each edit
(for example **Delete Wire through MCP**). `project.new` waits until the open
project's unsaved changes are saved or discarded in the editor, and every
command waits while an inline text edit, dialog or file operation is open.
`inspect_editor` reports `text_editing`, `dialog_open` and `file_operation` so
clients can identify the state holding command input.

It also lists window commands, marked `"scope": "window"`, which run through the
same handlers as the editor's own controls and never edit the project, its
revision or undo history; only `file.save` changes the unsaved state. Call `inspect_editor` for the window,
view and arrangement IDs they take, with each view's workspace, selection, zoom, center and grid:

| Commands | Matching control |
|---|---|
| `view.focus`, `view.split`, `view.open_beside`, `view.close`, `view.open_window`, `view.detach`, `view.dock` | Project and definition pane headers and **View → Split / Detach / New window** |
| `view.fit`, `view.center`, `view.zoom` | **Fit**, Browser centering and wheel zoom |
| `view.open_sheet` | A sheet row in the Browser; takes the view ID and sheet UUID |
| `app.set_workspace` | Choose Schematic, PCB layout, Symbol Editor or Footprint Editor in an addressed pane, here/beside/in a native window; definition workspaces resume their last document |
| `view.set_workspace` | Project view placement: schematic or PCB layout, here, beside or in a new window |
| `app.library_new`, `app.library_fork_selected`, `app.library_edit`, `app.library_focus`, `app.library_view`, `app.library_open`, `app.library_save`, `app.library_close` | Definition creation/forking, revision-checked edits, document focus, per-pane authoring ribbon/panels/navigation and open/save/export/close |
| `pcb.align_footprints` | Align two or more `footprints` by their physical bounds (`mode` `left`, `right`, `top`, `bottom`, `center_x`, `center_y`) or distribute three or more with equal gaps to nanometer precision (`distribute_x`, `distribute_y`), in one undo step; distribution keeps physical outermost footprints and the span fixed, refusing overlapping selections that cannot retain those anchors. Track ends follow moving pads, as for a transform |
| `pcb.set_footprint_transform` | Set a footprint center, whole-degree orientation and optional `side` (`front` or `back`) through the same atomic command used by the PCB canvas; inspection reports each footprint's `side`. Track ends follow the pads that move on their layer. Edits that join two nets are refused |
| `pcb.set_footprint_reference_offset` | Move the reference of a placed footprint. `offset` is a local `{x_nm, y_nm}` displacement from the library default. A zero offset restores that default |
| `pcb.delete_footprints` | Delete the addressed board placements atomically; retain schematic components and pinned physical assignments for re-placement |
| `view.set_grid`, `view.select`, `view.select_footprints` | Status-bar grid spacing; click and box selection in a schematic; footprint and Browser selection in a PCB view |
| `view.set_selection_filters` | Per-view Selection filters checkboxes, `collapsed` and `open` card state; selection commands refuse disabled object categories |
| `view.set_panel_position` | `panel`: `layers`, `properties` or `selection_filters`; `position`: `left`, `right` or `floating`, with optional logical `x`/`y` offsets when floating. `inspect_editor` reports each view's `panel_positions`. Properties also supports definition panes. |
| `arrangement.select`, `arrangement.create`, `arrangement.rename`, `arrangement.delete` | The named arrangement tabs: click, **+** and **Duplicate…**, **Rename…**, **Delete** |
| `window.set_panel`, `window.set_ribbon_tab`, `window.close` | Browser/Parts/Nets toggles, ribbon tabs (Capture, Analyze, Libraries, Manage and View for schematics; Layout, Libraries and View for PCB), closing a non-last window |
| `app.set_preferences`, `app.set_home` | **Preferences** (theme, scale, units, grid opacity, PCB grid style, selection following, navigation) and **Home** |
| `file.save` | **File → Save** for a project that already has a location |

Closing the last window and choosing where to save an Untitled project stay in
the editor, which asks about unsaved changes. Headless sessions refuse window
commands with a reason.

## Headless MCP

`pcb.add_drawing`, `pcb.set_drawing` and `pcb.delete_drawing` use the same
transactions as canvas Board Outline editing. Each takes `board` and
`expected_revision`; creation also takes a `layer` UUID, ordered `points`
(`x_nm`, `y_nm` integers) and `closed`. Updating takes `drawing`, `points` and
`closed`; deleting takes `drawing`. Inspection exposes layer metadata and drawing
identities at `project.boards[].layers` and `.drawings`. Paths contain 2–256
vertices (at least three when closed), without a repeated closing vertex. Only
Board Outline layers accept these graphics in this slice. Open paths are allowed,
without a manufacturing or clearance claim.

`pcb.add_tracks` and `pcb.delete_tracks` use the same transactions as the PCB
route tool. Each takes `board` and `expected_revision`. Routing also takes a copper
`layer` UUID from `project.boards[].layers`, `width_nm` (10,000 to 10,000,000) and
2–256 ordered `points`; each neighboring pair becomes one straight segment, all in
one undo step. Optional `layer_changes`, a list of `{at, layer}` in increasing
vertex order after the first, places a through via at `points[at]` and continues on
that copper layer; a change at the last vertex ends the route on a via. Layer changes
require `via_diameter_nm` and `via_drill_nm`, with the drill smaller than the pad.
`pcb.check_design` checks the addressed board without editing it; `execute` returns
`design_check` with the `rules` used, `violations` (each with a `kind`, `message`,
representative `at` point and `items`), `counts` per kind and the `total`, listing at
most 1000 findings. Optional `clearance_nm` (default 200000), `min_track_width_nm`
(150000), `edge_clearance_nm` (300000) and `min_annular_ring_nm` (100000) override the
defaults. Kinds are `clearance`, `track_width`, `annular_ring`, `unconnected`,
`no_outline`, `outside_board`, `edge_clearance` and `courtyard_overlap`.

`pcb.export_fabrication` generates the addressed board's fabrication files without
editing it; `execute` returns `fabrication.files`, each with a `name` and text `contents`:
Gerber X2 copper per layer, mask and paste top and bottom, top silkscreen and the outline,
and one Excellon plated drill file. It needs a board setup and a closed Board Outline.
File names are deterministic and unique after portable-name sanitization,
including case-insensitive collisions with fixed-purpose output names. One
`execute` request generates the file contents once; `apply` still validates
exportability without changing the project or revision.

`pcb.add_via` places one standalone through via from `at`, `via_diameter_nm` and
`via_drill_nm`, in one undo step, refusing a second via at the same position or a pad
that would join two nets. `pcb.move_tracks` moves `tracks` and/or `vias` by `dx_nm` and
`dy_nm`, keeping sizes and layers. `pcb.reshape_tracks` sets new `start` and `end` points for each listed `track` at once,
keeping layers and widths; a collapsed segment is refused. `pcb.set_track_width` gives
`tracks` one `width_nm`,
and `pcb.set_via_size` gives `vias` one `via_diameter_nm` and `via_drill_nm`. Each is one
undo step, checks shorts for the copper it changes, and is not an edit when nothing
changes. Deleting takes `tracks` and/or `vias`, unique UUIDs from `project.boards[].tracks`
and `.vias`; at least one item is required, and deleting a via keeps its tracks. A route is refused whole when a point is out of range,
neighbors repeat, the layer is not copper in the board setup, or its copper would
join pads on two different nets (the error names both). Only copper the new route
touches is checked, so a short left by an earlier edit does not block unrelated routes.
Copper connects where it overlaps on a shared layer, a via joins the copper it overlaps
on every layer, and the board ratsnest drops airwires between terminals already joined
by tracks. Routing does not check clearance; use `pcb.check_design` for that.

Live `view.set_layer` uses the same Layers handler for `active` and `visible` changes on an addressed view; those
presentation changes do not alter project revision or undo.
`view.set_board_flipped` takes `view` and `flipped` and shows that board view from below
or above through the same handler as **Design → Flip view**, keeping the centered point
and zoom; `inspect_editor` reports `board.flipped` per view. It changes no project data.
`view.set_layer_visibility` takes `view` and `mode` (`all` or `active`) through the
same handler as Show all / Isolate active. Isolation reveals the active layer,
hides the others and cancels unfinished drawing in that view.
`view.set_layer_color` takes `view`, `layer` (a layer UUID) and `color`
(`#RRGGBB`, or null to reset). It uses the swatch form's handler and shares the
personal board/layer palette across all views. `inspect_editor` reports effective
`board.layer_colors` per view, including whether each value is overridden.
The earlier `board.copper_colors` subset remains for compatibility.
These values persist locally without changing project revision or undo. Physical
pad inspection also exposes `pins[].copper_layers`: `resolved` includes the
binding and stack-ordered layer UUIDs; `unconfigured` retains the explicit library
binding without invented layers. Marker-only pins omit physical membership.
`view.navigate_layer` takes `view` and `direction` (`next`, `previous`, `front`,
`back`) through the keyboard's handler. Next/previous wrap in picker order across
technical and hidden layers; front/back resolve physical copper side metadata.
It preserves visibility, project revision and undo, and rejects unknown views or
unavailable copper sides before changing anything.

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

Library authoring actions now include atomic `replace_many`, group `transform`,
`duplicate`, typed-primitives `paste` and rectangular `array`, alongside the
existing insert/replace/delete/history operations. They all require the current
library document revision. Array row/column vectors and transform pivot/offset
use integer-nanometre `Point` objects; copies allocate unused terminal/pad
numbers. `app.library_view` can set a distinct `selected_objects` ID array instead
of scalar `selected`; per-pane inspection reports `selection`. These are live
editor contracts; headless project sessions do not host authoring documents.

### Placing an accepted footprint group

`pcb.place_assigned_footprints` places a specified list of assigned, accepted
`components` at `x_nm`/`y_nm`, with optional quarter-turn `rotation`, explicit
`board` and the current `expected_revision`. Packages use pinned definitions and
mappings; initial spacing follows their resolved geometry. Existing placements
stay fixed, and the entire group is one undo step. Duplicate/unknown, already
placed, excluded or pending assignments refuse the whole command. The native
Update PCB cursor handoff commits through this same command. Inspect and describe
the command before supplying IDs or coordinates; spacing is not a DRC result.
