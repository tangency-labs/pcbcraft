# Schematic keyboard shortcuts

This page inventories the schematic editor's current keyboard bindings and the
application-level shortcuts that apply while a schematic view is focused. It is a
record of observed behavior, not a policy: each entry describes what the key
does today and the conditions that decide whether it fires. The in-app
**Preferences → Shortcuts** page shows the same bindings formatted for the host
platform.

**Verification.** Checked against the desktop build on Linux (X11) in the
schematic workspace: each placement key below was pressed in a live session and
the resulting tool, dialog or status-bar hint was observed, and M/G/X/R/E/Q/
Delete/Undo/unit-switching were exercised on a placed symbol. The in-app
Shortcuts page reads "The keys PCBCraft answers to. They can't be changed yet."
macOS uses **⌘** wherever **Ctrl** is shown below; the browser build omits the
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
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste | Canvas owns input; uses the OS clipboard, so fields keep normal editing |
| Ctrl+Shift+V | Paste with options | Reference, named-net and original-position choices |
| Ctrl+D | Duplicate | Duplicates the selection without replacing the clipboard |
| N / Shift+N | Finer / coarser grid | Applies to the focused pane's snap spacing |
| Ctrl+U | Switch units | Toggles mil and mm; presentation only, no undo step |
| Esc | Back from Home | On the Home screen only; see cancellation order above |

Ctrl+C/X/V and Ctrl+D are inactive on the Home screen, while a dialog or popup
is open, while the parts panel is modal, and while a definition editor or text
field owns input.

## Schematic placement keys

Each of these starts a tool when the schematic canvas owns keyboard focus and
no placement tool is already running, except where noted.

| Key | Action | Conditions |
| --- | --- | --- |
| A | Add symbol | Focuses the parts search; the catalog offers symbols to place |
| P | Place power/ground | Focuses the power-symbol section of the parts panel — a different target from A |
| W | Draw wire | Starts the wire tool at the pointer; click to place vertices |
| L | Place local label | Opens the net-label form for a wire-local name |
| Ctrl+L | Place global label | Same form producing a project-wide net name |
| H | Place hierarchical label | Names a net exported to the parent sheet |
| S | Place hierarchical sheet | Draws a subsheet symbol on the canvas |

## Schematic editing keys

These are the context-sensitive keys. When several rows share a key, the first
matching condition wins: an active tool keeps its own meaning, then a hovered or
selected net label or sheet symbol takes the key, then the generic symbol or
selection action.

| Key | Action | Conditions |
| --- | --- | --- |
| M | Move, disconnecting | With several items or a single wire selected: picks the group up, leaving attached wiring in place — the pointer must be on the canvas to provide the move origin. With a symbol selected or hovered (selection of at most one): picks that symbol up, leaving its wiring; a selected symbol moves regardless of pointer position. No active tool. On a net label or sheet symbol: moves that item. Compare **G**. |
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
| Enter | Finish wire | Commits the drawn wire |
| Ctrl (hold) | Bypass grid | While the wire tool is active, holding Ctrl lets vertices land off-grid |
| Esc / right-click | Cancel tool | Right-click cancellation applies to wire, no-connect, move and net-label tools |
| Double-click | Enter sheet | On a sheet symbol, descends into the subsheet; on a symbol or field, opens its edit form |

## Notes for contributors

- The authoritative routing lives in `PcbApp::shortcuts` and `shortcut_list` in
  `crates/app/src/lib.rs`; pointer-scoped schematic keys are consumed in
  `crates/app/src/schematic_canvas.rs` (`Q`, `G`), `net_labels.rs` and
  `hierarchy.rs` (label- and sheet-directed `E`, `M`, `R`, `Delete`).
- The in-app Shortcuts page lists the subset of bindings that are stable for
  users. Some live keys are not listed there — X/Y mirroring, Alt+Backspace,
  the wire-drafting keys and the clipboard chords — so this page is the fuller
  inventory.
- Key reuse across workspaces is intentional: **X** routes a track in the board
  editor but mirrors a symbol in the schematic, and **Ctrl+A** selects all in
  the board editor while the schematic offers Select All as a tool action
  instead.
