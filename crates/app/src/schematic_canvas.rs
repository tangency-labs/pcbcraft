use std::sync::Arc;

pub(crate) mod fields;
mod overlaps;
pub(crate) use overlaps::Cache as OverlapCache;
mod selection;
mod tool_menu;
pub(crate) use tool_menu::Action as ToolAction;
#[cfg(not(target_arch = "wasm32"))]
mod thumbnail;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use thumbnail::Thumbnail;
mod wires;
pub(crate) use selection::BoxDrag;

/// A popup owns a revision-bound list without retaining an old project or re-resolving every row.
#[derive(Clone)]
struct SymbolChoices {
    target: pcbcraft_engine::SheetTarget,
    revision: u64,
    entries: Vec<(String, String)>,
}

#[cfg(test)]
mod tests;

pub(crate) use crate::canvas_camera::Camera;
pub(crate) use wires::{Draft, LegTarget, ReshapeDrag};

use crate::preferences::Navigation;
use egui::{Align2, FontId, Key, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};
use pcbcraft_chrome::{MenuItem, Tokens, grid as canvas_grid, menu, theme};
use pcbcraft_engine::wire::Anchor;
use pcbcraft_engine::{
    Project,
    schematic::{GRID_NM, MAX_CENTER_NM, Point, Shape, SymbolDefinition, SymbolPlacement, TextAlign},
    selection::Selection,
};

/// The active canvas interaction, bound to the revision at which it began.
#[derive(Clone, Debug)]
pub(crate) enum Tool {
    NoConnect {
        revision: u64,
    },
    Field {
        drag: fields::Drag,
        revision: u64,
    },
    /// A local/global label preview moves its electrical anchor and complete text together.
    NetLabel {
        label: pcbcraft_engine::net_labels::Label,
        revision: u64,
        /// Pointer-to-anchor offset for a held drag; keyboard pickup finishes on a click.
        drag: Option<Vec2>,
    },
    SheetGesture {
        drag: Box<crate::hierarchy::Drag>,
        revision: u64,
    },
    Hierarchy {
        kind: crate::hierarchy::Kind,
        id: Option<u64>,
        offset: Vec2,
        revision: u64,
    },
    PlaceSymbol {
        definition: Arc<SymbolDefinition>,
        unit: String,
        component: Option<String>,
        part: Option<pcbcraft_engine::part_instances::Selection>,
        value: String,
        revision: u64,
        turns: u8,
    },
    Wire {
        revision: u64,
        start: Option<Anchor>,
    },
    BoxSelect {
        drag: BoxDrag,
        revision: u64,
    },
    MoveItems {
        items: Selection,
        origin: Point,
        revision: u64,
        /// Keyboard pickup follows hover and finishes on a click; held drags finish on release.
        pickup: bool,
        /// Move cuts selection-boundary contacts; Grab retains and stretches them.
        disconnect: bool,
    },
    Move {
        symbol: SymbolPlacement,
        /// Preview-only quarter turns relative to the symbol's starting orientation.
        turns: u8,
        offset: Vec2,
        revision: u64,
        /// Leave old wires at the original pins instead of stretching them when true.
        disconnect: bool,
        /// Keyboard/context pickup follows the pointer without holding a mouse button.
        pickup: bool,
    },
    /// Dragging a segment, corner or free end of the selected wire.
    Reshape {
        drag: wires::ReshapeDrag,
        revision: u64,
    },
}

impl Tool {
    /// Return the revision used to reject stale canvas actions.
    pub(crate) fn revision(&self) -> u64 {
        match self {
            Self::NoConnect { revision, .. }
            | Self::Field { revision, .. }
            | Self::NetLabel { revision, .. }
            | Self::SheetGesture { revision, .. }
            | Self::Hierarchy { revision, .. }
            | Self::PlaceSymbol { revision, .. }
            | Self::Wire { revision, .. }
            | Self::Move { revision, .. }
            | Self::BoxSelect { revision, .. }
            | Self::MoveItems { revision, .. }
            | Self::Reshape { revision, .. } => *revision,
        }
    }
}

/// A placement, selection, move, edit or cancellation requested through the canvas.
pub(crate) enum Click {
    BeginNoConnect,
    NoConnect(pcbcraft_engine::no_connect::Attachment, bool),
    InvalidNoConnect,
    CancelNoConnect,
    ToolAction(ToolAction, egui::Context),
    GrabField(fields::Drag),
    RotateField(fields::Drag),
    EditField(fields::Drag),
    FinishField(Option<pcbcraft_engine::schematic::fields::Layouts>),
    NetLabelCommit(pcbcraft_engine::net_labels::Label),
    NetLabelAction(pcbcraft_engine::net_labels::Label, crate::net_labels::Action),
    NetLabelDrag(pcbcraft_engine::net_labels::Label, Vec2),
    NetLabelCancel,
    NetLabelRotate,
    GrabSheetGesture(Box<crate::hierarchy::Drag>),
    FinishSheetGesture(Option<crate::hierarchy::Geometry>),
    SelectSheetSymbol(u64),
    HierarchyPlace(Point),
    HierarchyClipboard(crate::hierarchy::Kind, u64, tool_menu::Action, egui::Context),
    HierarchyAction(crate::hierarchy::Kind, u64, crate::hierarchy::Action),
    Place(Point),
    StartBox(BoxDrag),
    SelectItems(Selection),
    /// Select one leg for direct deletion and reshaping.
    SelectLeg(LegTarget),
    /// Translate from this model point with explicit gesture and attachment behavior.
    MoveItems {
        items: Selection,
        origin: Point,
        pickup: bool,
        disconnect: bool,
    },
    FinishItems(Option<Point>),
    /// A drag started on a wire segment, corner or free end; the handler selects its target.
    GrabReshape(wires::ReshapeDrag),
    /// The reshape drag ended: a free position or explicit attachment target, or `None` to cancel.
    FinishReshape(Option<Anchor>),
    DeleteItems,
    WirePoint(Anchor, bool),
    /// A plain terminal/node click begins a fresh wire draft without a tool shortcut.
    StartWire(Anchor),
    CancelWire,
    Grab(SymbolPlacement, Vec2, bool),
    FinishMove(Option<Point>),
    /// Select a symbol by ID, or clear the selection after a click on empty canvas.
    Select(Option<String>),
    /// A row of the selected symbol's context menu.
    Edit(SymbolAction, String),
    /// A row of a wire's context menu, with the leg it was opened on.
    WireEdit(WireAction, wires::LegTarget),
    /// A native symbol template dragged from the catalog was released at this snapped point.
    Drop(PartDrag, Point),
}

/// A component/power catalog drag bound to its originating view, sheet and revision.
#[derive(Clone, Debug)]
pub(crate) struct PartDrag {
    pub(crate) placement: crate::parts_browser::Placement,
    pub(crate) source: crate::editor_view::ViewInput,
}

/// Draw a part's preview fitted inside `rect`, for the parts browser's thumbnails. The symbol
/// is drawn unrotated and without labels. A shared minimum preview extent keeps small symbols
/// from growing disproportionately large across the catalog.
pub(crate) fn paint_preview(painter: &Painter, definition: &SymbolDefinition, rect: Rect, color: egui::Color32) {
    let bounds = grid_bounds(definition.default_unit(), 0, false);
    let extent = bounds.size().max(theme::SCHEMATIC_PREVIEW_EXTENT);
    let step = (rect.width() / extent.x).min(rect.height() / extent.y).min(theme::SCHEMATIC_PREVIEW_MAX_STEP);
    let center = rect.center() - bounds.center().to_vec2() * step;
    paint_symbol(painter, definition.default_unit(), center, step, 0, false, Stroke::new(theme::SCHEMATIC_PREVIEW_STROKE, color));
}

/// What a wire's context menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WireAction {
    /// Normalize selected wire geometry and exact pin contacts.
    Heal,
    /// Split the wire at the grid point under the click, keeping it connected (KiCad's Slice).
    Slice,
    /// Detach the leg under the click from neighboring legs, pins and shared junctions.
    Break,
    /// Remove the leg under the click, leaving free ends.
    DeleteSegment,
    /// Remove the whole wire.
    Delete,
    /// Select the complete route for translation or deletion.
    SelectWire,
}

/// What the selected symbol's context menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SymbolAction {
    Grab,
    DisconnectMove,
    Rotate,
    Mirror(pcbcraft_engine::schematic::MirrorAxis),
    EditFields,
    AssignFootprint,
    Delete,
}

/// Draw the schematic and its active preview, and report a requested tool action.
/// A primary drag reshapes one wire, grabs a symbol or moves an explicitly selected whole-wire/group; empty-space and secondary drags draw
/// selection boxes over components, wires and named-net anchors, including power/ground.
/// Drag previews end on release, with no edit outside the canvas or after focus loss.
/// Junction handles preview explicit pin/junction attachment through the engine before release.
///
/// Disconnect and Move follows the pointer until a primary click commits; Escape, a secondary
/// click or focus loss discards the preview without changing the document.
/// Active tool menus return `Click::ToolAction` and retain the preview while the popup owns
/// input. Empty canvas offers drawing, selection and navigation actions.
///
/// With canvas input enabled, a primary press during
/// placement returns `Click::Place` at a valid snapped point. In Wire mode,
/// primary clicks return `Click::WirePoint` for pins, junctions or snapped free points;
/// double-click or a queued Enter request marks the route complete.
/// A browser part dragged over the canvas previews at the snapped pointer position; releasing
/// it returns `Click::Drop`. Blocked input or an out-of-bounds position suppresses both.
/// Return `None` for a preview without a valid click, an inactive tool, or
/// blocked or dragged canvas input when the wire menu did not cancel.
///
/// With no tool, double-click a body for properties or visible text for an in-place editor.
/// A primary click selects one wire leg or an item, or clears
/// selection on empty canvas. Shift/Ctrl combine those hits with the previous selection. A secondary click selects the symbol under it and opens its menu; a row
/// picked there returns `Click::Edit`. A secondary click on a wire selects it and opens Slice, Break, Delete Segment and Delete Wire for the leg under the click, returning `Click::WireEdit`
/// bound to `revision`. `selected_leg` limits blue ink and Delete to one leg; `whole_wire_drag`
/// opts out of handles and reshaping for explicit whole-route movement. G over a wire starts
/// a reshape completed by the next click. Other `selected` items use the accent colour. Both mouse buttons can start a box;
/// direction chooses containment or crossing, and Shift/Ctrl add or toggle hits. Group moves
/// preview the same bounded transformation used by the engine and commit only on valid release.
/// M picks up a multi-item selection or an explicitly selected wire and commits on a valid
/// primary click; Escape, right-click or focus loss cancels that preview.
/// M cuts contacts to unselected objects; G grabs a group with its external contacts retained,
/// matching held group dragging. Both preserve contacts within the selected group.
/// Symbol move previews also include their pending quarter turns; painted wire contacts use
/// that transformed snapshot, and cancellation discards both position and orientation.
/// `grid_opacity` is the grid's strength in percent; it changes only how the grid is drawn, never
/// where points snap. The camera's snap spacing sets both the drawn grid and every snapped point;
/// pins and junctions are picked by distance, so they stay reachable at any spacing.
/// `label` distinguishes simultaneously visible canvases in accessibility output.
/// `highlighted` marks counterparts selected in another view. It affects paint only; hit
/// testing, group moves and deletion use `selected`, which takes visual precedence on overlap.
/// Drawing-bound overlaps have persistent view-only markers and placement warnings; coincident
/// bodies remain allowed. The symbol context menu clarifies selection without authoring edits.
/// `filters` restricts item picking, boxes and context menus without disabling explicit drawing tools.
/// Navigation authority is separate from authoring input, allowing a read-only clipboard ghost
/// to pan/zoom without enabling picking, tools or model edits.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut Ui,
    project: &Project,
    revision: u64,
    camera: &mut Camera,
    overlap_cache: &mut OverlapCache,
    tool: Option<Tool>,
    selected: &Selection,
    selected_leg: Option<&LegTarget>,
    whole_wire_drag: bool,
    selected_sheet: Option<u64>,
    highlighted: &Selection,
    filters: crate::selection_filters::Filters,
    input_enabled: bool,
    navigation_enabled: bool,
    navigation: Navigation,
    grid_opacity: u8,
    auto_heal: bool,
    repeat: bool,
    unit: pcbcraft_chrome::units::Unit,
    draft: &Draft,
    label: &str,
    editing: Option<&crate::inline_text::Target>,
) -> Option<Click> {
    let rect = ui.available_rect_before_wrap();
    // Keep the canvas from taking keyboard focus away from editor shortcuts.
    let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Pane, true, label));
    let moving = matches!(
        tool,
        Some(
            Tool::Field { .. }
                | Tool::NetLabel { drag: Some(_), .. }
                | Tool::SheetGesture { .. }
                | Tool::Move { .. }
                | Tool::MoveItems { .. }
                | Tool::BoxSelect { .. }
                | Tool::Reshape { .. }
        )
    );
    camera.navigate(&response, rect, project, navigation_enabled && !moving, navigation);

    let field_destination = if let Some(Tool::Field { drag, .. }) = &tool {
        ui.input(|i| i.pointer.hover_pos()).and_then(|p| fields::destination(project, camera, rect, drag, p))
    } else {
        None
    };
    let field_preview = if let (Some(Tool::Field { drag, .. }), Some(layouts)) = (&tool, &field_destination) {
        fields::preview(project, &drag.symbol, layouts.clone())
    } else {
        None
    };
    let group_delta = match &tool {
        Some(Tool::MoveItems { origin, pickup, .. }) => {
            let pointer = if *pickup { response.hover_pos().or_else(|| response.interact_pointer_pos()) } else { response.interact_pointer_pos() };
            pointer.map(|pointer| {
                let point = camera.snap(pointer, rect);
                Point { x_nm: point.x_nm - origin.x_nm, y_nm: point.y_nm - origin.y_nm }
            })
        }
        _ => None,
    };
    let reshape_target = match &tool {
        Some(Tool::Reshape { drag, .. }) => {
            ui.input(|input| input.pointer.hover_pos()).and_then(|pointer| drag.destination(project, camera, rect, pointer))
        }
        _ => None,
    };
    let moved = match &tool {
        Some(Tool::Move { symbol, offset, .. }) => response
            .hover_pos()
            .or_else(|| response.interact_pointer_pos())
            .filter(|pointer| rect.contains(*pointer))
            .map(|pointer| (symbol.clone(), camera.snap(pointer + *offset, rect)))
            .filter(|(_, point)| valid_position(*point)),
        _ => None,
    };
    let group_preview = match (&tool, group_delta, reshape_target) {
        (Some(Tool::Move { symbol, turns, disconnect, .. }), _, _) => moved.as_ref().and_then(|(_, point)| {
            project
                .transformed_symbol(
                    &symbol.id(),
                    *point,
                    Some((symbol.turns() + turns) % 4),
                    Some(symbol.is_mirrored()),
                    *disconnect,
                    revision,
                    auto_heal,
                )
                .ok()
        }),
        (Some(Tool::MoveItems { items, disconnect, .. }), Some(delta), _) => items.transformed(project, delta, *disconnect, revision, auto_heal).ok(),
        (Some(Tool::Reshape { drag, .. }), _, Some(to)) => drag.preview(project, to, revision, auto_heal),
        _ => None,
    };
    let hierarchy_preview = if let Some(Tool::Hierarchy { kind, id: Some(id), offset, .. }) = &tool {
        response.hover_pos().and_then(|p| {
            project
                .moved_hierarchy(
                    &format!("{}-{id}", if *kind == crate::hierarchy::Kind::Sheet { "sheet-symbol" } else { "label" }),
                    camera.snap(p + *offset, rect),
                )
                .ok()
        })
    } else {
        None
    };
    let sheet_preview = if let Some(Tool::SheetGesture { drag, .. }) = &tool {
        ui.input(|i| i.pointer.hover_pos()).and_then(|pointer| drag.preview(project, drag.geometry(project, camera, rect, pointer)))
    } else {
        None
    };
    let label_position = if let Some(Tool::NetLabel { label, revision, drag }) = &tool {
        ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)).map(|pointer| {
            wires::label_position(
                project,
                camera,
                rect,
                pointer + drag.unwrap_or(Vec2::ZERO),
                draft,
                *revision,
                (label.id != 0).then_some(pcbcraft_engine::schematic::WireEndpoint::NetLabel(label.id)),
            )
        })
    } else {
        None
    };
    let net_label_preview = if let (Some(Tool::NetLabel { label, .. }), Some(at)) = (&tool, label_position) {
        project.dragged_net_label(label.id, at, label.rotation, revision, auto_heal).ok()
    } else {
        None
    };
    let project = field_preview
        .as_ref()
        .or(net_label_preview.as_ref())
        .or(sheet_preview.as_ref())
        .or(hierarchy_preview.as_ref())
        .or(group_preview.as_ref())
        .unwrap_or(project);
    let painter = ui.painter_at(rect);
    let tokens = Tokens::get();
    let overlap_index = overlap_cache.get(project, revision, &painter, camera.unit_step());
    let screen = |point| camera.screen(point, rect);
    let origin = screen(Point { x_nm: 0, y_nm: 0 });
    let unit_step = camera.unit_step();
    canvas_grid::paint(&painter, rect, origin, camera.snap_step(), grid_opacity);
    let mut whole_wires = selected.wires.clone();
    if let Some(leg) = selected_leg {
        whole_wires.remove(&leg.wire);
    }
    // A transformed snapshot already contains the preview's pin positions and orientation.
    // Overriding them with the captured starting symbol would draw wires to its old rotation.
    let moved_wires = if group_preview.is_some() { None } else { moved.clone() };
    wires::paint(&painter, rect, project, camera, moved_wires, &whole_wires, &highlighted.wires);
    if group_preview.is_some()
        && let Some(target) = reshape_target
        && !matches!(target, Anchor::Free(_))
    {
        wires::paint_attachment(&painter, rect, project, camera, target);
    }
    if let Some(leg) = selected_leg {
        let offset = match (&tool, reshape_target) {
            (Some(Tool::Reshape { drag, .. }), Some(Anchor::Free(to))) if group_preview.is_some() => {
                Point { x_nm: to.x_nm - drag.grabbed.x_nm, y_nm: to.y_nm - drag.grabbed.y_nm }
            }
            _ => Point { x_nm: 0, y_nm: 0 },
        };
        wires::paint_selected_leg(&painter, rect, camera, leg, offset);
    }
    let visible = rect.expand((unit_step * 5.0).max(theme::SCHEMATIC_FIT_MARGIN));
    for symbol in project
        .symbols()
        .iter()
        .filter(|s| !selected.symbols.contains(&s.id()))
        .chain(project.symbols().iter().filter(|s| selected.symbols.contains(&s.id())))
    {
        let position = moved.as_ref().filter(|(moving, _)| moving.id() == symbol.id()).map_or(symbol.position, |(_, point)| *point);
        let center = screen(position);
        if visible.contains(center) {
            let color = if selected.symbols.contains(&symbol.id()) {
                tokens.accent
            } else if highlighted.symbols.contains(&symbol.id()) {
                tokens.schematic_highlight
            } else {
                tokens.text
            };
            paint_symbol(
                &painter,
                symbol.unit_definition(),
                center,
                unit_step,
                symbol.turns(),
                symbol.is_mirrored(),
                Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color),
            );
        }
    }

    fields::paint(ui, project, camera, rect, selected, highlighted, tool.as_ref(), input_enabled && filters.symbols, editing);
    overlap_index.paint(ui, response.id, project, camera, rect);
    crate::no_connect::paint(ui, &painter, project, camera, rect, selected, highlighted);
    crate::net_labels::paint(ui, rect, camera, project, tool.as_ref(), label_position, selected, editing);
    crate::hierarchy::paint(&painter, rect, camera, project, tool.as_ref(), response.hover_pos().filter(|_| filters.sheets), selected_sheet);
    // Handles go over the symbols, so a corner under a symbol's outline can still be grabbed.
    if !whole_wire_drag
        && matches!(tool, None | Some(Tool::Reshape { .. }))
        && let Some(wire) = selected.single_wire()
    {
        wires::paint_handles(&painter, rect, project, camera, wire);
    }

    // A placement commits on press. Own the rest of that press/release even if one-shot
    // placement already returned to selection mode, so it cannot drag the newly placed part.
    let pressed_placement = response.id.with("placed_on_press");
    if ui.data(|data| data.get_temp::<bool>(pressed_placement).unwrap_or(false)) {
        if !ui.input(|input| input.pointer.primary_down())
            || ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))))
        {
            ui.data_mut(|data| data.remove::<bool>(pressed_placement));
        }
        return None;
    }

    if tool_menu::supports(tool.as_ref())
        && (input_enabled || response.context_menu_opened())
        && let Some(popup) = menu::context(&response, |ui| tool_menu::active(ui, tool.as_ref(), repeat))
    {
        let anchor = egui::Popup::position_of_id(ui.ctx(), egui::Popup::default_response_id(&response));
        if let (Some(Tool::PlaceSymbol { definition, unit, value, turns, .. }), Some(at)) = (&tool, anchor)
            && let Some(drawing) = definition.unit(unit)
        {
            let at = wires::symbol_position(project, drawing, *turns, camera, rect, at, draft, revision);
            if valid_position(at)
                && let Some(preview) =
                    SymbolPlacement::preview_unit(definition.clone(), unit, *turns, definition.defaults.clone().unwrap_or_default().field_layout)
            {
                paint_unit_template(&painter, &preview, value, screen(at), unit_step, tokens.accent);
                overlap_index.preview(ui, camera, rect, drawing, *turns, false, at);
            }
        }
        if let Some(Tool::Wire { start, .. }) = tool {
            wires::preview(&painter, rect, project, camera, start, draft, anchor, auto_heal, revision);
            if matches!(popup.inner, Some(ToolAction::Cancel)) {
                return Some(Click::CancelWire);
            }
        }
        return popup.inner.map(|action| Click::ToolAction(action, ui.ctx().clone()));
    }

    // A part dragged from the browser previews under the pointer and places where it's released.
    // hover_pos is absent while another widget owns the drag. Read the pointer directly and
    // transform it into this canvas layer, matching the coordinates used by camera snapping.
    let drag_pointer = ui
        .input(|i| i.pointer.hover_pos())
        .map(|pointer| ui.ctx().layer_transform_from_global(response.layer_id).map_or(pointer, |from_global| from_global * pointer));
    if input_enabled
        && let Some(drag) = response.dnd_hover_payload::<PartDrag>()
        && let Some(pointer) = drag_pointer
    {
        let point = wires::symbol_position(project, drag.placement.definition.default_unit(), 0, camera, rect, pointer, draft, revision);
        if valid_position(point) {
            paint_template(&painter, &drag.placement.definition, &drag.placement.value, screen(point), unit_step, 0, tokens.accent);
            overlap_index.preview(ui, camera, rect, drag.placement.definition.default_unit(), 0, false, point);
        }
    }
    if let Some(drag) = response.dnd_release_payload::<PartDrag>()
        && let Some(pointer) = drag_pointer
    {
        let point = wires::symbol_position(project, drag.placement.definition.default_unit(), 0, camera, rect, pointer, draft, revision);
        return (input_enabled && valid_position(point)).then(|| Click::Drop(PartDrag::clone(&drag), point));
    }

    if let Some(click) =
        crate::net_labels::input(ui, &response, rect, camera, project, tool.as_ref(), input_enabled, selected, label_position, filters)
    {
        return click;
    }
    if let Some(click) = crate::hierarchy::input(ui, &response, project, camera, rect, tool.as_ref(), input_enabled, selected_sheet, filters) {
        return click;
    }
    // Handle active rectangles before context menus: a secondary drag consumes its release.
    let lost_focus = ui.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))));
    if matches!(tool, Some(Tool::Field { .. })) {
        if lost_focus || response.secondary_clicked() {
            return Some(Click::FinishField(None));
        }
        if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
            return Some(Click::FinishField(field_destination.filter(|_| input_enabled && response.contains_pointer())));
        }
        return None;
    }
    if let Some(Tool::BoxSelect { drag, .. }) = &tool {
        if lost_focus {
            return Some(Click::SelectItems(drag.base.clone()));
        }
        let pointer = response.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.interact_pos()));
        if let Some(pointer) = pointer {
            let hits = selection::boxed(ui, project, camera, rect, drag.start, pointer, filters);
            pcbcraft_chrome::selection::marquee(&painter, Rect::from_two_pos(drag.start, pointer), pointer.x < drag.start.x, hits.len());
            if ui.input(|i| i.pointer.button_released(drag.button)) {
                let valid = input_enabled && response.contains_pointer();
                return Some(Click::SelectItems(if valid { drag.base.combine(&hits, drag.add, drag.toggle) } else { drag.base.clone() }));
            }
        }
        return None;
    }
    if let Some(Tool::Reshape { drag, .. }) = &tool {
        if lost_focus || response.secondary_clicked() {
            return Some(Click::FinishReshape(None));
        }
        if if drag.pickup {
            response.clicked_by(egui::PointerButton::Primary)
        } else {
            ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary))
        } {
            return Some(Click::FinishReshape(reshape_target.filter(|_| input_enabled && response.contains_pointer() && group_preview.is_some())));
        }
        return None;
    }
    if let Some(Tool::MoveItems { pickup, .. }) = &tool {
        if lost_focus || response.secondary_clicked() {
            return Some(Click::FinishItems(None));
        }
        if if *pickup {
            response.clicked_by(egui::PointerButton::Primary)
        } else {
            ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary))
        } {
            return Some(Click::FinishItems(group_delta.filter(|_| input_enabled && response.contains_pointer() && group_preview.is_some())));
        }
        return None;
    }
    // Dragging a wire-end mark stretches its attached routes through the existing junction
    // edit. A pin mark stays with its symbol; drawing tools retain their own input priority.
    if input_enabled
        && !lost_focus
        && tool.is_none()
        && filters.no_connects
        && selected.len() <= 1
        && !ui.input(|i| i.modifiers.shift || i.modifiers.ctrl || i.modifiers.command)
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && let Some(pcbcraft_engine::no_connect::Attachment::WireEnd(marker)) = crate::no_connect::marker_at(project, camera, rect, origin)
        && let Some(drag) = wires::marker_drag(project, marker, camera.snap(origin, rect))
    {
        return Some(Click::GrabReshape(drag));
    }
    // Endpoint and corner handles use their own hit area before the narrow stroke picker.
    // A ready wire tool allows a deliberate handle drag; a plain click still starts a wire.
    if input_enabled
        && !lost_focus
        && matches!(tool, None | Some(Tool::Wire { start: None, .. }))
        && selected.len() <= 1
        && !(filters.no_connects
            && ui.input(|i| i.pointer.press_origin()).is_some_and(|p| crate::no_connect::marker_at(project, camera, rect, p).is_some()))
        && !whole_wire_drag
        && filters.wires
        && !ui.input(|i| i.modifiers.shift || i.modifiers.ctrl || i.modifiers.command)
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && (!filters.symbols || fields::at(ui, project, camera, rect, origin).is_none())
    {
        let selected_drag = selected.single_wire().and_then(|wire| wires::reshape_at(project, camera, rect, wire, origin, camera.snap(origin, rect)));
        let drag = match selected_drag {
            Some(drag) => Some(drag),
            None => wires::handle_at(project, camera, rect, origin, camera.snap(origin, rect)).ok()?,
        };
        if let Some(drag) = drag {
            return Some(Click::GrabReshape(drag));
        }
    }
    if input_enabled
        && !lost_focus
        && tool.is_none()
        && egui::DragAndDrop::payload::<PartDrag>(ui.ctx()).is_none()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && (response.drag_started_by(egui::PointerButton::Primary) || response.drag_started_by(egui::PointerButton::Secondary))
    {
        let right = response.drag_started_by(egui::PointerButton::Secondary);
        let modifiers = ui.input(|i| i.modifiers);
        if !right
            && !modifiers.shift
            && !modifiers.ctrl
            && !modifiers.command
            && filters.symbols
            && let Some(field) = fields::at(ui, project, camera, rect, origin)
        {
            return Some(Click::GrabField(field));
        }
        // A plain drag on the one selected wire reshapes it instead of moving it whole.
        if !right
            && !whole_wire_drag
            && !modifiers.shift
            && !modifiers.ctrl
            && !modifiers.command
            && let Some(wire) = selected.single_wire()
            && let Some(drag) = wires::reshape_at(project, camera, rect, wire, origin, camera.snap(origin, rect))
        {
            return Some(Click::GrabReshape(drag));
        }
        let hit = selection::at(ui, project, camera, rect, origin, filters);
        if right || hit.is_empty() || modifiers.shift || modifiers.ctrl || modifiers.command {
            return Some(Click::StartBox(BoxDrag {
                start: origin,
                base: selected.clone(),
                add: modifiers.shift,
                toggle: modifiers.ctrl || modifiers.command,
                button: if right { egui::PointerButton::Secondary } else { egui::PointerButton::Primary },
            }));
        }
        if !hit.no_connects.is_empty() && selected.symbols.is_empty() && selected.wires.is_empty() && selected.net_labels.is_empty() {
            return Some(Click::SelectItems(selected.combine(&hit, false, false)));
        }
        if hit.is_subset(selected) && (selected.len() > 1 || whole_wire_drag) {
            return Some(Click::MoveItems { items: selected.clone(), origin: camera.snap(origin, rect), pickup: false, disconnect: false });
        }
        if let Some(wire) = hit.single_wire()
            && let Some(drag) = wires::reshape_at(project, camera, rect, wire, origin, camera.snap(origin, rect))
        {
            return Some(Click::GrabReshape(drag));
        }
        return grab_symbol(project, camera, rect, origin, false, selected);
    }

    if tool.is_none() && (input_enabled || response.context_menu_opened()) {
        // A right-click names its symbol this frame; afterwards the menu follows the selection.
        let pressed = response.secondary_clicked().then(|| response.interact_pointer_pos()).flatten();
        let choices_key = response.id.with("symbol_choices");
        if let Some(pointer) = pressed {
            let hits = if filters.symbols {
                symbols_at(project, camera, rect, pointer)
                    .into_iter()
                    .map(|(symbol, _)| (symbol.id(), format!("Select {}", symbol.display_reference(&project.reference(symbol)))))
                    .collect()
            } else {
                Vec::new()
            };
            ui.data_mut(|data| data.insert_temp(choices_key, Arc::new(SymbolChoices { target: project.sheet_target(), revision, entries: hits })));
        } else if !response.context_menu_opened() {
            ui.data_mut(|data| data.remove::<Arc<SymbolChoices>>(choices_key));
        }
        let captured_choices = ui
            .data(|data| data.get_temp::<Arc<SymbolChoices>>(choices_key))
            .filter(|choices| choices.target == project.sheet_target() && choices.revision == revision && filters.symbols);
        let choices = captured_choices.as_ref().map_or(&[][..], |choices| choices.entries.as_slice());
        let pin_key = response.id.with("no_connect_pin");
        if let Some(pointer) = pressed {
            let pin = crate::no_connect::target_at(project, camera, rect, pointer, selected).filter(|pick| {
                if project.has_no_connect_attachment(&pick.target) {
                    return filters.no_connects;
                }
                match pick.target {
                    pcbcraft_engine::no_connect::Attachment::Pin(_) => filters.symbols,
                    pcbcraft_engine::no_connect::Attachment::WireEnd(_) => filters.wires,
                }
            });
            ui.data_mut(|data| {
                data.remove::<(pcbcraft_engine::SheetTarget, u64, pcbcraft_engine::no_connect::Attachment, bool)>(pin_key);
                if let Some(pick) = pin {
                    let marked = project.has_no_connect_attachment(&pick.target);
                    data.insert_temp(pin_key, (project.sheet_target(), revision, pick.target, marked));
                }
            });
        }
        let pin = ui
            .data(|data| data.get_temp::<(pcbcraft_engine::SheetTarget, u64, pcbcraft_engine::no_connect::Attachment, bool)>(pin_key))
            .filter(|(sheet, captured_revision, target, marked)| {
                *sheet == project.sheet_target()
                    && *captured_revision == revision
                    && if *marked {
                        filters.no_connects
                    } else {
                        match target {
                            pcbcraft_engine::no_connect::Attachment::Pin(_) => filters.symbols,
                            pcbcraft_engine::no_connect::Attachment::WireEnd(_) => filters.wires,
                        }
                    }
            });
        let mut target = match pressed {
            Some(pointer) => filters.symbols.then(|| symbol_at(project, camera, rect, pointer)).flatten().map(|(symbol, _)| symbol.id()),
            None => selected.single_symbol().filter(|_| filters.symbols).map(str::to_owned),
        };
        if let Some(current) = selected.single_symbol().filter(|current| choices.iter().any(|(id, _)| id == current)) {
            target = Some(current.to_owned());
        }
        let mut group = selected.len() > 1
            && (pressed.is_none()
                || pressed.is_some_and(|p| {
                    let hit = selection::at(ui, project, camera, rect, p, filters);
                    !hit.is_empty() && hit.is_subset(selected)
                }));
        // Capture the menu's hit once. An empty-space menu must not turn into the selected
        // symbol's menu on the next frame, or clear that selection just to keep its rows stable.
        let context_target = response.id.with("context_target");
        if pressed.is_some() {
            ui.data_mut(|data| data.insert_temp(context_target, (target.clone(), group)));
        } else if response.context_menu_opened()
            && let Some(captured) = ui.data(|data| data.get_temp::<(Option<String>, bool)>(context_target))
        {
            (target, group) = captured;
        }
        // A wire's menu keeps the leg it was opened on until it closes.
        let leg_id = response.id.with("wire_menu_leg");
        let origin_id = response.id.with("wire_menu_origin");
        if let Some(pointer) = pressed {
            ui.data_mut(|data| data.insert_temp(origin_id, pointer));
            let leg = (filters.wires && target.is_none() && !group).then(|| wires::leg_at(project, camera, rect, pointer, revision)).flatten();
            ui.data_mut(|data| match leg {
                Some(leg) => {
                    data.insert_temp(leg_id, leg);
                }
                None => data.remove::<wires::LegTarget>(leg_id),
            });
        }
        let leg = ui.data(|data| data.get_temp::<wires::LegTarget>(leg_id)).filter(|_| filters.wires && target.is_none() && !group);
        let grab = leg.as_ref().filter(|leg| leg.revision == revision).and_then(|leg| {
            let pointer = ui.data(|data| data.get_temp::<Pos2>(origin_id))?;
            let mut drag = wires::reshape_at(project, camera, rect, &leg.wire, pointer, camera.snap(pointer, rect))?;
            drag.pickup = true;
            Some(drag)
        });
        let picked = menu::context(&response, |ui| {
            if let Some((_, _, target, marked)) = &pin {
                if (!marked || !group)
                    && MenuItem::new(if *marked { "Delete" } else { "Mark no connect" }).shortcut(if *marked { "Del" } else { "Q" }).show(ui)
                {
                    return Some(Click::NoConnect(target.clone(), !marked));
                }
                if *marked && !group {
                    return None;
                }
                menu::separator(ui);
            }
            if choices.len() > 1 {
                if let Some(chosen) = MenuItem::new("Select symbol")
                    .submenu(ui, |ui| {
                        let tokens = Tokens::get();
                        let height = tokens.control_height + tokens.space_2;
                        egui::ScrollArea::vertical()
                            .max_height(height * 8.)
                            .show_rows(ui, height, choices.len(), |ui, range| {
                                choices[range].iter().find_map(|(id, label)| {
                                    MenuItem::new(label)
                                        .choice(selected.single_symbol() == Some(id.as_str()))
                                        .show(ui)
                                        .then(|| Click::Select(Some(id.clone())))
                                })
                            })
                            .inner
                    })
                    .and_then(|shown| shown.inner)
                {
                    return Some(chosen);
                }
                menu::separator(ui);
            }
            if let Some(action) = tool_menu::clipboard(ui, !selected.is_empty() || selected_sheet.is_some()) {
                return Some(Click::ToolAction(action, ui.ctx().clone()));
            }
            menu::separator(ui);
            if group {
                return MenuItem::new("Delete selected items").shortcut("Del").show(ui).then_some(Click::DeleteItems);
            }
            if let Some(leg) = leg.clone() {
                return wire_menu(ui, &painter, rect, camera, leg, grab);
            }
            let Some(id) = target.clone() else {
                return tool_menu::idle(ui, camera.snap_nm(), unit).map(|action| Click::ToolAction(action, ui.ctx().clone()));
            };
            let rows = [
                ("Grab (keep connected)", "G", SymbolAction::Grab),
                ("Disconnect and Move", "M", SymbolAction::DisconnectMove),
                ("Rotate", "R", SymbolAction::Rotate),
                ("Mirror Horizontally", "X", SymbolAction::Mirror(pcbcraft_engine::schematic::MirrorAxis::Horizontal)),
                ("Mirror Vertically", "Y", SymbolAction::Mirror(pcbcraft_engine::schematic::MirrorAxis::Vertical)),
                ("Edit Fields…", "E", SymbolAction::EditFields),
                ("Assign Footprint…", "", SymbolAction::AssignFootprint),
                ("Delete", "Del", SymbolAction::Delete),
            ];
            rows.into_iter().find_map(|(label, key, action)| {
                MenuItem::new(label)
                    .shortcut(key)
                    .enabled(action != SymbolAction::AssignFootprint || project.board_target().is_some())
                    .show(ui)
                    .then(|| Click::Edit(action, id.clone()))
            })
        });
        if let Some(click) = picked.and_then(|shown| shown.inner) {
            return Some(click);
        }
        if pressed.is_some() && !group {
            if let Some((_, _, target, true)) = &pin {
                return Some(Click::SelectItems(Selection::one_no_connect(target.clone())));
            }
            if let Some(leg) = leg.filter(|_| target.is_none()) {
                return Some(Click::SelectLeg(leg));
            }
            if target.is_some() {
                return Some(Click::Select(target));
            }
        }
    }

    let symbol_move = matches!(tool, Some(Tool::Move { .. }));
    let pickup = matches!(tool, Some(Tool::Move { pickup: true, .. }));
    if symbol_move && (lost_focus || response.secondary_clicked()) {
        return Some(Click::FinishMove(None));
    }
    if (pickup && response.clicked_by(egui::PointerButton::Primary))
        || (symbol_move && !pickup && response.drag_stopped_by(egui::PointerButton::Primary))
    {
        return Some(Click::FinishMove(
            moved.map(|(_, point)| point).filter(|_| input_enabled && response.contains_pointer() && group_preview.is_some()),
        ));
    }

    if matches!(tool, Some(Tool::NoConnect { .. })) && lost_focus {
        return Some(Click::CancelNoConnect);
    }
    if !input_enabled || (response.dragged() && !matches!(tool, Some(Tool::PlaceSymbol { .. }))) {
        return None;
    }

    if !ui.ctx().egui_wants_keyboard_input()
        && response.contains_pointer()
        && tool.is_none()
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Q))
    {
        let target =
            response.hover_pos().and_then(|pointer| crate::no_connect::target_at(project, camera, rect, pointer, selected)).map(|pick| pick.target);
        return Some(target.map_or(Click::BeginNoConnect, |target| Click::NoConnect(target, true)));
    }
    if matches!(tool, Some(Tool::NoConnect { .. })) {
        if response.secondary_clicked() {
            return Some(Click::CancelNoConnect);
        }
        // Accessible marker labels may own hover without taking canvas editing input.
        // Keep the tool cursor over those labels while respecting panels on higher layers.
        let target = ui
            .input(|input| input.pointer.hover_pos())
            .filter(|_| ui.rect_contains_pointer(rect))
            .and_then(|pointer| crate::no_connect::preview(&painter, project, camera, rect, pointer, selected));
        return response.clicked().then(|| target.map_or(Click::InvalidNoConnect, |target| Click::NoConnect(target, true)));
    }
    if tool.is_none()
        && response.hovered()
        && !ui.ctx().egui_wants_keyboard_input()
        && !ui.input(|input| input.modifiers.shift || input.modifiers.ctrl || input.modifiers.command)
        && let Some(pointer) = response.hover_pos()
        && !(filters.no_connects && crate::no_connect::marker_at(project, camera, rect, pointer).is_some())
        && (!filters.symbols || fields::at(ui, project, camera, rect, pointer).is_none())
        && let Some(anchor) = wires::node_at(project, camera, rect, pointer, draft, revision)
        && let Some(point) = anchor.position(project)
        && (!filters.symbols
            || symbol_at(project, camera, rect, pointer).is_none()
            || camera.screen(point, rect).distance(pointer) <= theme::SCHEMATIC_PIN_RADIUS.min(camera.unit_step() / 2.))
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        pcbcraft_chrome::cursor::wire(&painter, pointer, screen(point), true);
    }
    if matches!(tool, None | Some(Tool::Wire { start: None, .. }))
        && response.contains_pointer()
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::G))
        && let Some(pointer) = response.hover_pos()
    {
        if selected.len() > 1 {
            return Some(Click::MoveItems { items: selected.clone(), origin: camera.snap(pointer, rect), pickup: true, disconnect: false });
        }
        if filters.symbols && fields::at(ui, project, camera, rect, pointer).is_some() {
            return None;
        }
        if filters.wires {
            let selected_drag =
                selected.single_wire().and_then(|wire| wires::reshape_at(project, camera, rect, wire, pointer, camera.snap(pointer, rect)));
            let mut drag = match selected_drag {
                Some(drag) => Some(drag),
                None => wires::handle_at(project, camera, rect, pointer, camera.snap(pointer, rect)).ok()?,
            }
            .or_else(|| {
                let hit = selection::at(ui, project, camera, rect, pointer, filters);
                wires::reshape_at(project, camera, rect, hit.single_wire()?, pointer, camera.snap(pointer, rect))
            });
            if let Some(drag) = &mut drag {
                drag.pickup = true;
            }
            if let Some(drag) = drag {
                return Some(Click::GrabReshape(drag));
            }
        }
        if filters.symbols && fields::at(ui, project, camera, rect, pointer).is_none() {
            return grab_symbol(project, camera, rect, pointer, true, selected);
        }
    }
    // Field editing and body properties precede click-to-wire for enabled symbols.
    // Explicit placement/wiring tools retain their own double click.
    if tool.is_none()
        && response.double_clicked_by(egui::PointerButton::Primary)
        && filters.symbols
        && !ui.input(|i| i.modifiers.shift || i.modifiers.ctrl || i.modifiers.command)
    {
        let pointer = response.interact_pointer_pos()?;
        if let Some(field) = fields::at(ui, project, camera, rect, pointer) {
            return Some(Click::EditField(field));
        }
        if let Some((symbol, _)) = symbol_at(project, camera, rect, pointer) {
            return Some(Click::Edit(SymbolAction::EditFields, symbol.id()));
        }
    }
    if tool.is_none() && response.clicked_by(egui::PointerButton::Primary) {
        let pointer = response.interact_pointer_pos()?;
        if let Some(target) = filters.no_connects.then(|| crate::no_connect::marker_at(project, camera, rect, pointer)).flatten() {
            let modifiers = ui.input(|i| i.modifiers);
            return Some(Click::SelectItems(selected.combine(
                &Selection::one_no_connect(target),
                modifiers.shift,
                modifiers.ctrl || modifiers.command,
            )));
        }
        if filters.symbols
            && let Some(field) = fields::at(ui, project, camera, rect, pointer)
        {
            let modifiers = ui.input(|i| i.modifiers);
            return Some(Click::SelectItems(selected.combine(
                &Selection::one_symbol(field.symbol),
                modifiers.shift,
                modifiers.ctrl || modifiers.command,
            )));
        }
        let hit = selection::at(ui, project, camera, rect, pointer, filters);
        let modifiers = ui.input(|i| i.modifiers);
        if !hit.no_connects.is_empty() {
            return Some(Click::SelectItems(selected.combine(&hit, modifiers.shift, modifiers.ctrl || modifiers.command)));
        }
        if !modifiers.shift
            && !modifiers.ctrl
            && !modifiers.command
            && let Some(anchor) = wires::node_at(project, camera, rect, pointer, draft, revision)
            && (hit.single_symbol().is_none()
                || anchor
                    .position(project)
                    .is_some_and(|at| camera.screen(at, rect).distance(pointer) <= theme::SCHEMATIC_PIN_RADIUS.min(camera.unit_step() / 2.)))
        {
            return Some(Click::StartWire(anchor));
        }
        if !modifiers.shift
            && !modifiers.ctrl
            && !modifiers.command
            && hit.single_wire().is_some()
            && let Some(leg) = wires::leg_at(project, camera, rect, pointer, revision)
        {
            return Some(Click::SelectLeg(leg));
        }
        return Some(Click::SelectItems(selected.combine(&hit, modifiers.shift, modifiers.ctrl || modifiers.command)));
    }
    match tool? {
        Tool::PlaceSymbol { definition, unit, value, turns, .. } => {
            placement_preview(ui, rect, response, camera, tokens, &definition, &unit, &value, turns, project, draft, revision, &overlap_index)
                .map(Click::Place)
        }
        Tool::Wire { start, .. } => wires::pick(&painter, rect, &response, project, camera, start, draft, auto_heal, revision)
            .map(|(anchor, finish)| Click::WirePoint(anchor, finish)),
        Tool::NoConnect { .. }
        | Tool::Field { .. }
        | Tool::NetLabel { .. }
        | Tool::SheetGesture { .. }
        | Tool::Hierarchy { .. }
        | Tool::Move { .. }
        | Tool::BoxSelect { .. }
        | Tool::MoveItems { .. }
        | Tool::Reshape { .. } => None,
    }
}

/// The wire context menu offers Slice (when the leg
/// has a grid point to split at), Break (when the leg has any attachment), Delete Segment
/// and Delete Wire, plus Select Wire for whole-route selection. The hovered row previews
/// its effect on the canvas.
fn wire_menu(ui: &mut Ui, painter: &Painter, rect: Rect, camera: &Camera, leg: wires::LegTarget, grab: Option<wires::ReshapeDrag>) -> Option<Click> {
    if MenuItem::new("Grab").shortcut("G").enabled(grab.is_some()).show(ui) {
        return grab.map(Click::GrabReshape);
    }
    let rows = [
        (MenuItem::new("Slice").enabled(leg.split.is_some()), WireAction::Slice),
        (MenuItem::new("Break").enabled(leg.breakable), WireAction::Break),
        (MenuItem::new("Heal Wire"), WireAction::Heal),
        (MenuItem::new("Delete Segment").shortcut("Del"), WireAction::DeleteSegment),
        (MenuItem::new("Delete Wire"), WireAction::Delete),
        (MenuItem::new("Select Wire"), WireAction::SelectWire),
    ];
    let mut hovered = None;
    let mut picked = None;
    for (row, action) in rows {
        let response = row.show_response(ui);
        if response.hovered() && row.enabled {
            hovered = Some(action);
        }
        if response.clicked() {
            picked = Some(action);
        }
    }
    wires::paint_leg_preview(painter, rect, camera, &leg, hovered);
    picked.map(|action| Click::WireEdit(action, leg))
}

/// Preserve the chosen stacked symbol under the press, otherwise pick the nearest body.
/// The grab offset remains relative to that symbol so selection clarification also works for moves.
fn grab_symbol(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, pickup: bool, selected: &Selection) -> Option<Click> {
    let hits = symbols_at(project, camera, rect, pointer);
    let (symbol, center) =
        hits.iter().find(|(symbol, _)| selected.single_symbol() == Some(symbol.id().as_str())).or_else(|| hits.first()).copied()?;
    Some(Click::Grab(symbol.clone(), center - pointer, pickup))
}

/// The symbol whose body is under the pointer, nearest center first, with its screen center.
pub(super) fn symbol_at<'a>(project: &'a Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<(&'a SymbolPlacement, Pos2)> {
    project
        .symbols()
        .iter()
        .rev()
        .map(|symbol| (symbol, camera.screen(symbol.position, rect)))
        .filter(|(symbol, center)| symbol_contains(symbol, *center, camera.unit_step(), pointer))
        .min_by(|(_, a), (_, b)| a.distance_sq(pointer).total_cmp(&b.distance_sq(pointer)))
}

/// Test the same drawn body area for ordinary picking and selection clarification.
fn symbol_contains(symbol: &SymbolPlacement, center: Pos2, step: f32, pointer: Pos2) -> bool {
    body(symbol.unit_definition(), symbol.turns(), symbol.is_mirrored())
        .translate(center.to_vec2() / step)
        .contains((pointer.to_vec2() / step).to_pos2())
}

/// Return all bodies under the pointer in nearest-center order, newest first for ties.
fn symbols_at<'a>(project: &'a Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Vec<(&'a SymbolPlacement, Pos2)> {
    let mut hits: Vec<_> = project
        .symbols()
        .iter()
        .rev()
        .map(|symbol| (symbol, camera.screen(symbol.position, rect)))
        .filter(|(symbol, center)| symbol_contains(symbol, *center, camera.unit_step(), pointer))
        .collect();
    hits.sort_by(|(_, a), (_, b)| a.distance_sq(pointer).total_cmp(&b.distance_sq(pointer)));
    hits
}

/// Transform an offset in grid steps exactly as the engine does: reflect horizontally first,
/// then turn counterclockwise.
fn turn(x: f32, y: f32, turns: u8, mirrored: bool) -> (f32, f32) {
    let x = if mirrored { -x } else { x };
    match turns % 4 {
        0 => (x, y),
        1 => (y, -x),
        2 => (-x, -y),
        _ => (-y, x),
    }
}

/// A rectangle in grid steps after the same mirror and quarter turns as the drawing.
fn turn_rect(rect: Rect, turns: u8, mirrored: bool) -> Rect {
    let rect = if mirrored { Rect::from_min_max(pos2(-rect.max.x, rect.min.y), pos2(-rect.min.x, rect.max.y)) } else { rect };
    let (ax, ay) = turn(rect.min.x, rect.min.y, turns, false);
    let (bx, by) = turn(rect.max.x, rect.max.y, turns, false);
    Rect::from_two_pos(pos2(ax, ay), pos2(bx, by))
}

/// Keep previews inside the same coordinate bounds the command validates.
fn valid_position(point: Point) -> bool {
    (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.x_nm) && (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.y_nm)
}

/// Place on the primary press so a fast release or incidental motion cannot discard the click.
/// Explicit placement owns this press; the following release never places a second symbol.
#[allow(clippy::too_many_arguments)]
fn placement_preview(
    ui: &mut Ui,
    rect: Rect,
    response: Response,
    camera: &Camera,
    tokens: Tokens,
    definition: &Arc<SymbolDefinition>,
    unit: &str,
    value: &str,
    turns: u8,
    project: &Project,
    draft: &Draft,
    revision: u64,
    overlaps: &overlaps::Index,
) -> Option<Point> {
    let screen = |point| camera.screen(point, rect);
    let pointer = response.hover_pos().or_else(|| response.interact_pointer_pos())?;
    let point = wires::symbol_position(project, definition.unit(unit)?, turns, camera, rect, pointer, draft, revision);
    if !valid_position(point) {
        return None;
    }
    let preview = SymbolPlacement::preview_unit(definition.clone(), unit, turns, definition.defaults.clone().unwrap_or_default().field_layout)?;
    paint_unit_template(&ui.painter_at(rect), &preview, value, screen(point), camera.unit_step(), tokens.accent);
    overlaps.preview(ui, camera, rect, preview.unit_definition(), turns, false, point);
    let pressed = response.contains_pointer() && response.ctx.input(|input| input.pointer.primary_pressed());
    if pressed && response.ctx.input(|input| input.pointer.primary_down()) {
        response.ctx.data_mut(|data| data.insert_temp(response.id.with("placed_on_press"), true));
    }
    response.on_hover_cursor(egui::CursorIcon::Crosshair);
    pressed.then_some(point)
}

/// Preview ordinary drawing and instance fields from the same native defaults as placement.
fn paint_template(painter: &Painter, definition: &Arc<SymbolDefinition>, value: &str, center: Pos2, step: f32, turns: u8, color: egui::Color32) {
    let defaults = definition.defaults.clone().unwrap_or_default();
    let symbol = SymbolPlacement::preview(definition.clone(), turns, defaults.field_layout);
    paint_unit_template(painter, &symbol, value, center, step, color);
}

/// Preview the selected drawing with the same fields and geometry as its committed placement.
fn paint_unit_template(painter: &Painter, symbol: &SymbolPlacement, value: &str, center: Pos2, step: f32, color: egui::Color32) {
    paint_symbol(
        painter,
        symbol.unit_definition(),
        center,
        step,
        symbol.turns(),
        symbol.is_mirrored(),
        Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color),
    );
    let defaults = symbol.definition.defaults.clone().unwrap_or_default();
    fields::paint_labels(painter, symbol, fields::Content { reference: "", value, metadata: &defaults.metadata }, center, step, color);
}

/// One schematic grid step in nanometers, for converting library drawings to grid steps.
const GRID: f32 = GRID_NM as f32;

/// A library point in grid steps from the symbol's center, before rotation.
fn grid(point: Point) -> (f32, f32) {
    (point.x_nm as f32 / GRID, point.y_nm as f32 / GRID)
}

/// [`grid`] as a position, for bounds.
fn grid_pos(point: Point) -> Pos2 {
    let (x, y) = grid(point);
    pos2(x, y)
}

/// The area a symbol's drawing covers around its center, in grid steps, after its turns: its
/// pins and every stroke. Fit uses it so every part stays in view.
pub(super) fn grid_bounds(definition: &pcbcraft_engine::schematic::SymbolUnit, turns: u8, mirrored: bool) -> Rect {
    turn_rect(unturned_bounds(definition), turns, mirrored)
}

/// [`grid_bounds`] before rotation.
fn unturned_bounds(definition: &pcbcraft_engine::schematic::SymbolUnit) -> Rect {
    let pins = definition.pins().iter().map(|pin| Rect::from_pos(grid_pos(pin.offset)));
    pins.fold(shape_bounds(definition.graphics().iter()), Rect::union)
}

/// The bounds of a symbol's strokes alone, in grid steps before rotation, or nothing when it
/// has none.
fn drawing_bounds(definition: &pcbcraft_engine::schematic::SymbolUnit) -> Rect {
    shape_bounds(definition.drawing.iter())
}

/// Measure a chosen drawing set: Fit includes generated pin ink; body selection uses native drawing only.
fn shape_bounds<'a>(shapes: impl Iterator<Item = &'a Shape>) -> Rect {
    let point = |point: &Point| Rect::from_pos(grid_pos(*point));
    shapes.fold(Rect::NOTHING, |bounds, shape| {
        bounds.union(match shape {
            Shape::Graphic(graphic) => graphic.paths().iter().flatten().map(point).fold(Rect::NOTHING, Rect::union),
            Shape::Line(from, to) | Shape::Rect(from, to) => point(from).union(point(to)),
            Shape::Polyline(points) => points.iter().map(point).fold(Rect::NOTHING, Rect::union),
            Shape::Circle(center, radius) => point(center).expand(*radius as f32 / GRID),
            Shape::Text(at, _, _) => point(at),
        })
    })
}

/// Where an upright label sits on its anchor after the symbol turns. Half a turn swaps start
/// and end so a label inside a body edge stays inside; a quarter turn centers it.
fn text_anchor(align: TextAlign, turns: u8, mirrored: bool) -> Align2 {
    let align = if mirrored {
        match align {
            TextAlign::Start => TextAlign::End,
            TextAlign::End => TextAlign::Start,
            TextAlign::Middle => TextAlign::Middle,
        }
    } else {
        align
    };
    match (align, turns % 4) {
        (TextAlign::Middle, _) | (_, 1 | 3) => Align2::CENTER_CENTER,
        (TextAlign::Start, 0) | (TextAlign::End, 2) => Align2::LEFT_CENTER,
        _ => Align2::RIGHT_CENTER,
    }
}

/// Use the same zoom cutoff and clamped font for native drawing text and its overlap bounds.
fn drawing_text_font(step: f32) -> Option<FontId> {
    (step >= theme::SCHEMATIC_MIN_GRID_SPACING)
        .then(|| FontId::proportional((step * 1.1).clamp(Tokens::get().font_small * 0.8, Tokens::get().font_button)))
}

/// The grabbable area of a symbol in grid steps around its center, after its turns: a 5 by 4
/// step box at the center, grown to the drawing's strokes so a large part can be picked by its
/// body. Pins stay outside it unless a stroke reaches them.
fn body(definition: &pcbcraft_engine::schematic::SymbolUnit, turns: u8, mirrored: bool) -> Rect {
    let core = Rect::from_center_size(Pos2::ZERO, vec2(5.0, 4.0));
    let drawing = drawing_bounds(definition);
    let unturned = if drawing.is_positive() { core.union(drawing.shrink2(vec2(1.0, 0.0)).intersect(drawing)) } else { core };
    turn_rect(unturned, turns, mirrored)
}

/// Draw the symbol's library strokes around its center, with one grid step of `step` points,
/// turned with the symbol.
/// Part instance fields are drawn separately so their layout never changes the body or pins.
fn paint_symbol(
    painter: &Painter,
    definition: &pcbcraft_engine::schematic::SymbolUnit,
    center: Pos2,
    step: f32,
    turns: u8,
    mirrored: bool,
    stroke: Stroke,
) {
    let color = stroke.color;
    let xy = |point: &Point| {
        let (x, y) = grid(*point);
        let (x, y) = turn(x, y, turns, mirrored);
        pos2(center.x + x * step, center.y + y * step)
    };
    for shape in &definition.graphics() {
        match shape {
            Shape::Graphic(graphic) => {
                if matches!(graphic.geometry, pcbcraft_engine::library_graphics::Geometry::Text { .. }) {
                    if step < theme::SCHEMATIC_MIN_GRID_SPACING {
                        continue;
                    }
                    let text = crate::definition_graphics::symbol_text(graphic, turns, mirrored);
                    crate::definition_graphics::paint(
                        painter,
                        &text,
                        |p| center + vec2(p.x_nm as f32, p.y_nm as f32) * (step / GRID),
                        step / GRID,
                        color,
                        Tokens::get().canvas,
                    );
                } else {
                    crate::definition_graphics::paint(painter, graphic, |p| xy(&p), step / GRID, color, Tokens::get().canvas);
                }
            }
            Shape::Line(from, to) => {
                painter.line_segment([xy(from), xy(to)], stroke);
            }
            Shape::Polyline(points) => {
                painter.add(egui::Shape::line(points.iter().map(xy).collect(), stroke));
            }
            Shape::Rect(min, max) => {
                painter.rect_stroke(Rect::from_two_pos(xy(min), xy(max)), 0, stroke, egui::StrokeKind::Middle);
            }
            Shape::Circle(at, radius) => {
                painter.circle_stroke(xy(at), *radius as f32 / GRID * step, stroke);
            }
            Shape::Text(at, text, align) => {
                if let Some(font) = drawing_text_font(step) {
                    painter.text(xy(at), text_anchor(*align, turns, mirrored), text, font, color);
                }
            }
        }
    }
}
