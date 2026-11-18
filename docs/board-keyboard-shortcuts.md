# Board keyboard shortcuts

This page inventories the board (PCB) editor's current keyboard bindings and the
application-level shortcuts that apply while a board view is focused. It is the
board-workspace sibling of the [schematic inventory](keyboard-shortcuts.md) and
uses the same convention: a record of observed behavior, not a policy. The
in-app **Preferences → Shortcuts** page shows a subset of these bindings
formatted for the host platform; this page is the fuller inventory.

**Verification.** Checked against the desktop build on Linux (X11) in the board
workspace of a copied fixture: X armed the route tool, Ctrl+Shift+X the via
tool and Ctrl+Shift+M the ruler; PageDown/− switched the active layer Top↔
Bottom; M picked up a hovered reference designator and a footprint body, R
rotated the draft and F flipped it (the designator rendered mirrored); Ctrl+A
selected all 15 footprints; ` highlighted the net under the pointer and U
expanded the seed to its connected set; D selected a segment for dragging;
Enter attempted a route finish with the pointer off the copper (the commit ran
and was rejected for connecting two nets — Enter reached the handler without a
pointer target); Ctrl+D opened the PCB Paste Options dialog; Ctrl+U toggled
units and N stepped the grid finer. The armed clipboard preview's own
Esc/R/Shift+R handling is code-cited, not exercised live. For command chords,
macOS uses **⌘** where **Ctrl** is shown below. The routing grid bypass and
the route-time layer keys read the physical **Ctrl** key, as noted in their
rows. The browser build omits the desktop-only project keys and has not been
separately verified for this inventory. macOS and Windows have not been tested
for this inventory either; the macOS modifier notation describes the bindings,
not a verified platform run.

## Reading the conditions

The shared gates from the schematic page apply unchanged: the focused view
receives the event, no dialog, popup or modal panel is open, no text field or
the Assistant owns input. Board-specific additions:

- The layer keys (**PageUp**, **PageDown**, **+**, **−**) run ahead of the other
  board-key checks, with their own text-field, Assistant, dialog and popup gates.
  They can still change the active layer while a paste or duplicate preview is
  armed, because they run before the clipboard-preview guard. That guard blocks
  the remaining board-key dispatch; the preview handles Esc and R/Shift+R itself.
- A footprint transform in progress (move draft, reference drag or the movement
  tool) blocks **X**, **Ctrl+Shift+X** and **Ctrl+Shift+M**.
- Hover-directed actions (**`**, **D**, and hover-directed uses of **U**, **M**,
  **R**, **F**) need the pointer over the canvas. **U** can instead use selected
  copper, and **M**, **R**, **F** can act on a single selected footprint when
  the pointer is outside the canvas; the shared input gates still apply.

## Application and edit keys

The shared chords from the schematic inventory's
[application table](keyboard-shortcuts.md#application-and-edit-keys) apply to a
focused board view: New/Open/Save/Preferences/Quit, Undo/Redo, the clipboard
chords, **N**/**Shift+N** grid stepping, **Ctrl+U** units and **Esc** on Home.
Board differences:

- **Ctrl+A** selects visible board items enabled by the selection filters — the
  schematic has no Select All binding.
- The clipboard chords operate on board selections (footprints, tracks, vias,
  drawings). Copy, cut and duplicate reject the entire selection if it includes
  a zone; deselect zones before using those operations. Duplicating linked
  footprints goes through the **PCB Paste Options** dialog first; paste and
  duplicate then arm a placement preview that owns input until it is placed or
  cancelled — while armed, **R**/**Shift+R** rotate the ghost and **Esc** drops it,
  while the layer keys remain available as described above.

## Board tool keys

These keys arm or replace a tool rather than requiring an idle canvas; the
shared focus gates still apply.

| Key | Action | Conditions |
| --- | --- | --- |
| X | Route tracks | Active layer must be visible copper; no footprint transform in progress. Drops drawing, zone and measure tools first. This is routing, not the schematic's mirror |
| Ctrl+Shift+X | Place via | Board must have two copper layers and no footprint transform in progress; places vias at clicks until Esc |
| Ctrl+Shift+M | Measure | Same transform gate; starts the distance ruler and drops other gestures |
| PageUp / PageDown | Front / back layer | Switches the active layer. Mid-route, with the pointer on the canvas, they instead drop a via and continue on that side. Physical Ctrl still counts while routing — Ctrl+PageUp/PageDown work then |
| + / − | Next / previous layer | Shift+= also counts as +. These two use unmodified keys only — Ctrl+= does nothing even while routing |

## Selection and editing keys

| Key | Action | Conditions |
| --- | --- | --- |
| Ctrl+A | Select all | Selects visible board items enabled by the selection filters; hidden-layer items are excluded. Suppressed while a tool or clipboard preview owns input |
| Delete | Delete selection | Removes the selection. While a track, zone or drawing tool owns a selection, Delete routes to that tool instead |
| M | Move | Hovered or single-selected footprint (or a selected reference designator): picks it up for click placement. A multi-item selection containing footprints, zones or drawings: picks the group up. A copper-only selection: the track module's own move. A selected item also responds when the pointer is elsewhere |
| R / Shift+R | Rotate 90° | Rotates the hovered or single-selected footprint, or the active move/placement preview, counterclockwise and clockwise. The board has no mirror keys |
| F | Flip side | Moves the hovered or single-selected footprint (or move draft) to the opposite board side |
| U | Select connected copper | Seeds from copper under the pointer, or selected copper when no copper is hovered, including when the pointer is outside the canvas; expands to the connected set allowed by visibility and selection filters |
| D | Drag track segment | Requires a track under the pointer (not over a footprint); moves the segment, click commits |
| ` (backtick) | Highlight net | Highlights the net under the pointer, respecting copper-layer visibility; over empty space it clears the highlight |
| Esc | Cancel | Order: a pending DRC job first, then movement, measure, tracks, zones, drawing, a move draft, reference drag, placement preview, box selection, then the selection itself; panels, popups and dialogs come earlier still |
| Right-click | Cancel / menu | Cancels an in-progress gesture; on an idle canvas opens the clipboard context menu |

## Keys while a board tool is running

| Key | Action | Conditions |
| --- | --- | --- |
| Enter | Finish | While routing: commits the route draft as drawn — unlike the schematic wire tool, no pointer target is needed. While drawing a zone or graphic-line outline: closes the shape |
| Backspace | Undo last step | While routing: undoes a via at the last corner first, then the corner itself; with no draft, does nothing. In zone/drawing outlines: removes the last point |
| Slash | Flip track posture | While the route tool is armed, swaps the bend direction of future corners |
| V | Place via mid-route | While routing with the pointer on the canvas, drops a via at the pointer and switches to the paired layer |
| Ctrl (hold) | Bypass grid | While routing, holding the physical **Ctrl** key lets corners land off-grid; copper contacts still snap. Physical Ctrl on macOS too — an exception to the modifier note above |
| Esc / right-click | Cancel | Follows the layered order above; right-click cancels the gesture directly |
| Double-click | Finish | During a route or a drawing-line outline: commits the current shape |

## Where each binding is handled

Every row is consumed in `PcbApp::shortcuts` (`crates/app/src/lib.rs`) or in the
board module named here. Focused tests that exercise a binding are listed beside
it.

| Rows | Consumed by | Focused tests |
| --- | --- | --- |
| Layer keys (PageUp/PageDown/+/−) | `PcbApp::shortcuts` → `navigate_board_layer`; mid-route they set `tracks.via_side_requested` | `tests/board_layers.rs` (`keyboard_layer_navigation_matches_the_picker_and_preserves_history`) |
| Clipboard chords | Shared `clipboard_shortcuts`; board items via `board_clipboard.rs`, including the armed preview's own Esc and R/Shift+R | — (board clipboard paths are covered in `tests/board_tracks.rs`; the chords themselves have no focused test) |
| Ctrl+A | `PcbApp::shortcuts` → `select_all_board` (`board_clipboard.rs`) | `tests/board_zones.rs` exercises `select_all_board` |
| X | `PcbApp::shortcuts` → `start_route` (`board_workspace.rs`) | `tests/board_tracks.rs` (`x_routes_between_pads_in_one_undoable_command`, `routing_requires_a_visible_copper_layer`) |
| Ctrl+Shift+X | `PcbApp::shortcuts` → `start_via` (`board_workspace.rs`, `board_tracks.rs`) | `tests/board_tracks.rs` (`place_via_tool_drops_vias_until_escape`) |
| Ctrl+Shift+M | `PcbApp::shortcuts` → `start_measure` (`board_workspace.rs`) | `tests/board_tracks.rs` (`measure_reports_the_distance_between_snapped_points`) |
| M | `board.move_requested` → `board_move.rs` `MovePlan` for mixed groups, `board_canvas.rs` draft for a single footprint, `board_tracks.rs` `move_requested` for copper-only selections | `tests/board_tracks.rs` (`m_and_drag_move_selected_copper`), `tests/board_views.rs` (`pcb_reference_can_move_with_m_or_primary_drag`), `tests/board_zones.rs` (`selected_zone_moves_deletes_and_respects_filters`) |
| R, Shift+R | `board.rotate_requested` (90/270) → `board_canvas.rs` footprint draft/edit | `tests/board_tracks.rs` (`f_flips_footprints_between_sides` shares the draft path) |
| F | `board.flip_requested` → `board_canvas.rs` `Draft.bottom` toggle | `tests/board_tracks.rs` (`f_flips_footprints_between_sides`) |
| U | `tracks.connected_requested` → `board_tracks.rs` connected-copper selection | `tests/board_tracks.rs` (`u_selects_connected_copper`) |
| D | `tracks.drag_segment_requested` → `board_tracks.rs` `SegmentDrag` | — |
| ` (backtick) | `board.net_highlight_requested` → `board_canvas.rs` `highlight_net` | `tests/board_tracks.rs` (`backtick_highlights_the_net_under_the_pointer`) |
| Delete | `PcbApp::shortcuts` → `delete_board_items`, or the owning tool's `delete_requested` | `tests/board_tracks.rs` (`clicking_a_track_selects_it_and_delete_removes_it`, `a_delete_pressed_while_track_input_is_disabled_never_deletes_later`) |
| Esc layers | `PcbApp::shortcuts` → `drc.cancel` → `movement`/`measure`/`tracks`/`zones`/`drawing` `escape()` → draft and gesture `take()` → `select_board_items` clear; armed clipboard previews consume it in `board_clipboard.rs` | `tests/board_tracks.rs` (`unfinished_routes_cancel_and_finish_from_the_keyboard`) |
| Route Enter/Backspace/Slash/V | `tracks.finish_requested`/`backspace_requested`/`posture_requested`/`via_requested` → `board_tracks.rs` (`diagonal_first`, `Draft`) | `tests/board_tracks.rs` (`unfinished_routes_cancel_and_finish_from_the_keyboard`, `v_places_vias_and_switches_layers_mid_route`, `backspace_undoes_a_via_and_vias_select_and_delete`) |
| Zone/drawing Enter/Backspace | `zones.finish_requested`/`backspace_requested`, `drawing.finish_requested`/`backspace_requested` → `board_zones.rs`, `board_drawing.rs` | `tests/board_zones.rs` (`real_zone_rectangle_and_polygon_are_single_edits`), `tests/board_drawing.rs` (`rectangle_canvas_create_cancel_delete_and_undo`) |
| Ctrl grid bypass | `board_tracks.rs` `snap_to_grid` reads `input.modifiers.ctrl` | `tests/board_tracks.rs` (`ctrl_bypasses_the_routing_grid_but_keeps_copper_contacts`) |
| Double-click | `board_tracks.rs` and `board_drawing.rs` (`double_clicked` finishes the draft) | — |

## Notes for contributors

The board/schematic differences and input exceptions below and in this inventory
are [reported to #168](https://github.com/tangency-labs/pcbcraft/issues/168#issuecomment-6967295238)
for later shortcut consolidation. This page records existing behavior without
choosing new bindings or defaults.

- Board keys deliberately reuse letters the schematic gives other meanings:
  **X** routes instead of mirroring, **R**/**Shift+R** rotate both directions
  (the schematic mirrors on **X**/**Y** instead), **Ctrl+A** exists here but not
  there, and **U** selects connected copper rather than switching units. The
  unit toggle is **Ctrl+U** on both canvases.
- Schematic-only keys that do nothing on the board include **W**, **Q**, **E**,
  **G**, **L**, **H**, **S**, **P** and **A**. Footprint properties are edited
  through the Properties panel or context menu, not a key.
- Enter is intentionally asymmetric between workspaces: a route finishes on the
  draft alone, while a schematic wire needs the pointer's in-canvas target.
