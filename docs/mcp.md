# AI assistants and MCP

PCBCraft offers an in-app Assistant and a live MCP connection on desktop.
The CLI can also run a separate headless session. Project edits use the
same engine commands and undo history within each session.

`pcb.preview_board_setup` returns the [setup change review](board-materials.md#review-a-setup-edit)
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
See [layer groups](board-materials.md#layer-groups).

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
[conversion support and limits](native-project-storage.md#convert-a-previous-split-project).

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
project's unsaved changes are saved or discarded in the editor. Commands normally
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

For pin remapping, normal live execution/`inspect_editor` exposes the captured
token and report at `library_editors.pin_remap`; apply/cancel uses its string
`document`, `expected_revision` and `token`. The token is runtime correlation,
not an authentication secret. `inspect:false` on this window command returns only
an acknowledgement/project revision, not the capture. Native preview counts the
complete prospective inspection reply against the existing 16 MiB JSON limit
before publishing a new capture/modal; oversize refusal preserves the previous
capture. No report rows are silently truncated. This bounds encoded reply bytes,
not peak heap. All report routes enforce the common 4 MiB report budget. The
[editor guide](properties-and-library-editors.md#review-pin-renumbering) describes
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
the revision is unchanged. `file.open` replaces the project. The separate
`ui.` input commands described below can invoke authored edits. Call `inspect_editor` for the window,
view and arrangement IDs they take, with each view's workspace, selection, zoom, center and grid:

| Commands | Matching control |
|---|---|
| `view.focus`, `view.split`, `view.open_beside`, `view.close`, `view.open_window`, `view.detach`, `view.dock` | Project and definition pane headers and **View → Split / Detach / New window**. `view.open_beside` takes optional `beside` and `side` to open source `view` inside a destination pane. `view.dock` accepts source `view`, destination `beside` and optional `side` (`top`, `bottom`, `left`, `right`) for an explicit move within or across windows; `scope: "window"` splits the entire destination window. |
| `view.fit`, `view.center`, `view.zoom` | **Fit**, Browser centering and wheel zoom |
| `view.open_sheet` | A sheet row in the Browser; takes the view ID and sheet UUID |
| `app.set_workspace` | Choose Schematic, PCB layout, Symbol Editor or Footprint Editor in an addressed pane, here/beside/in a native window; definition workspaces resume their last document |
| `view.set_workspace` | Project view placement: schematic or PCB layout, here, beside or in a new window |
| `app.library_new`, `app.library_fork_selected`, `app.library_edit`, `app.library_focus`, `app.library_view`, `app.library_open`, `app.library_save`, `app.library_close` | Definition creation/forking, revision-checked edits, document focus, per-pane authoring ribbon/panels/navigation and open/save/export/close |
| `app.library_pin_remap` | Library **Remap pins…**: preview explicit numbers/current pinned evidence, then apply/cancel the captured review; definition-only, no placed adoption |
| `pcb.align_footprints` | Align by bounds (`left`, `right`, `top`, `bottom`, `center_x`, `center_y`) or distribute along X or Y (`distribute_x`, `distribute_y`). Omit `pitch_nm` for equal gaps by bounds; supply it for exact origin spacing. See [set-pitch distribution](pcb-layout-workflow.md#set-pitch-distribution). Track ends follow moving pads in the same undo step |
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
| `file.save_as`, `file.open` | **File → Save As** and **File → Open** with an absolute `path` instead of the picker. Save As takes `format` `folder` or `archive` (`.pc` is added when missing) and refuses an existing file or project folder, while an empty folder becomes the project folder; an Untitled project takes the destination's name. Open accepts a project folder, its entry point or a `.pc` archive and is refused while the project has unsaved changes. Both run in the background through the editor's file workers: keep the `file.operation.id` the reply reports and follow `inspect_editor` until `file.last.id` is at least that ID. Read `file.last.status` only when the IDs are equal; a greater `file.last.id` means a later job replaced the result, so report it as unavailable rather than taking that job's outcome as yours. A failed or abandoned job keeps the current design, unsaved state and history. An unavailable result says nothing about the outcome: the earlier job may have saved or opened, and that is not undone, so inspect the project and `file.location` before going on |
| `file.abandon` | **Stop waiting** in the Close Project wait: releases the editor from a running open, such as one stalled on an unresponsive disk. Pass the observed `file.operation.id` as `operation`; a different running job is left alone. Other commands wait while a file job runs, but this one does not. The late result is discarded, the current project is unchanged and `file.last` reports `abandoned`. Saves are refused because they may still be writing; the editor's Close Project wait always offers **Close without waiting**, which warns that unsaved edits are lost and that a first save may leave nothing on disk. At most four abandoned opens may still be running before abandoning is refused |

Closing the last window stays in the editor, which asks about unsaved changes.
Headless sessions refuse window commands with a reason.

The native live connection also exposes `ui.handles`, `ui.at`, `ui.project` and queued pointer,
keyboard, text and wheel input through `execute`. See [UI automation](ui-automation.md)
for named targets, window-local coordinates, completion tickets and cancellation.
These calls interact with ordinary controls, including dialogs, and may author
edits or reach file controls. Headless sessions cannot run them.

### Preferences and external updates

`app.set_preferences` changes personal editor settings, including the
Preferences → Saving intervals `autosave_minutes` and `save_to_project_minutes`;
it never changes project documents or project Undo, and writes no project file
during the call. Enabling `save_to_project_minutes` lets the editor later save
an eligible dirty project to its existing location on that schedule without
another command.

## Command API version

`list_commands` keeps returning the command array as its first text part. A
second text part, also given as the result's `structuredContent`, reads
`{"command_api_version": 1, "catalog_revision": "<64 hex digits>", "capabilities": ["command_api", "error_codes", "version_check", "deprecation_warnings"]}`.
These are three separate things:

- `command_api_version` is one number for the whole public command contract,
  `COMMAND_API_VERSION` in `crates/engine/src/command_api.rs`. It rises only
  when a published change breaks existing callers, by an explicit decision, and
  never falls. There is no version per command.
- `catalog_revision` is the SHA-256 of every command this session describes,
  sorted by id, with object keys sorted and titles, summaries and descriptions
  included. It changes whenever a command is added or removed or a description
  changes, so a client can drop cached discovery data; a new revision alone
  does not mean existing callers are incompatible. It covers the whole catalog
  whatever `filter` keeps. A live editor (project, window and `ui.` commands)
  and a headless session (project commands) describe different catalogs, and
  builds differ, so each reports its own revision; it is computed when asked,
  not stored.
- `capabilities` names the optional protocol features this build offers; a
  feature added later is appended and none is removed. Command schemas from
  `list_commands` and `describe_command` say which commands and parameters the
  running build supports.

None of them is the MCP `protocolVersion` negotiated by `initialize`,
`serverInfo.version` (the build's version) or a native file-format version,
and none is saved in projects.

[`docs/schemas/command-contracts.json`](schemas/command-contracts.json) is a
snapshot of every command's contract: `command_api_version`, then one line per
command in `commands` (project commands) and `window_commands` (window and
`ui.` commands), each the description without its `id` and without the
`title`, `summary`, `description` and deprecation annotations of the command
and its schemas, with object keys sorted (values of `const`, `default`, `enum` and `examples` are
kept whole). The engine and app tests compare the described commands with it
and label detected differences. If either parameter schema contains `$ref`,
`$dynamicRef` or `$recursiveRef`, every difference in it is conservatively
breaking: an unchanged reference may reach any target, including a property
or array item used under `not` elsewhere. The classifier does not resolve
reference targets or their polarity. The same conservative treatment applies
if either schema contains `unevaluatedProperties` or `unevaluatedItems`, even
nested: a locally looser alternative can stop evaluating a property or item,
so an unchanged outer restriction can refuse a formerly valid call. Both
scans also include data values, which can cause extra breaking labels. If
unequal schemas in these guarded contexts produce no child diagnostic (for
example, removing an empty map), the whole schema is reported as breaking.
Without references or unevaluated keywords, the rules are:

- compatible, when every call the old schema accepted is still accepted: a
  command added, an optional parameter added to an object that refuses
  unknown names (`additionalProperties: false` and no `patternProperties`,
  as every command's top-level parameters are), a required parameter made
  optional, enum values added, a type widened (`integer` to `number`
  included), a bound loosened, a `pattern`, `format`, `const`, `multipleOf`
  or `uniqueItems` dropped, `additionalProperties` or `items` loosened,
  `anyOf` alternatives appended, a `$defs` or `definitions` entry added,
  `examples` changed, or an equal-length `prefixItems` child loosened;
- breaking otherwise: a command or parameter removed, a parameter newly
  required, enum values removed, a type narrowed or added, a bound tightened or
  added, a `pattern`, `format`, `const` or `multipleOf` added or changed, a
  `default` added, changed or dropped (also by replacing a schema that
  carries one with one that accepts anything), a `$defs` or `definitions`
  entry removed or changed in any way (a definition may be used under `not`,
  where loosening it refuses calls), a property added to an object that
  accepted unknown names (a call may already pass that name with another
  value), a changed `prefixItems` length or a tightened child, any `oneOf`
  change (a value may then match two alternatives), any
  change under a keyword compared as a whole (`not`, `allOf`, `$ref`, `if`,
  `then`, `else`, `patternProperties`, `dependentRequired`,
  `dependentSchemas`, `dependencies`), and any change under a keyword without
  a rule.

The rules apply at every depth of a schema. Against an unchanged snapshot,
a detected breaking difference fails the tests until `COMMAND_API_VERSION`
is raised. After the raise, or for other detected differences, the failing
test prints the section to copy into the snapshot. Rewriting a snapshot in
the same commit instead needs comparison with a retained prior snapshot.
The snapshot drops a removed command; an id that comes back later is an
addition.

The tests see only the described parameter schemas. Changed units, a changed
result or meaning, and a stricter check the schema does not express need the
same explicit decision with no failing test to prompt it; and they cannot tell
whether a new default keeps the old behaviour. They also compare the code with
the snapshot in the same commit, so a breaking change committed together with
a rewritten snapshot passes them. When the pull request’s target commit has a snapshot, CI on
a pull request also compares that `command-contracts.json` with the pull
request's checked-out merge result, with the same classifier (the `command_contracts_against_base`
example of `pcbcraft-engine`, run as `cargo run -p pcbcraft-engine --example
command_contracts_against_base -- BASE.json HEAD.json`): it fails when the
classifier detects a breaking difference in either section without a raised
`command_api_version`, or when the version falls. If the target commit has no
snapshot, the workspace tests check the first snapshot against discovery
instead. The cross-revision check sees only recorded contracts, so a changed
meaning still needs review, and it cannot tell whether a raise was the right
decision; a review must still read
every change to `command-contracts.json`.

### Stating a version

Pass optional `version` to `execute` to pin the `command_api_version` from
`list_commands`. There is no per-command version. Omitting `version` runs
against the session’s current API. The MCP tool accepts a positive JSON
integer (for example `1`, not `1.0`, `"1"` or `null`). Check the serving
session’s `version_check` capability before pinning. Legacy MCP backends that
do not implement pinning refuse a pin without dispatching; unpinned calls
still work.

The MCP bridge and raw live clients that need a guarantee across editor versions
use `engine.execute_versioned` with a required non-null `params.version` beside
`params.command`. Use the `command_api_version` reported by the serving
editor's command discovery (`list_commands` over MCP, `engine.commands` on the
raw live endpoint). For example, if the editor reports version 1:

```json
{"method":"engine.execute_versioned","params":{"command":"edit.undo","version":1}}
```

A missing or null pin on that method is `invalid_request`; an older editor
rejects the method rather than executing the command. Use plain `engine.execute`
for unpinned requests. A current editor also checks a non-null `version` on
plain `engine.execute` (omitted or `null` means unpinned), but a legacy editor
ignores it and may execute the command. A malformed pin on a known command
that reaches the version check returns `invalid_params` on the live endpoint.

A known, routable project, window or `ui.` command with a different API version
returns `version_mismatch` without running. Refresh the catalog and adapt the
call. Unknown commands and a missing live editor keep their existing refusals.
Outer admission checks still run first, including assistant approval, ordinary
project/window editor refusal and deferred-job expiry, busy and editor refusal.
The `ui.` handler's own checks, such as busy or native focus, run only after a
matching or omitted pin.

### Deprecated commands and parameters

Mark a command or top-level parameter deprecated for at least one release
before removing it. Discovery annotates the command or its schema under
`params.properties` with `"deprecated": true` and
`"x-deprecation": {"since": "0.2.0", "replacement": "new_name"}`. The
replacement is a command id or parameter name, or `null` when there is none.
Marking a deprecation changes discovery and its catalog revision, not the
command contract snapshot or the API version.

A successful call that uses something deprecated still returns its ordinary
result as the first MCP text part. A second text part, also given as
`structuredContent`, carries `{"warnings": [...]}`. Each entry has `code`
(`deprecated`), `command`, `parameter` (`null` for a whole command), `since`,
`replacement` and a human-readable `message`. Passing a deprecated parameter
explicitly counts as use even when its value is `null` and the command accepts
it. The live endpoint puts `warnings` beside `result`; failures and successes
without warnings keep their previous execution reply shape. The
`deprecation_warnings` capability indicates support. No production command or
parameter is deprecated yet.

## Error codes

A tool call that fails after its arguments were accepted is still a result with
`isError: true` whose first text part is the message, as before. (Argument
faults caught by the MCP tool validator, such as a missing `command`, remain a
JSON-RPC `invalid params` error without a PCBCraft code.) A second text part, also given as the result's
`structuredContent`, reads
`{"error": {"code": "stale_revision", "message": "...", "details": {"current_revision": 12}}}`.
Branch on `code`, never on the message: messages may be reworded, codes are
not. `details` is an object, empty unless the table says otherwise; its keys are
only ever added. The live editor endpoint answers a failed request with
`{"error": message, "code": ..., "details": {...}}`; a reply without a code, from
an older editor, reads as `unavailable`. A well-formed code from a newer editor
that this list does not have yet is passed through unchanged; treat a code you
do not know like `unavailable`. Missing or non-object details in a live reply
read as an empty object. The table describes failures produced by this build;
a malformed reply from another editor may omit the facts it describes.

| Code | Meaning |
| --- | --- |
| `unknown_command` | No command has the requested id. |
| `invalid_params` | The parameters are a non-null scalar or array, name a parameter the command does not describe, leave out a required one, have the wrong type or range, or fail the command's own checks. |
| `unavailable` | The command cannot run in the current state: no project is open, nothing to undo, a dialog or file operation is open in the editor, and similar. |
| `stale_revision` | `expected_revision` is not the current revision. `details.current_revision` gives the current one; inspect it and retry. |
| `not_found` | An ID names nothing in the project, such as a deleted wire, symbol, net label, sheet or footprint. Some commands report an ID that is not of the expected form the same way, others as `invalid_params`. |
| `cap_exceeded` | The edit would pass a fixed limit, such as 256 net labels on a sheet, the sheet or wire limit, bends on a wire, or the project's object IDs, or a live `schematic.complete_packages` report would exceed the 16 MiB reply limit (nothing is edited). |
| `needs_live_editor` | A window or `ui.` command, or `inspect_editor`, in a session without an open editor. |
| `denied` | The person declined the request in the editor's assistant. |
| `cancelled` | The request was stopped, timed out or lost its connection before it finished; nothing changed. |
| `busy` | The editor is already running a request of the same kind, such as a design check, or has reached a worker or request capacity limit; retry later. |
| `no_reply` | No usable reply came back from the editor, so the command may have run; inspect before retrying an edit. |
| `connection_failed` | Connecting or sending the request failed. A failed send may still have delivered a complete request; inspect before retrying an edit. |
| `invalid_request` | The request to the live editor is malformed: an unknown method, a missing command id or a missing required version pin. |
| `version_mismatch` | The stated command API version is older or newer than the one this session serves; the command did not run. `details` gives `command`, `stated_version` and `current_version`. |
| `out_of_scope` | A known command is not allowed in the read-only session. `details.command` names it and `details.scope` is `read_only`. The check precedes revision and command parameter validation. |

A version pin is checked before the command’s parameters, including
`expected_revision`. Once that pin matches or is omitted, the read-only scope is checked.
After that, project commands that take `expected_revision` check it before
their other parameters. A stale
edit reports `stale_revision`; a malformed revision is `invalid_params`. The read-only [`pcb.plan_route`](route-planning.md#command)
query keeps its existing `stale` plan outcome for otherwise valid requests. The
part and library document revisions that some window commands take are reported
as `unavailable` when stale, for now.
At direct command and live control dispatch, catalog validation treats omitted
or null parameters as an empty map; required keys still apply. Other non-object
parameter values reaching that validation are `invalid_params`. At the outer
MCP `execute` boundary, omit `arguments.params` to use the same empty-map
behavior; a supplied `arguments.params: null` is a JSON-RPC `invalid params`
error. Deferred work can be cancelled or refused by capacity or live host policy
before engine validation. Once admitted, deferred copper checks stale revision,
then catalog parameters, then board identity like headless execution.
After live host policy permits a window request, its catalog reports
`unknown_command` for an unknown id and `invalid_params` for unknown keys,
missing required keys or invalid value types. A host-policy refusal remains
`unavailable`, before that classification. UI commands use their separate
input lifecycle: their catalog checks ids, parameter names and required keys;
other handler refusals, including UI value checks, are `unavailable` for now.
The list is locked in
[`docs/schemas/error-codes-v1.json`](schemas/error-codes-v1.json): codes are
only appended, never renamed, removed or reused; the engine tests fail when the
lock and the code disagree. A code that is no longer produced stays listed. A failure may move to a more
specific code in a later version: some lookups and limits still report
`invalid_params`, and some state checks `unavailable`, so treat those two as
the general cases they are. The capability
`error_codes` says a build reports them.

## Headless MCP

`nets.promote` and `nets.resolve_ambiguity` use the shared revision-checked
transactions in headless and live sessions. Both take `expected_revision` and
an `anchor`, using an existing terminal/label/port endpoint or a complete typed
anchor from `project.net_registry.nets`. Address `sheet` when multiple sheets
exist; an optional `board` must match the current owner. Resolution also takes
the ambiguous source UUID as `net` and assigns it to the selected current
candidate branch. Promotion of an already persistent network is a no-op.
Bare pins and unnamed one-pin stubs cannot be promoted. Inspection reports saved
IDs, anchors, lineage and decisions only when a registry exists, and never
creates records. See the [storage and command contract](net-registry-storage.md)
for scope and version details. There are no new editor controls in this slice.

`pcb.add_zone`, `pcb.set_zone` and `pcb.delete_zones` edit
[zone outlines](copper-zone-outlines.md#shared-commands) through the same revision-checked
transactions as PCB Layout. A derived typed net target is promoted in the same
undo step as creation; inspection reports authored zone records and current net
identity state. These commands do not create conductive fill.

`pcb.move_items` takes `board`, `expected_revision`, a mixed `items` selection
and integer `dx_nm`/`dy_nm` displacements. It translates the complete selection in
one transaction, preserving identities and carrying unselected track ends on moved
pads. Explicitly selected tracks translate once. Unknown/locked objects, stale
revisions, out-of-range geometry or a new physical short refuse the complete move.

`pcb.delete_items` also accepts zone UUIDs in `items.zones` and deletes mixed
board selections in one transaction. Copy, Cut and Duplicate refuse a selection
containing zones until zone clipboard support is available; they never omit the
zones and operate on a subset.

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
`layer` UUID from `project.boards[].layers`, optional `width_nm` (10,000 to 10,000,000) and
2–256 ordered `points`; each neighboring pair becomes one straight segment, all in
one undo step. Optional `layer_changes`, a list of `{at, layer}` in increasing
vertex order after the first, places a through via at `points[at]` and continues on
that copper layer; a change at the last vertex ends the route on a via. Omitted
width, via diameter and drill use the board routing preferences, including layer
changes; explicit drills must be smaller than their pads.

`pcb.unroute_connection` takes `board`, `expected_revision`, and `from` and `to`
addresses such as `{"footprint":"footprint-7","pad":"2"}`. Pick `from` on the
arm to remove and `to` on the retained side. `execute` returns `revision`,
`dry_run`, `removed` (sorted `tracks` and `vias` UUID lists), `requested` (the
input pair), and `unrouted` (new ratsnest edges from the change to authored
copper, as pairs of pad addresses), even with project inspection disabled. An
edge may end at a different pad on the retained side than the requested `to`.
Refilled zones can reconnect the pads and clear those edges. Without accepted
connectivity there are no edges to report. Set `dry_run: true` to get the same
report without changing copper, revision or undo. A stale revision returns `stale_revision` with `current_revision` in
`details`; a missing copper endpoint returns `not_found`. See
[Unroute one connection](pcb-layout-workflow.md#unroute-one-connection) for the
behavior and other refusal cases.

`pcb.check_design` checks the addressed board without editing it; `execute` returns
`design_check` with the `rules` used, `violations` (each with a `kind`, `message`,
representative `at` point and `items`), `counts` per kind and the `total`, listing at
most 1000 findings. Saved board defaults apply unless a request supplies a
supported hard-limit override; authored minimums can only tighten upward and
maximums downward. The report includes per-property `sources`. Kinds are `short`,
`clearance`, `track_width`, `via_diameter`, `via_drill`, `annular_ring`, `unconnected`,
`no_outline`, `ambiguous_outline`, `outside_board`, `edge_clearance` and `courtyard_overlap`.
Closed Board Outline drawings whose boundaries cross, touch or share an edge are grouped into one
union component and reported as `ambiguous_outline` (listing those drawings' UUIDs), while
disjoint closed outlines still nest by even-odd parity so a cut-out away from an overlap remains
a cut-out. An `outside_board` finding for a copper item or
non-plated hole inside a cut-out also lists the cut-out drawing's UUID after that item. A
`courtyard_overlap` lists both footprints and marks a point inside the overlap; touching
courtyards pass. When unsupported courtyard geometry (circles, arcs, open paths) decided
the finding by bounds, its message says the comparison is conservative.

`pcb.export_fabrication` generates the addressed board's fabrication files without
editing it; `execute` returns `fabrication.files`, each with a `name` and text `contents`,
and `fabrication.origin`, the placement origin every coordinate is measured from:
Gerber X2 copper per layer, mask, paste and silkscreen for both sides, and the outline,
plus one Excellon plated drill file. Boards with placed circular non-plated holes also get
a separate non-plated drill file. It needs a board setup, a closed Board Outline and a
resolvable placement origin. Boards with copper zones or non-plated holes whose
diameters round to zero in the drill file are refused before any output is returned.
File names are deterministic and unique after portable-name sanitization,
including case-insensitive collisions with fixed-purpose output names. `apply`
validates board and file generation, not ZIP construction; `execute` also returns
`fabrication.manifest` for the same captured files. Neither operation writes files
or changes the project or revision; `execute` does not build or return a ZIP.

The manifest matches the `manifest.json` embedded in the fabrication ZIP for the
same capture: `generator_version` and ordered `files` records with `name`,
`size_bytes` and lowercase hexadecimal `sha256` of the uncompressed UTF-8 bytes.
It lists only the generated files, not itself. Clients that used
`fabrication.manifest.archive` must adapt: the outer ZIP name, size and digest are
no longer reported. Member records are independent of the ZIP filename. They
support byte checks, not source identity, configuration or check freshness. See
[desktop/browser publication limits](editor-guide.md).

`project.export_netlist` generates the project's netlist without editing it;
`execute` returns `netlist.files`, one `<project name>-netlist.json` (the project name made portable as for the
fabrication files) with its `name`
and text `contents`, and `netlist.findings` for unplaced units and nets without a
persistent ID. `dry_run: true` returns only the file name and the findings. The
document lists every component with its logical terminals and every net across all
sheets with its persistent UUID and joined terminals; see the
[netlist format](netlist-format.md). Exporting an unchanged project gives the same bytes.

`pcb.export_netlist` takes `board` and `dry_run` and returns the same shape for
one board: `<project name>-<board name>-netlist.json`, format `pcbcraft-netlist/2` with `basis: accepted`, the
components, packages, terminal pads and nets the board accepted at its last
reviewed update. Findings report nets and components the schematic has changed
since (`pending_net`, `pending_removal`) and a `pending_update` summary matching
`pcb.preview_update` when comparison is available, or `update_unavailable` otherwise.
It fails for a board with footprints but no accepted baseline. `dry_run: true`
returns the filename and findings without rendering the file text. If the full
reply exceeds the live editor's 16 MiB frame limit, save the project and use the
headless command for the full export.

`project.compare_netlist` takes a netlist file's text as `netlist` and compares
it with what the open project exports now, the project netlist or, for a board
file, that board's accepted netlist, without editing anything. `execute` returns
`comparison`: `matches`, `identical`, the counts on both sides, `differences`
such as a terminal on another net (named with its net in the design and the
file's net it is on), and `notes` for renamed nets and other changes that leave connectivity
alone. Terminals are matched by part and terminal, never by net name or
position; see [comparing a file with the design](netlist-format.md#comparing-a-file-with-the-design).

`pcb.set_copper_locked` requires an explicit `board` UUID, `expected_revision`,
`locked` boolean and at least one of `tracks`/`vias` (unique UUID arrays). It locks
or unlocks the whole selection in one undo step, with no geometry or connectivity
change. `project.boards[].tracks[]` and `.vias[]` always report `locked`, including
false; the typed board snapshot exposes the same flag. Geometry, size and deletion
commands refuse the whole edit when they would modify existing locked copper,
including indirect footprint/segment transforms. No-op operations remain no-ops.
Copy/paste preserve locks on new identities; Cut of protected copper is refused.

`pcb.add_via` places one standalone through via from `at`, with optional `via_diameter_nm` and
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

`pcb.plan_route` plans a route without editing: `from`, `to` and a copper `layer`, with
optional `width_nm`, `posture` (`diagonal` or `orthogonal`), `flipped`, `previous`,
`budget` and `expected_revision`. `execute` returns `route_plan` with the resolved
width, `points`, an `outcome` (`reached`, `blocked` with `last_legal`, `incomplete`,
`canceled` or `stale`), any `blocker` and the `probes` charged, also with
`inspect: false`. The points go round existing copper with the effective width,
pair clearances and board-edge clearance. Check that there are at least two points
before passing them to `pcb.add_tracks` with the same `layer` and `width_nm`;
`from` equal to `to` produces a one-point `reached` plan with nothing to add.
See [walkaround route planning](route-planning.md).

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
`view.set_net_color` takes `view`, an active persistent `net` UUID and `color`
(`#RRGGBB`, or null to use the layer palette). The shared personal color applies
across layers. `view.set_net_colors_visibility` switches the addressed view
between net and layer colors. `inspect_editor` exposes the saved overrides in
`board.net_colors` and the display setting in `board.net_colors_visible`.
Effective net and layer color changes enter the live editor’s Undo and Redo
sequence. They persist locally and leave the project revision and files
unchanged. Physical pad inspection also exposes `pins[].copper_layers`: `resolved` includes the
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
`--project path/to/project.json` (or a folder or `.pc`) after `mcp` to start with a saved snapshot. The
headless session is separate from the app window. Send `inspect: false` with
`execute` to omit inspection and return the revision for ordinary edits.
Commands that return their own data, such as exports, clipboard queries and
[schematic annotation preview](annotation.md), retain that data. In particular,
`schematic.set_symbol_fields` with `dry_run: true` still returns the full preview,
including the changed native documents. Use `inspect_project` to read
the project, or `cross_probe` to look up a linked terminal or pad. The additive
read-only `resolve_counterparts` tool returns complete pin/pad sets and explicit
unavailable states at a captured revision; see [pin/pad correspondence](pin-pad-correspondence.md)
for qualified addresses and the versioned response. `cross_probe` retains its
singular compatibility response.

To give a reviewer only command-level read access, start a separate headless process:

```sh
pcbcraft-cli mcp --read-only --project path/to/project.pc
```

The host sets this scope at startup; no MCP request can widen it. `inspect_project`
reports `scope: "read_only"`. `describe_command` and `list_commands` report it
in `structuredContent` and the second text part. The first text part keeps the
unchanged command descriptor or catalog list.
Queries, data exports and transient previews remain available; project edits,
Undo/Redo, Cut and library-source changes return `out_of_scope` without changing
the snapshot or the file. Startup does not recover an interrupted folder save;
if recovery is needed, have the owner open the project normally first. A command
added to the catalog must be explicitly classified as a read before a read-only
session can run it.

This setting confines the command interface of that process, not the client's
filesystem permissions. It does not grant read-only access to a live editor:
`--read-only` cannot be combined with `--connect`.

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

## Native part definitions and quantities

`library.search` accepts `kind: "part"` or `"category"` alongside symbols and
footprints. `library.resolve_part` takes an exact `library`, `definition` and
`definition_revision`, validates category values and every package map, and
returns native dependency documents. It never fetches or substitutes definitions.
`library.parse_quantity` takes a declared `quantity` and input `value`; for
example, resistance `100 mΩ` produces
`{"type":"quantity","value":"0.1","unit":"Ohm"}`. Both operations are read-only
and work without an open project. [The native contracts](part-schema.md)
distinguish part definitions from placed part instances. `schematic.place_part`
pins the full native dependency set; `part.set_selection` edits package, MPN and
typed requirements through the same revision-checked command path as the GUI.

`schematic.place_symbol` accepts an optional `unit` for a new part, while
`schematic.place_unit` adds an unplaced unit to an existing `component-…` part
using its project-pinned definition. Supply `component`, `unit`, `x_nm`, `y_nm`,
`expected_revision` and `sheet` when the project contains multiple sheets.
Schematic inspection reports `available_units` and `display_reference`; the
base `reference` stays shared by the part. The library authoring transaction
accepts `select_unit` and `add_unit` actions. See the
[multi-unit contract](symbol-schema.md#multiple-schematic-units).

`schematic.complete_packages` requires `expected_revision`. Set `scope` to
`"sheet"` (include the addressed `sheet` UUID when several sheets exist) or
`"project"` to opt in; omission or `"none"` does nothing. Set `dry_run: true` to
validate the same plan without editing. Both modes return `{revision, changes,
skipped}`, even with `inspect: false`. Each change names `sheet`, `symbol`,
`from_part`, `from_unit`, `to_part`, `to_unit` and `remapped_terminals` (changed
pin numbers as an old-to-new map). A source part in `changes` is retired; its
symbol is retained. Skipped entries name `part` and `reason`. Authored
no-connect markers or selected alternate pin functions report `terminal_intent`.
A read-only session refuses the command with `out_of_scope`, even for a dry run, because the
same command applies the transfer. In the live editor, Preview and Apply prepare
the plan off the editor thread, so the canvas stays responsive; just before
publishing, the editor re-checks the revision, open dialogs, the request deadline
and the connection. If the report would exceed the 16 MiB live reply limit,
Preview and Apply both return `cap_exceeded` and nothing is edited; use `"sheet"`
scope or headless MCP for very large projects. For a live Apply, the editor
checks its request deadline after preparing the transfer and does not publish if
the deadline or an observed disconnect wins. Once publication is authorized, that server
deadline no longer ends the reply wait. A client timeout or transport failure can
still lose the reply to a committed edit; inspect before retrying. See [package
completion](symbol-schema.md#complete-loose-packages) for the matching and
electrical restrictions.

## Placement origin

`pcb.set_placement_origin` requires an explicit `board`, `expected_revision` and
`origin`: `{"kind":"board_zero"}`, `{"kind":"outline_lower_left"}` (the lower-left
corner of the Board Outline's bounds, following later outline edits), or
`{"kind":"point","x_nm":...,"y_nm":...}` in board coordinates. `null` removes the saved
origin. It is one undo step, keeps the origin's UUID while its anchor changes, and
ignores a repeat of the current value. Board inspection reports `placement_origin`
with the `authored` record and the `effective` origin outputs use, which is null when
an outline-attached origin has no outline yet.

Fabrication outputs shift every coordinate to this origin and write Y up, as seen
from the top; rotations are unchanged. The export result's `fabrication.origin` names
the `anchor`, its `source` (`{"kind":"builtin"}` without a saved origin, otherwise
`{"kind":"board","origin":"<uuid>"}`), the resolved `at` point in board coordinates, and `axes: "x_right_y_up"`. Without a saved origin, outputs
are measured from board zero. Gerber writes exact nanometres; Excellon rounds each
millimetre value to three decimals from integer nanometres with halves going up, so
moving the origin by whole micrometres shifts every written value by exactly that amount.

## Assembly output profiles

`pcb.set_assembly_profile` pins immutable project-owned content. It requires
`expected_revision` and `content`. Omit `profile` to create a UUID, or supply an
existing or portable profile UUID. Content has a positive 32-bit `revision`, a
`name`, and `provenance: {"kind":"authored"}` or
`{"kind":"imported","source":"<human source hint>"}`. The optional `vendor` is
metadata, never a behaviour selector. Names, source hints and vendor metadata
have 1–64 characters without surrounding whitespace or controls; available
profiles have unique names. `bottom_side` defaults to `viewed_from_top`, or is
`viewed_from_bottom`. The schema version (`pcbcraft-assembly-profile/1`) is
independent of content revisions. The command currently requires a project with a
board; it is unavailable in schematic-only projects even though the catalog is
project-owned.

Optional `corrections` has one entry per `library` and footprint `definition`,
with a `rotation` (0–359 whole degrees, counterclockwise from the component side)
and/or a nonzero `centroid` (`x_nm`, `y_nm` in library footprint coordinates, Y
down, each within 100 mm). Each entry must change something. Commands sort entries
and omit zero rotation; an empty array is refused. Integer-valued numbers already
parsed by serde_json are accepted without further rounding. Digits beyond what
the JSON parser keeps are not preserved. Corrections apply across definition revisions; legacy
markers never match. See [placement conventions](footprint-schema.md#placement-output-conventions).

An existing ID and content revision is a no-op only for identical content;
different content is a conflict. New content must exceed that identity's revision
watermark. `content: null` retires the named identity without changing existing
adoptions. Reusing an already cleaned revision is refused, even with identical
content; choose a greater revision instead. Undo/Redo also retain issued revision watermarks. An undone pin whose payload is no longer present cannot reuse that revision, even after Save/Open; a higher revision is required. This reservation is local to the saved project lineage, not a global registry across independently edited copies.

`pcb.preview_assembly_profile` requires `board`, `expected_revision`, `profile`
and `revision` (a pinned content revision, or null to clear that board's adoption).
It returns the old/new adoption, changed corrections, bottom convention and actual
affected placement rows before and after, plus an `acceptance` digest. It edits
nothing. `pcb.adopt_assembly_profile` accepts the same parameters and that digest,
recomputing the preview atomically. A stale revision or altered review is refused.
Pinning is not adoption; only explicit adoption changes which profile revision the
board's rows use.
Each edit is one undo step and repeated unchanged adoptions are no-ops. Pinning on
a supported older project adopts the folder, occurrence and allocation metadata
required by the catalog's saved format in that same undo step; existing project,
board and object identities and registered storage roots are preserved. Undo restores profile choices but keeps revision reservations and the storage metadata needed to save them.

Inspection exposes project `assembly_profiles` (identity watermarks and pinned
revisions) and board `assembly_profiles` (explicit revision adoptions), sorted by
identity. Limits are 64 identities including retired and watermark-only ones, 128 retained revisions
and 4096 corrections across retained revisions. Cleanup on edits releases only
unadopted superseded or retired payloads; adopted revisions survive retirement,
offline save/reopen and Undo/Redo. Undo shares immutable payloads. Profiles change
no board geometry or fabrication output. No placement exporter, import/export
wrapper or profile-management UI is provided here.

## Saved board defaults

`pcb.set_rules` requires an explicit `board`, `expected_revision` and complete
`rules` values, or `rules: null` to restore built-in defaults. Board inspection
reports authored records and effective values with stable rule IDs and per-field
provenance. DRC resolves saved limits; request overrides can tighten authored
minimums/maximums but cannot weaken them or persist a change. It also reports via
diameter and drill bounds. Omitted dimensions on track/via creation use the same
preferred sizes as the GUI. See [board-rules.md](board-rules.md) for the full
property, precedence, compatibility and undo contract.

## Intentional no-connect targets

`schematic.set_no_connect` uses `target: {"part":"component-N","terminal":"1"}`
for a logical pin, or `target: {"junction":"junction-N"}` for a physical open
wire end, plus `marked` and `expected_revision`. Both forms use the same edits
as Q and the Capture tool. `schematic.check_no_connects` returns a read-only,
revision-qualified report even with `inspect:false`; its `markers`/`findings`
arrays describe pins and `wire_markers`/`wire_findings` describe wire ends.
Totals and truncation cover both. These markers record intent without changing
the real netlist or physical DRC. See the [contract](no-connect-markers.md).

Schematic selections may include an optional `no_connects` array of the same
pin/wire target objects. `view.select` exposes marker selection and
`view.set_selection_filters` accepts `no_connects`. Passing these targets to
`schematic.delete_items` removes only the selected markers, or combines them
with ordinary owner deletion in one undoable transaction. Target validation is
sheet-scoped and revision-checked. Copy/group-move selections must include each selected marker's owner; standalone
marker geometry is not persisted. A direct wire-end X drag uses the existing
`schematic.move_junction` command and preserves marker identity.

A placed no-connect mark blocks new wire attachments at that point. Shared
wire drawing, pin connection and endpoint attachment commands reject the edit
atomically with “Remove the no-connect marker before connecting a wire.” This
also covers coordinate and wire/at aliases. Remove the mark through the same
commands or Delete before connecting; saved conflicts from older projects are
preserved and remain reportable.


## Explicit net-class rules

`net_classes.create`, `net_classes.update` and `net_classes.remove` author the
project catalog with `expected_revision`, without creating a physical board.
`net_classes.inspect` reads that catalog, while `net_classes.resolve` combines
explicit `classes` and optional `other_classes` UUID sets with built-ins or an
explicitly addressed `board`. Both queries return their data with `inspect: false`.
See the [property, conflict and storage contract](net-classes.md) for examples
and precedence. `nets.inspect_classes` discovers current typed targets without
promotion; with a target it returns bindings, origins and conflicts.
`nets.set_classes` replaces the full class set with revision checking and promotes
an eligible derived target in the same history step. The assignment query retains
its payload with `inspect: false`. See the [assignment contract](net-class-assignments.md)
for target shapes and protected removal/rebind. `pcb.route_rules` inspects effective
sizes, clearance and conflicts at an addressed copper contact without mutation,
including with `inspect: false`. Omitted routing sizes use that contact's class
preferences; explicit changed copper must satisfy applicable hard dimensions.
Queries and DRC retain per-object class IDs, revisions and assignment origins.
See the [consumer contract](net-class-consumers.md) for parameters, provenance,
stale authority and operation-specific conflict behavior.

Standalone via net intent uses `pcb.add_via` with optional `target`, or
`pcb.set_via_net` with `vias` and either `target` or `automatic: true`. Both share
revision-checked GUI transactions. See [via net assignments](via-net-assignments.md)
for target qualification, inspection and unresolved-state behavior.
