# AI assistants and MCP

PCBCraft offers an in-app Assistant and a live MCP connection on desktop.
The CLI can also run a separate headless session. Project edits use the
same engine commands and undo history within each session.

`pcb.preview_board_setup` returns the [setup change review](pcb/board-materials.md#review-a-setup-edit)
with affected object IDs, consent/refusal and a session-local token, even with
`inspect: false`. `pcb.set_board_setup` optionally consumes that token;
`pcb.cancel_board_setup_preview` revokes it without an authored edit.
`pcb.inspect_layer_groups` returns derived outer/inner/plane/signal and user groups
with layer IDs and names, including with `inspect: false`. It is read-only and also
runs in a `--read-only` session. User
memberships are edited in the complete setup candidate, not a separate mutation.
Omitting `stackup.layer_groups` in a setup request preserves existing named groups
(pruning memberships only for removed copper); pass an explicit `[]` to clear them.
Mixed copper belongs to plane and signal. Preview reports
members of removed copper by proposed group UUID, even for newly added groups.
See [layer groups](pcb/board-materials.md#layer-groups).

The built-in Assistant instructions and the server instructions sent to external
MCP clients ask agents working in a live editor to keep the area they are editing
visible, unless the user requests otherwise. Agents are asked to inspect the
views before starting, frame planned work with `view.center` and `view.zoom`,
and reframe after meaningful batches as needed. `view.fit` provides a
whole-document overview. Requests to keep the camera fixed take precedence,
and unrelated panes should be left alone. This is guidance for the agent;
the editor does not automatically move the camera when an agent edits.

Projects from the previous split layout can be copied into the current format
with `pcbcraft-cli convert SOURCE DESTINATION` before starting MCP. A `.pc`
destination creates a ZIP; other destinations create a new working folder. See
[conversion support and limits](formats/native-project-storage.md#convert-a-previous-split-project).

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

For a quick check, open `crates/app/tests/fixtures/split/two-resistors/project.json`. Use
`inspect_project` to get R1's symbol ID and the current revision. Call `execute`
with `schematic.move_symbol` and pass the ID as `symbol`, coordinates as integer
`x_nm` and `y_nm`, and the inspected revision as `expected_revision`. Move R1 to
`(0, 5_080_000)` and watch the attached wire follow it. Undo and redo in the
editor, then inspect again to confirm the position. The edit stays unsaved
until you save the project. The move command also accepts optional absolute
`rotation` in degrees (0, 90, 180 or 270), applying movement and orientation in
one undo step; omitting it preserves the current orientation. `disconnect: true`
leaves the old wire paths in place while applying the new pose.
`schematic.mirror_symbol` reflects the addressed symbol about its center across
the screen's `horizontal` (left-to-right) or `vertical` (top-to-bottom) axis,
composes with its quarter-turn rotation, keeps attached wires on their terminals,
and is one undoable revision-checked edit. Applying the same axis twice restores
the original pose; inspection publishes `mirrored` on each affected symbol.

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
positions remain symbol-local nanometres. Optional `designator_locked` sets the
lock for the entire part instance; omitting it retains the current lock.
`dry_run: true` previews the full changed native documents without applying the
edit or adding an undo step. See [designator locks and previews](quickstart.md#designator-locks)
for the response, revision requirements, and limits.

`schematic.place_net_label` and
`schematic.set_net_label` accept optional `text` with the same appearance schema;
omission preserves a marker's existing appearance. The strict schemas appear in the shared
command catalog, and dangling field layouts or invalid values reject the entire
transaction.

For a project with several sheets, pass its inspected sheet UUID as `sheet` to
schematic commands and footprint placement. Omission remains compatible with
single-sheet projects; ambiguous and unknown targets are refused. `sheet.create`,
`sheet.rename` and `sheet.delete` use the same project-wide undo history as edits.
`sheet.set_page` sets a sheet's `paper` (`a4` to `a0`) and `orientation`
(`landscape` or `portrait`), keeping the paper's top-left corner unless `at` moves
it; each inspected sheet reports its `page`.
`sheet.set_template` gives a sheet a frame template (`template`
`{library, name, revision}` from `template.list`) or a plain border (`template:
null`); the paper stays put. `template.list` needs no open project.
`parameter.set` sets, renames or removes (`value: null`) a project or sheet
parameter, with `scope` `project` or `sheet`; a `rename` keeps the value unless
`value` gives a new one, and refuses `value: null`; `parameter.list` returns a list's own
values and every parameter it offers with its source; `parameter.resolve`
replaces the `@project`, `@sheet` and `@instance` references in a `text` as a
given `sheet` shows it and lists unknown references.
Inspection includes each sheet's own schematic, while the legacy top-level
`schematic` is available only for single-sheet projects.

The live connection runs every project command in `list_commands`, such as
`schematic.delete_wire` or `edit.undo`, in the window's own session, so the
editor and the agent share one undo history and the status bar names each edit
(for example **Delete Wire through MCP**). `project.new` waits until the open
project's unsaved changes are saved or discarded in the editor. Headless-only
commands, such as the [KiCad schematic import](schematic/kicad-import.md), are not in the
live `list_commands` and fail with `unavailable` there. Commands normally
wait while an inline text edit, dialog or file operation is open. Addressed
`app.library_pin_remap` apply/cancel may resolve its own remap dialog; a new live
preview and other gated document/presentation commands still refuse that dialog.
Inspection remains available; separate `ui.*` automation uses its own input lifecycle.
`file.abandon` can release a stalled open without waiting for the file job.
`inspect_editor` reports `text_editing`, `dialog_open` and `file_operation` so
clients can identify the state holding input. Its `file` object gives
the open project's saved `location` (`path` and `format`), the
running `operation` (`id`, `kind` and known `destination`) and the `last`
result (`id`, `operation`, `status` `succeeded`/`failed`/`cancelled`/`abandoned`, `path`,
`error`). Jobs run one at a time; a `last.id` beyond the one a client started
means a later job replaced its result. `location` is null until the project has been
opened from or saved to a location, whatever its name: a renamed project that was
never saved has none, while a saved `Untitled.pc` has one. Use `file.save` when
`location` is present and `file.save_as` when it is null.

`file.archive_folder` takes the absolute `path` for a new compressed `.pc`
outside the active folder. It starts a save job, verifies the archive, and
switches to it. The original folder stays by default. Pass `trash_source: true`
to request system Trash after verification. If the archive is saved and only
the source move fails, `file.last.status` is `failed`, its `error` explains why,
and `file.location` reports the new archive.

For pin remapping, normal live execution/`inspect_editor` exposes the captured
token and report at `library_editors.pin_remap`; apply/cancel uses its string
`document`, `expected_revision` and `token`. The token is runtime correlation,
not an authentication secret. `inspect:false` on this window command returns only
an acknowledgement/project revision, not the capture. Native preview counts the
complete prospective inspection reply against the existing 16 MiB JSON limit
before publishing a new capture/modal; oversize refusal preserves the previous
capture. No report rows are silently truncated. This bounds encoded reply bytes,
not peak heap. All report routes enforce the common 4 MiB report budget. The
[editor guide](libraries/properties-and-library-editors.md#review-pin-renumbering) describes
conservative graph-input and path-text admission, which runs only for a
connectivity lookup on an associated, placed terminal.
A failed form refresh keeps its old token/map owned by the form, so its visible Cancel
or dismissal can retire it. Refresh can adopt a changed project snapshot while the
library revision is still current; if the library revision changed, cancel and reopen
the form at the current library revision. A project-close decision that displaces the form retires
its review even if closing is cancelled. Addressed apply/cancel cannot close another form;
changing typed numbers
withdraws the old capture. Unrelated later editor changes can affect inspection
size, so this is a capture-time check, not permanent transport availability.
Headless `library.preview_pin_remap` is a focused read-only query and preserves
its report even with `inspect:false`; it does not host the live editor documents.
Preparation is synchronous and can outlast a request deadline. After a lost/timed-out
reply, inspect both library revision and capture state before retrying. A successful
unchanged Apply consumes the token without advancing the library revision. If full
inspection is too large, use visible Cancel or send an addressed Cancel with known
routing fields and `inspect:false`. Disconnect does not roll back a captured review
or an applied edit, and cannot guarantee delivery of a returned error.

It also lists presentation commands, marked `"scope": "window"`, which run through
the same handlers as the editor's own controls without editing the project, its
revision or undo history. `file.save` and `file.save_as` change the unsaved
state, and a completed save can also adopt project metadata without advancing the
revision: the first Save As of an Untitled project takes the destination's name,
and the session and its undo history adopt that name, the storage layout and sheet
occurrence IDs. Inspect the project again after every successful save, even when
the revision is unchanged. `file.open` replaces the project and `file.close`
unloads it, ending its undo history. The separate
`ui.` input commands described below can invoke authored edits. Call `inspect_editor` for the window,
view and arrangement IDs they take, with each view's workspace, selection, zoom, center and grid:

| Commands | Matching control |
|---|---|
| `view.focus`, `view.split`, `view.open_beside`, `view.close`, `view.open_window`, `view.detach`, `view.dock` | Project and definition pane headers and **View → Split / Detach / New window**. `view.open_beside` takes optional `beside` and `side` to open source `view` inside a destination pane. `view.dock` accepts source `view`, destination `beside` and optional `side` (`top`, `bottom`, `left`, `right`) for an explicit move within or across windows; `scope: "window"` splits the entire destination window. |
| `view.fit`, `view.center`, `view.zoom` | **Fit**, Browser centering and wheel zoom |
| `view.open_sheet` | A sheet row in the Browser; takes the view ID and sheet UUID |
| `app.set_workspace` | Choose a workspace in an addressed pane, here, beside it or in a native window. Use `render` for the desktop board preview. |
| `app.set_render_board` | **Browse…** in 3D render. Takes the view ID and a board UUID from the open project. |
| `app.board_image` | In a 3D render pane, enable GPU ray tracing (`render`) or return to the raster preview (`preview` or `cancel`). `inspect_editor` reports the pane's `ray_tracing` status. |
| `view.set_workspace` | Project view placement: schematic or PCB layout, here, beside or in a new window |
| `app.library_new`, `app.library_fork_selected`, `app.library_edit`, `app.library_focus`, `app.library_view`, `app.library_open`, `app.library_save`, `app.library_close` | Definition creation/forking, revision-checked edits, document focus, per-pane authoring ribbon/panels/navigation and open/save/export/close |
| `app.library_pin_remap` | Library **Remap pins…**: preview explicit numbers/current pinned evidence, then apply/cancel the captured review; definition-only, no placed adoption |
| `pcb.align_footprints` | Align by bounds (`left`, `right`, `top`, `bottom`, `center_x`, `center_y`) or distribute along X or Y (`distribute_x`, `distribute_y`). Omit `pitch_nm` for equal gaps by bounds; supply it for exact origin spacing. See [set-pitch distribution](pcb/set-pitch-distribution.md#set-pitch-distribution). Track ends follow moving pads in the same undo step |
| `pcb.set_footprint_transform` | Set a footprint center, whole-degree orientation and optional `side` (`front` or `back`) through the same atomic command used by the PCB canvas; inspection reports each footprint's `side`. Track ends follow the pads that move on their layer. Edits that join two nets are refused |
| `pcb.set_footprint_reference_offset` | Move the reference of a placed footprint. `offset` is a local `{x_nm, y_nm}` displacement from the library default. A zero offset restores that default |
| `pcb.set_footprint_reference_text` | Set sparse placement-local `appearance` fields for printed text, font, paired size/stroke, local rotation, alignment and visibility, with an optional `offset`. Null appearance resets inheritance. The component's reference and schematic use the separate shared reference edit |
| `pcb.delete_footprints` | Delete the addressed board placements atomically; retain schematic components and pinned physical assignments for re-placement |
| `view.set_grid`, `view.select`, `view.select_footprints` | Status-bar grid spacing, remembered separately for schematic and PCB views; click and box selection in a schematic; footprint and Browser selection in a PCB view; optional `element: "reference"` selects one placement's text and Properties |
| `view.set_selection_filters` | Per-view Selection filters checkboxes, `collapsed` and `open` card state. On PCB views, optional `include_locked: false` excludes locked footprints, tracks and vias from selection (default: `true`); project inspection is unchanged. Selection commands refuse disabled targets |
| `view.set_panel_position` | `panel`: `layers`, `properties` or `selection_filters`; `position`: `left`, `right` or `floating`, with optional logical `x`/`y` offsets when floating. `inspect_editor` reports each view's `panel_positions`. Properties also supports definition panes. |
| `arrangement.select`, `arrangement.create`, `arrangement.rename`, `arrangement.delete` | **File → Layouts**: **Activate**, **New arrangement**, **Duplicate…**, **Rename…** and **Delete…** |
| `window.set_panel`, `window.set_ribbon_tab`, `window.close` | Browser/Parts/Nets toggles, ribbon tabs (Capture, Sheets, Analyze, Libraries and View for schematics; Layout, Draw, Analyze, Manufacture, Libraries and View for PCB; `manage` remains an alias for `sheets`, and PCB `route` remains an alias for `layout`), closing a non-last window |
| `app.set_preferences`, `app.set_home` | **Preferences** (theme, scale, units, panel style, grid opacity, PCB grid style, schematic grid outside paper, paper background mode, per-theme sheet/canvas colours, selection following, navigation, recovery snapshot and save-to-project intervals) and **Home**. Explicit grid visibility takes precedence over an accompanying Off style or zero opacity |
| `file.save` | **File → Save** for a project that already has a location |
| `file.close` | **Close Project** returns to Home. If the project is dirty, the editor asks the user whether to save, discard, or cancel. Use `inspect_editor` to check the result. |
| `file.save_as`, `file.open` | **File → Save As** and **File → Open** with an absolute `path` instead of the picker. Save As takes `format` `folder` or `archive` (`.pc` is added when missing) and refuses an existing file or project folder, while an empty folder becomes the project folder; an Untitled project takes the destination's name. Open accepts a project folder, its entry point or a `.pc` archive and is refused while the project has unsaved changes. Both run in the background through the editor's file workers: keep the `file.operation.id` the reply reports and follow `inspect_editor` until `file.last.id` is at least that ID. Read `file.last.status` only when the IDs are equal; a greater `file.last.id` means a later job replaced the result, so report it as unavailable rather than taking that job's outcome as yours. A failed or abandoned job keeps the current design, unsaved state and history. An unavailable result says nothing about the outcome: the earlier job may have saved or opened, and that is not undone, so inspect the project and `file.location` before going on |
| `file.abandon` | **Stop waiting** in the Close Project wait: releases the editor from a running open, such as one stalled on an unresponsive disk. Pass the observed `file.operation.id` as `operation`; a different running job is left alone. Other commands wait while a file job runs, but this one does not. The late result is discarded, the current project is unchanged and `file.last` reports `abandoned`. Saves are refused because they may still be writing; the editor's Close Project wait always offers **Close without waiting**, which warns that unsaved edits are lost and that a first save may leave nothing on disk. At most four abandoned opens may still be running before abandoning is refused |

Closing the last window stays in the editor, which asks about unsaved changes.
Headless sessions refuse window commands with a reason.

The native live connection also exposes `ui.handles`, `ui.at`, `ui.project`,
`ui.window`, `ui.resize` and queued pointer, keyboard, text and wheel input
through `execute`. See [UI automation](automation/ui-automation.md) for named targets,
window-local coordinates, window sizes, completion tickets and cancellation.
These calls interact with ordinary controls, including dialogs, and may author
edits or reach file controls. Headless sessions cannot run them.

### Preferences and external updates

`app.set_preferences` changes personal editor settings, including the
Preferences → Saving intervals `autosave_minutes` and `save_to_project_minutes`;
it never changes project documents or project Undo, and writes no project file
during the call. Enabling `save_to_project_minutes` lets the editor later save
an eligible dirty project to its existing location on that schedule without
another command.

## Net wire appearance

`nets.inspect_appearance` reports a logical net’s effective schematic wire
override and identity status. `nets.set_appearance` takes `expected_revision`,
a net `target` and an `appearance` patch. Send an RGB `color` array or a
`width_cp` integer from 50 to 800 hundredths of a point to set those channels.
Use a nonempty `clear` array naming `color` or `width_cp` to remove individual
overrides, or use `{}` or `null` to reset both.

The edit promotes an eligible derived net in the same Undo step and rejects stale
revisions. The commands work in live and headless sessions and share the command
path used by schematic Properties. The overrides are persisted against the
[logical net registry](formats/native-project-storage.md#net-wire-appearance).
These are additions to command API version 1.

Use `nets.set_properties` to edit up to 64 distinct current logical nets in one
Undo step. Send `targets`, `expected_revision` and a `changes` object with an
optional `appearance` patch, `add_classes` and `remove_classes`. Omitted fields
keep their values on each net. Class lists update assignments to existing rule
definitions. An invalid or repeated electrical target rejects the entire edit.

## MCP reference

- <a name="command-api-version"></a>[Command API version](automation/command-api.md#command-api-version),
  <a name="stating-a-version"></a>[version pins](automation/command-api.md#stating-a-version) and
  <a name="error-codes"></a>[error codes](automation/command-api.md#error-codes)
- <a name="headless-mcp"></a>[Headless MCP](automation/headless-mcp.md)
- <a name="native-part-definitions-and-quantities"></a>[Native part and quantity commands](automation/part-commands.md)
- [Board and connectivity commands](automation/board-commands.md)
- [Generate fabrication files and review a ZIP](manufacturing/fabrication-mcp.md)
