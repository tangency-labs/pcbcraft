# Schematic keyboard shortcuts

This page inventories the schematic editor's current keyboard bindings and the
application-level shortcuts that apply while a schematic view is focused. It is a
record of observed behavior, not a policy: each entry describes what the key
does today and the conditions that decide whether it fires. The in-app
**Preferences → Shortcuts** page shows a subset of these bindings formatted for
the host platform; this page is the fuller inventory.

**Verification.** Checked against the desktop build on Linux (X11) in the
schematic workspace: each placement key below was pressed in a live session and
the resulting tool, dialog or status-bar hint was observed, and M/G/X/R/E/Q/
Delete/Undo/unit-switching were exercised on a placed symbol. The in-app
Shortcuts page reads "The keys PCBCraft answers to. They can't be changed yet."
For command chords, macOS uses **⌘** where **Ctrl** is shown below. Wire grid
bypass still uses physical **Ctrl**, as noted in its row. The browser build omits the
desktop-only project keys noted in the table and has not been separately
verified for this inventory.

## Reading the conditions

Keyboard input reaches a canvas key binding only when all of these hold:

- The view receiving the event is the focused view and no dialog, popup or modal
  parts panel is open.
- No text field owns input (for example a label form, a properties field or the
  parts search box — those take the keys for typing).
- The Assistant panel does not have focus; assistant focus suppresses canvas
  tools.
- For pointer-scoped bindings (**Q**, **G**, and hover-directed uses of
  **M**, **R**, **E**, **X**, **Y**, **D**, **Delete**), the pointer must be over
  the canvas, and the binding acts on what is under the pointer or on the
  current selection.

**Esc** is layered rather than a single cancel. In order, it dismisses any open
popup or card drag, closes the parts chooser or releases its search focus,
cancels the active tool, and finally clears the selection. Pressing it
repeatedly walks down that list. On the **Home** screen it returns to the open
project.

## Application and edit keys

These are processed in both workspaces; the schematic-relevant ones are listed
here.

| Key | Action | Conditions |
| --- | --- | --- |
| Ctrl+N | New project | Desktop only; a save prompt appears first when there are unsaved changes |
| Ctrl+O | Open project | Desktop only; same save prompt |
| Ctrl+S | Save project | Desktop only, with a project open |
| Ctrl+Shift+S | Save project as | Desktop only, with a project open; opens the encoding dialog |
| Ctrl+, | Preferences | Opens the preferences window |
| Ctrl+Q | Quit | Desktop only; asks about unsaved changes |
| Ctrl+Z | Undo | When the undo history has an entry |
| Ctrl+Y | Redo | When the redo history has an entry |
| Ctrl+A | Select all | Idle focused canvas; selects supported items on the current sheet and respects Selection filters |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste | Canvas owns input; uses the OS clipboard, so fields keep normal editing |
| Ctrl+Shift+V | Paste with options | Reference, named-net and original-position choices |
| Ctrl+D | Duplicate | Duplicates the selection without replacing the clipboard |
| N / Shift+N | Finer / coarser grid | Applies to the focused pane's snap spacing |
| Ctrl+U | Switch units | Toggles mil and mm; presentation only, no undo step |
| Esc | Back from Home | On the Home screen only; see cancellation order above |

Ctrl+A, Ctrl+C/X/V and Ctrl+D are inactive on the Home screen, while a dialog or popup
is open, while the parts panel is modal, and while a definition editor or text
field owns input.

## Schematic placement keys

These keys are not gated on the canvas being idle: pressing one while a tool is
active replaces that tool (or redirects focus) rather than doing nothing. The
shared focus gates above — no dialog or popup, no text field, no assistant
focus — still apply.

| Key | Action | Conditions |
| --- | --- | --- |
| A | Add symbol | Focuses the parts search (`parts.focus_search`); the catalog offers symbols to place |
| P | Place power/ground | Focuses the power-symbol collection of the parts panel (`parts.focus_power`) — a different target from A |
| W | Draw wire | `begin_wire` resets the draft and replaces any active tool; click to place vertices |
| L | Place local label | `begin_net_label` clears the selection and tool, then opens the net-label form for a wire-local name |
| Ctrl+L | Place global label | Same path producing a project-wide net name |
| H | Place hierarchical label | `begin_hierarchy` replaces the tool with label placement; names a net exported to the parent sheet |
| S | Place hierarchical sheet | Same path; draws a subsheet symbol on the canvas |

## Schematic editing keys

These are the context-sensitive keys. When several rows share a key, the first
matching condition wins: an active tool keeps its own meaning, then a hovered or
selected net label or sheet symbol takes the key, then the generic symbol or
selection action.

| Key | Action | Conditions |
| --- | --- | --- |
| M | Move, disconnecting | With several items or a single wire selected: picks the group up, leaving attached wiring in place — the pointer must be on the canvas to provide the move origin. With a symbol selected or hovered (selection of at most one): picks that symbol up, leaving its wiring; a selected symbol moves regardless of pointer position and takes priority over a different hovered symbol. No active tool. On a net label or sheet symbol: moves that item. Compare **G**. |
| G | Grab, keeping connections | Picks up a multi-item selection, a hovered wire reshape handle, or a hovered symbol while retaining external wiring; click to place. Pointer must be over the canvas; no active tool (or a wire tool that has not started). |
| R | Rotate | While a placement or move tool is active: rotates the preview 90°. While placing a net label: rotates the label. Hovering a symbol field: rotates the field about its anchor. Hovering a net label: rotates the label. Otherwise rotates the hovered or selected symbol 90° counterclockwise. |
| X | Mirror horizontally | Mirrors the hovered or selected symbol across its center; wiring stays attached. No active tool. |
| Y | Mirror vertically | Same as X, on the other axis. Note this is mirror, not the board's rotate-clockwise **Shift+R**. |
| E | Edit | Hovering a visible symbol field: opens inline text editing. Hovering or selecting a symbol: opens its fields. On a net label: opens its properties. On a sheet symbol: edits the sheet. |
| D | Datasheet | Opens the component datasheet for the hovered or selected symbol. |
| Q | No-connect | Pointer over a pin lead or open wire end: marks it no-connect. Elsewhere on the canvas: starts the no-connect tool, which then marks the next clicked target. No active tool; pointer on the canvas. |
| Delete / Backspace | Delete | Removes the selection; on a hovered or selected net label or sheet symbol removes that item instead. With the wire tool active, Backspace removes the last placed vertex (see below). |
| Esc | Cancel | Cancels the active tool first; with no tool, clears the selection. Panels and popups get it first (order above). |
| Alt+Backspace | Leave sheet | Returns from a subsheet to its parent. |

## Keys while a schematic tool is running

| Key | Action | Conditions |
| --- | --- | --- |
| Space | Flip wire bend | While drawing a wire, swaps horizontal-first and vertical-first routing |
| Backspace | Remove last vertex | While drawing a wire; with no vertices, abandons the start point |
| Enter | Finish wire | Commits the wire at the pointer's target. The pointer must be inside the canvas resolving to a wire anchor — Enter alone, with no in-canvas pointer, does nothing |
| Ctrl (hold) | Bypass grid | While the wire tool is active, holding the physical **Ctrl** key lets vertices land off-grid. On macOS this is still Ctrl, not ⌘ — this row is an exception to the modifier note above |
| Esc / right-click | Cancel tool | Right-click cancellation applies to wire, no-connect, move and net-label tools |
| Double-click | Enter sheet | On a sheet symbol, descends into the subsheet; on a symbol or field, opens its edit form |

## Where each binding is handled

Every row above is consumed in `PcbApp::shortcuts` (`crates/app/src/lib.rs`) or
in the canvas module named here. Focused tests that exercise a binding are
listed beside it.

| Rows | Consumed by | Focused tests |
| --- | --- | --- |
| Project keys (Ctrl+N/O/S/Shift+S/,,/Q) | `PcbApp::shortcuts` → `new_project`, `open_project_file`, `save_project_file`, `open_preferences`, `request_project_close` | — |
| Undo/Redo (Ctrl+Z/Y) | `PcbApp::shortcuts` → `session.available` + `run` on `edit.undo`/`edit.redo` | — |
| Clipboard (Ctrl+C/X/V/Shift+V, Ctrl+D) | `clipboard_shortcuts` → `copy_clipboard`, `request_clipboard`, `duplicate_clipboard` in [clipboard.rs](../crates/app/src/clipboard.rs) | [clipboard/tests.rs](../crates/app/src/clipboard/tests.rs): `copy_between_independent_instances_uses_system_clipboard_events` and `paste_options_and_duplicate_share_the_validated_path` cover OS events and handlers/options, rather than physical key chords |
| Grid (N, Shift+N) | `PcbApp::shortcuts` → `cycle_grid` | `tests/grid_spacing.rs` (`grid_keys_step_through_the_presets`) |
| Units (Ctrl+U) | `PcbApp::shortcuts` → `preferences.units.toggled` | `tests/display_units.rs` |
| A, P | `PcbApp::shortcuts` → `parts.focus_search` / `parts.focus_power` (`parts_browser.rs`) | — |
| W | `PcbApp::shortcuts` → `begin_wire` (replaces the tool) | `tests/tool_modes.rs` (`wire_mode_stays_armed_for_independent_routes_until_cancelled`) |
| L, Ctrl+L | `PcbApp::shortcuts` → `begin_net_label` (`net_labels.rs`) | `tests/net_labels.rs` |
| H, S | `PcbApp::shortcuts` → `begin_hierarchy` (`hierarchy.rs`) | `tests/hierarchy.rs` |
| M | `PcbApp::shortcuts` → `schematic.move_requested`, applied in the canvas block as `Click::MoveItems`/`SymbolAction::DisconnectMove`; labels and sheets route through `net_labels.rs`/`hierarchy.rs` `Action::Move` | `tests/selection_move.rs` (`group_m_…`, `selected_wire_m_…`) |
| G | `schematic_canvas.rs` → `Click::MoveItems`/`GrabReshape`/`grab_symbol` | `tests/grab.rs` |
| R | `PcbApp::shortcuts` → `rotate_requested` or the active tool's turns; `net_labels.rs` rotates a placing/hovered label; `Click::RotateField` rotates a field | `tests/grab.rs` (pickup rotation), `tests/net_labels.rs` |
| X, Y | `PcbApp::shortcuts` → `mirror_requested` → `SymbolAction::Mirror` | `tests/mirror.rs` (`x_and_y_mirror_hovered_symbols…`) |
| E | `PcbApp::shortcuts` → `edit_requested`; labels/sheets route through `net_labels.rs`/`hierarchy.rs` `Action::Properties`/`Edit` | `tests/net_labels.rs`, `tests/hierarchy.rs` |
| D | `PcbApp::shortcuts` → `datasheet_requested` → `open_datasheet` | — |
| Q | `schematic_canvas.rs` → `Click::BeginNoConnect`/`NoConnect` | `tests/no_connect.rs` |
| Delete/Backspace | `PcbApp::shortcuts` → `delete_selection`; labels/sheets route through `net_labels.rs`/`hierarchy.rs` `Action::Delete` | — |
| Esc layers | `workspace_panels.rs` `take_panel_escape` → `parts_browser.rs` `take_escape` → `PcbApp::shortcuts` tool cancel → `select` clear | `tests/selection_move.rs` (`group_m_cancels_on_escape_…`) |
| Alt+Backspace | `PcbApp::shortcuts` → `leave_sheet` (`hierarchy.rs`) | `tests/hierarchy.rs` |
| Wire Space/Backspace/Enter | `PcbApp::shortcuts` toggles `wire_draft.horizontal_first` for Space, calls `Draft::back` for Backspace, and sets `finish_requested` for Enter; [schematic_canvas/wires.rs](../crates/app/src/schematic_canvas/wires.rs) `pick` consumes the finish request using the pointer's in-canvas target | `tests/tool_modes.rs` (Enter finish, right-click cancel) |
| Ctrl grid bypass | `schematic_canvas/wires.rs` `preview` reads `input.modifiers.ctrl` | — |
| Double-click | `schematic_canvas.rs` (`EditField`/`Edit`), `hierarchy.rs` `Action::Enter` | `tests/hierarchy.rs` |

## Notes for contributors

- The in-app Shortcuts page lists the subset of bindings that are stable for
  users. Some live keys are not listed there — X/Y mirroring, Alt+Backspace,
  the wire-drafting keys and the clipboard chords — so this page is the fuller
  inventory.
- Key reuse across workspaces is intentional: **X** routes a track in the board
  editor but mirrors a symbol in the schematic, and **Ctrl+A** selects all in
  the board editor while the schematic offers Select All as a tool action
  instead. The [board keyboard shortcuts](board-keyboard-shortcuts.md) page
  inventories that workspace's bindings, and the
  [library editor keyboard shortcuts](library-editor-keyboard-shortcuts.md)
  page covers the Symbol Editor, Footprint Editor and Parts Library.
