# UI automation

The native live MCP connection can locate named controls and send pointer,
keyboard and text input through the editor's ordinary egui event handling. This
supports repeatable demonstrations and interaction tests. It does not move the
operating system's pointer or record video. Headless sessions and the browser
build do not expose this native input endpoint.

Input currently targets the original native window (`window: 0`), including its
split panes. Named lookup is window-scoped and also works in detached editor
windows, but input to them is explicitly refused: the pinned eframe immediate
viewport path does not call the application's raw-input hook. Supporting it
requires a separate framework input boundary rather than injecting events after
egui has already processed them.

Enable the connection through **Help → Connect AI assistants…**, as described in
[the MCP guide](mcp.md#live-mcp-in-the-desktop-editor). Use `inspect_editor` to
find the intended window ID, then `list_commands` with `filter: "ui."` to discover
the schemas. These commands run through the existing MCP `execute` tool and
require the existing local connection token. Geometry is collected only after a
UI query opts in; if the first call reports that geometry is not ready, wait for
the window to paint and retry the query.

## Locate a control

```json
{"name":"execute","arguments":{"command":"ui.handles","params":{"window":0,"prefix":"toolbar:Connect:"}}}
```

The response contains this window's visible named targets, their enabled state,
center coordinates and clipped rectangles. Responses identify the contract as
`schema: "pcbcraft-ui/1"`. The `layout` value changes when the
published geometry or input destination changes. An optional matching `layout`
on an input call rejects coordinates obtained from an older layout.

Names currently cover ribbon actions (`toolbar:Connect:Wire`), ribbon tabs
(`tab:View`), menu triggers/rows (`menu:File`), common button and checkbox adapters
(`button:OK`, `checkbox:…`), named search/property fields (`field:…`) and the
uncovered region of each project canvas (`canvas:VIEW_ID`). Names are explicit
host aliases, not image recognition or persisted electronics identities. Some
specialized controls do not yet publish names; use the returned list rather than
assuming every visible widget has one. Closed popups and scrolled-out controls
are absent. Fully covered targets are excluded, and disabled targets remain
inspectable but cannot be selected by name.

`ui.at` takes `window` and `handle`, returning the enabled target's `x`, `y` and
`layout`. Missing or duplicate names produce errors. Identical names in separate
windows are independent; duplicate names within one window are ambiguous.

## Perform input

```json
{"name":"execute","arguments":{"command":"ui.click","params":{"window":0,"target":{"handle":"toolbar:Connect:Wire"}}}}
```

Input returns a ticket and `state: "queued"`. This acknowledges acceptance, not
completion. Poll `ui.inspect` with empty parameters until `operation.ticket`
matches and `operation.state` becomes `completed` or `cancelled`. A cancelled
operation includes its reason. Only the pending operation or latest completion
is retained. Wait for it before submitting the next input or project/window
command; inspection remains available while input is pending.

Completion means the editor accepted the UI pass; it does not acknowledge GPU
submission or compositor presentation. A recording should allow its intended
visual/caption hold before capturing the next milestone.

| Command | Parameters in addition to `window` and optional `layout` |
| --- | --- |
| `ui.move` | `target` |
| `ui.click` | `target`, optional `button` and `modifiers` |
| `ui.drag` | `from`, `to`, `steps` (2–120), optional `button` and `modifiers` |
| `ui.key` | `key`, optional `modifiers` |
| `ui.text` | `text` (1–4096 UTF-8 bytes without control characters) |
| `ui.scroll` | `target`, `delta_x`, `delta_y`, optional `modifiers` |

A target is either `{"handle":"NAME"}` or `{"x":400,"y":300}`. Coordinates
and wheel deltas use window-local logical points, independent of desktop origin
and physical capture resolution. Points must lie inside the addressed window;
wheel deltas are bounded to ±10,000 points per axis. Buttons are `left`, `right`
or `middle`. Modifier flags are `ctrl`, `shift`, `alt` and `command`; `command`
means the platform shortcut modifier. Key names include `W`, `Escape`, `Enter`,
`Tab` and `PageDown`. Use `ui.key` for Enter/Tab rather than embedding them in text.

Pointer events, including intermediate drag positions, run on successive
accepted UI passes. Click/key press and release pairs stay together so a slow
renderer cannot turn them into a held input. The normal controls, tools,
validation, command history and modal behavior still apply. A click that edits
the project uses the same command and Undo as a person's click; it is not an
engine mutation disguised by a cursor animation.

The addressed window must be focused. Physical input, loss of focus/visibility,
resizing/scaling, a changed project/input destination, a file operation, endpoint
revocation or the five-second deadline cancels remaining events. Assistant jobs
also belong to their originating reply and stop when that reply is stopped or
replaced. Named destinations are checked again before button events. `ui.cancel`
takes empty parameters and cancels explicitly. Cancellation releases owned input
without clicking a control and cancels unfinished editor gestures; completed
edits remain in normal history. An acknowledgement followed by a lost reply is
not grounds to repeat an input call: inspect its operation and project state.

UI input can reach save/close controls as well as undoable edits. In the
Assistant's **Auto-allow project edits** mode, input commands therefore ask for
approval; UI inspection and cancellation remain available in every mode. This
uses the existing permission policy, not a separate automation permission switch.

Synthetic passes disable the Assistant conversation controls and AI settings.
Approval answers, permission changes and agent configuration require human input,
including when a permission popup was already open. The host tracks input
provenance per viewport, so coordinate clicks and keyboard activation follow the
same consent rule as named clicks. Detached Assistant controls are not collected
into an editor window's lookup.
