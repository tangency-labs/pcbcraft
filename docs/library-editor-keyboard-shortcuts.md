# Library editor keyboard shortcuts

This page inventories the keys answered by the **Symbol Editor** and
**Footprint Editor** workspaces, plus the one chord handled by the **Parts
Library** workspace. It describes observed behavior, not a shortcut policy.
For the shared input gates and the application-level chord notation, see
[Schematic keyboard shortcuts](keyboard-shortcuts.md); for board bindings, see
[Board keyboard shortcuts](board-keyboard-shortcuts.md).

Verified on Linux/X11 (desktop build) against a scratch `Custom symbol`
document and the starter `Custom footprint` document: every tool key armed its
tool, Ctrl+A selected all objects, Ctrl+D armed a paste preview that R rotated,
E opened the selected object's properties, M started a move, the pending-fields
refusal message appeared, Delete removed the selected object, Ctrl+Z restored
it, Ctrl+S opened the publish dialog, Ctrl+Q opened the unsaved-items prompt,
and Ctrl+, opened Preferences in all three workspaces. The raw-key clipboard
path (Ctrl+Shift+C/V) was exercised live; plain Ctrl+C/V are covered through
the real `egui_winit` translation boundary —
`native_paste_translation_reaches_the_internal_library_paste` feeds the events
the vendored translator emits for empty and image-only clipboards into the
harness, and `clipboard_translation_tests` inside `vendor/egui-winit` checks
the emission itself.
macOS and Windows have not been tested; for command
chords, macOS uses **⌘** where **Ctrl** is shown. The browser build differs in
three rows — **Ctrl+S** uses the save/export path, and **Ctrl+Q** and
**Ctrl+Shift+A** are desktop-only — and has not been separately verified.

## How keys reach these workspaces

The library workspaces do **not** share `PcbApp::shortcuts`, which serves the
schematic and board workspaces. Library editor frames dispatch their own small
set of chords in `PcbApp::library_frame`
(`crates/app/src/library_editors.rs`), and the shared definition canvas
consumes its editing keys in `library_editors/canvas.rs`. Consequences:

- Schematic and board bindings (W, A, S, H, X, D, Ctrl+A on schematic,
  layer keys, net labels) are **not dispatched at all** in these workspaces —
  they do not fire dormant schematic handlers.
- Application file keys other than the handled chords do not reach the
  library workspaces: **Ctrl+N**, **Ctrl+O**, **Ctrl+U** and
  **N**/**Shift+N** are dead here. Opening another library document uses the
  Library tab instead.
- Losing window focus resets the canvas tool to Select; switching the active
  document does the same to the document losing focus. A paste chord only arms
  while the window is focused, and a later Escape inside the same input batch
  cancels it (events resolve in order).
- While the Assistant owns input, the editing chords (Ctrl+S, Ctrl+Z, Ctrl+Y,
  Ctrl+Q) are withheld; Ctrl+, still opens Preferences.

## Workspace-level chords (library editor frame)

| Keys | Action |
| --- | --- |
| **Ctrl+,** | Opens Preferences. Runs even while the Assistant owns input. |
| **Ctrl+S** | Publishes the active document — opens the Publish dialog (desktop). In the browser build it triggers the save/export path instead. Modifier matching ignores extra Shift, so **Ctrl+Shift+S** reaches the same binding. |
| **Ctrl+Z** | Undoes the active document's last committed edit — the library document's own history, not the project's. |
| **Ctrl+Y** | Redoes the active document's edit. |
| **Ctrl+Q** | Closes the project — offers to review or discard unsaved library edits first (desktop only). |
| **Ctrl+Shift+A** | Toggles the Assistant panel (desktop only). Unlike the other rows it is handled before the frame's `enabled` gate — it still fires while a file or publish operation is busy — but only in the main host, not in a detached library window, and only when no dialog is open and the Assistant is enabled. |

Except for **Ctrl+Shift+A**, these run regardless of pointer position but
require the editor frame to be enabled: no modal dialog or file/publish
operation in progress, and no text field owning input.

## Canvas keys

The definition canvas answers keys only while its document is the active
library document, the window is enabled, and no text field owns keyboard input.
Every row except **Esc** additionally requires the pointer inside the canvas
area, not covered by a panel or floating layer.

| Keys | Action |
| --- | --- |
| **Esc** | Resets the tool to Select — cancels Draw, Polyline, Move, MoveGroup and Paste previews without committing. Does not require the pointer over the canvas and does not clear the selection. |
| **P** | Places a pin (Symbol Editor) or a pad (Footprint Editor). Ignored while Ctrl is held. |
| **L** | Starts the line tool. Ignored while Ctrl is held. |
| **T** | Starts the text tool. Ignored while Ctrl is held. |
| **M** | Starts a move: single selection → Move; multiple → MoveGroup anchored on the first object. Refuses with "Apply or reset the property fields before moving." while property input is unapplied. Fires regardless of held modifiers. |
| **R** / **Shift+R** | Rotates a quarter turn, in opposite directions. Applies to the Move, MoveGroup or Paste preview when one is armed, to the Draw-tool placement prototype otherwise — or to the current selection when neither applies. |
| **E** | Opens the selected object's details — only when exactly one object is selected. Fires regardless of held modifiers. |
| **Delete** | Deletes the selected objects. Fires regardless of held modifiers. |
| **Enter** | Commits the in-progress polyline as a graphic object; does nothing under any other tool. |
| **Ctrl+A** | Selects all objects in the definition. |
| **Ctrl+D** | Duplicates the selection into a cancellable Paste preview. Refuses while property fields are pending. |
| **Ctrl+C** | Copies the selection to the internal library clipboard via the normalized Copy event; the raw key is also accepted when another modifier bypasses native translation. |
| **Ctrl+V** | Arms the cancellable paste preview via the normalized Paste or PasteImage event (same raw-key fallback); the chord completes even when the OS clipboard holds nothing usable; refuses while property fields are pending. |

Tools with ribbon buttons but **no** key binding: Rectangle, Circle, Arc,
Polyline, Mirror X and Array. Polyline is armed from the ribbon and committed
with Enter.

### Clipboard chords

On desktop, `egui-winit` translates command-only **C**/**V**/**X** presses into
`Event::Copy`, `Event::Paste` and `Event::Cut` and does not emit the
corresponding `Event::Key`. A plain **Ctrl+V** press is a completed clipboard
read even when the OS clipboard holds nothing usable: the vendored
`egui-winit` (`vendor/egui-winit/PATCH.md`) emits `Event::Paste("")` when the
read comes back empty or unsupported — a no-op for text fields — and keeps
upstream's `Event::PasteImage` for image-only clipboards. The canvas consumes
`Paste` (text or empty) and `PasteImage` as paste intent, so plain
**Ctrl+C**/**Ctrl+V** reach the internal clipboard operations whatever the OS
clipboard contains; **Ctrl+X** has no library operation and its `Event::Cut`
remains unhandled.

Adding a second modifier escapes the translation: **Ctrl+Shift+C** and
**Ctrl+Shift+V** emit their `Key` events, and the canvas accepts those too —
each press produces exactly one of the two event forms, so a chord runs copy
or paste once. The Paste preview behaves the same as Ctrl+D's. Copy and paste
are gated to a selection of the same document kind: pasting a symbol's objects
into a footprint document reports "Paste into a definition of the same kind."

## Pending fields gate

Property fields (name, number, orientation, dimensions) apply atomically.
While unapplied input exists, **M**, **Ctrl+D**, paste and tool changes via
**P**/**L**/**T** are refused with "Apply or reset the property fields …"
rather than silently dropping the draft. Typing in a field owns keyboard input,
so canvas keys do not fire while a field is focused.

## Where each binding is handled

| Row | Consumer | Focused test |
| --- | --- | --- |
| Ctrl+, | `PcbApp::library_frame` → `open_preferences` | — |
| Ctrl+S | `library_frame` → `Intent::Publish` / `Intent::Save` → `begin_library_publish` / `library_editors.save` | — (`completed_save_releases_close_without_reusing_a_cancelled_decision` covers the save path, not the chord) |
| Ctrl+Z / Ctrl+Y | `library_frame` → `Intent::Undo/Redo` → `Editors::edit(Action::Undo/Redo)` | `live_library_commands_share_the_gui_documents` (undo/redo via `app.library_edit`), `canvas_selection_move_and_cancel_use_library_history` (Undo Library Edit) |
| Ctrl+Q | `library_frame` → `begin_project_close` | `native_library_close_preserves_unsaved_authoring` (close path) |
| Ctrl+Shift+A | `library_frame` → `assistant_shortcuts` → `toggle_assistant` | `assistant.rs` toggle tests |
| Esc | `canvas` → `Tool::Select` | `canvas_selection_move_and_cancel_use_library_history` (draw-tool cancel), `unfocused_host_does_not_cancel_another_windows_definition_tool` (focus scoping) |
| P / L / T | `canvas` → `start_tool(Place::Pin\|Pad\|Line\|Text)` | — (ribbon clicks exercise `start_tool`) |
| M | `canvas` → `begin_move` | `canvas_selection_move_and_cancel_use_library_history` (move + cancel), `library_group_selection_and_stale_array_do_not_overwrite_edits` (group path) |
| R / Shift+R | `canvas` → `rotate_by(-1\|+1)` → preview/prototype/`transform_selection` | — |
| E | `canvas` → `details_requested` (single selection) | — |
| Delete | `canvas` → `delete` → `Action::Delete` | — |
| Enter | `canvas` → `Action::Insert` for `Tool::Polyline` only | — |
| Ctrl+A | `canvas` → `select_many(all)` | — (the Ctrl+A presses in `library_editors/tests.rs` are field-scoped select-all, not canvas) |
| Ctrl+D | `canvas` → `duplicate` → `start_copy` → `Tool::Paste` | — |
| Ctrl+C / Ctrl+V | `canvas` → `copy` / `paste` → `Tool::Paste` (normalized `Event::Copy`/`Event::Paste`/`Event::PasteImage`, or the raw key when another modifier bypasses translation) | `native_paste_translation_reaches_the_internal_library_paste` (empty + image-only clipboards, both kinds, single undoable commit), `native_paste_chord_only_arms_the_focused_library_view` (detached-view ownership), `normalized_clipboard_events_drive_library_copy_and_paste` (gates, same-frame Escape ordering + pending refusal), `clipboard_translation_tests` in `vendor/egui-winit` (emission) |
| Workspace boundary | `PcbApp::shortcuts` not invoked; `library_frame` key block | `home_has_no_editor_shortcuts_and_selected_copy_enters_the_workspace` |
| Parts Library | `library_workspace.rs` — `COMMAND_COMMA` only | `library_workspaces_share_the_picker_with_schematic_and_pcb` |

Rows marked `—` have no focused key test; their handlers are exercised through
the ribbon actions and command path instead.
