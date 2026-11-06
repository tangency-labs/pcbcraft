use std::sync::Arc;

mod camera;
mod wires;

#[cfg(test)]
mod tests;

pub(crate) use camera::Camera;
pub(crate) use wires::Draft;

use crate::preferences::Navigation;
use egui::{Align2, FontId, Key, Modifiers, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};
use pcbcraft_chrome::{MenuItem, Tokens, menu, theme};
use pcbcraft_engine::wire::Anchor;
use pcbcraft_engine::{
    Project,
    schematic::{GRID_NM, MAX_CENTER_NM, Point, Shape, SymbolDefinition, SymbolPlacement, TextAlign},
};

/// The active canvas interaction, bound to the revision at which it began.
#[derive(Clone, Debug)]
pub(crate) enum Tool {
    PlaceSymbol { definition: Arc<SymbolDefinition>, revision: u64, turns: u8 },
    Wire { revision: u64, start: Option<Anchor> },
    Move { symbol: SymbolPlacement, offset: Vec2, revision: u64 },
}

impl Tool {
    /// Return the revision used to reject stale canvas actions.
    pub(crate) fn revision(&self) -> u64 {
        match self {
            Self::PlaceSymbol { revision, .. } | Self::Wire { revision, .. } | Self::Move { revision, .. } => *revision,
        }
    }
}

/// A placement, pin selection, or cancellation requested through the canvas.
pub(crate) enum Click {
    Place(Point),
    WirePoint(Anchor, bool),
    SelectWire(String),
    CancelWire,
    Grab(SymbolPlacement, Vec2),
    FinishMove(Option<Point>),
    /// Select a symbol by ID, or clear the selection after a click on empty canvas.
    Select(Option<String>),
    /// A row of the selected symbol's context menu.
    Edit(SymbolAction, String),
    /// A part dragged from the parts browser was released over the canvas at this snapped point.
    Drop(PartDrag, Point),
}

/// A browser drag bound to the project revision at which the gesture began.
#[derive(Clone, Debug)]
pub(crate) struct PartDrag {
    pub(crate) definition: Arc<SymbolDefinition>,
    pub(crate) revision: u64,
}

/// Draw a part's preview fitted inside `rect`, for the parts browser's thumbnails. The symbol
/// is drawn unrotated and without labels.
pub(crate) fn paint_preview(painter: &Painter, definition: &SymbolDefinition, rect: Rect, color: egui::Color32) {
    let bounds = grid_bounds(definition, 0);
    let step = (rect.width() / bounds.width().max(1.0)).min(rect.height() / bounds.height().max(1.0));
    let center = rect.center() - bounds.center().to_vec2() * step;
    paint_symbol(painter, definition, center, step, 0, color, None);
}

/// What the selected symbol's context menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SymbolAction {
    Rotate,
    EditFields,
    Delete,
}

/// Draw the schematic and its active preview, and report a requested tool action.
/// A primary drag with no active tool grabs the nearest symbol body. The move
/// preview ends on release, with no destination if released outside the canvas.
///
/// Return `Click::CancelWire` when Cancel or Escape dismisses an in-progress wire
/// through its context menu. An open wire menu can handle cancellation even
/// while canvas input is blocked.
///
/// With canvas input enabled and no drag in progress, a primary click during
/// placement returns `Click::Place` at a valid snapped point. In Wire mode,
/// primary clicks return `Click::WirePoint` for pins, junctions or snapped free points;
/// double-click or a queued Enter request marks the route complete.
/// A browser part dragged over the canvas previews at the snapped pointer position; releasing
/// it returns `Click::Drop`. Blocked input or an out-of-bounds position suppresses both.
/// Return `None` for a preview without a valid click, an inactive tool, or
/// blocked or dragged canvas input when the wire menu did not cancel.
///
/// With no tool, a primary click returns `Click::Select` for the symbol under it, or `None` in
/// it for empty canvas. Wire strokes return `Click::SelectWire` when no symbol is hit. A secondary click selects the symbol under it and opens its menu; a row
/// picked there returns `Click::Edit`. `selected` is drawn in the accent colour.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut Ui,
    project: &Project,
    camera: &mut Camera,
    tool: Option<Tool>,
    selected: Option<&str>,
    input_enabled: bool,
    navigation: Navigation,
    draft: &Draft,
    selected_wire: Option<&str>,
) -> Option<Click> {
    let rect = ui.available_rect_before_wrap();
    // Keep the canvas from taking keyboard focus away from editor shortcuts.
    let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Panel, true, "Schematic canvas"));
    let moving = matches!(tool, Some(Tool::Move { .. }));
    camera.navigate(&response, rect, project, input_enabled && !moving, navigation);

    let painter = ui.painter_at(rect);
    let tokens = Tokens::get();
    let screen = |point| camera.screen(point, rect);
    let origin = screen(Point { x_nm: 0, y_nm: 0 });
    let grid_step = camera.grid_step();
    paint_grid(&painter, rect, origin, tokens, grid_step);
    let moved = match &tool {
        Some(Tool::Move { symbol, offset, .. }) => response
            .interact_pointer_pos()
            .filter(|pointer| rect.contains(*pointer))
            .map(|pointer| (symbol.clone(), camera.snap(pointer + *offset, rect)))
            .filter(|(_, point)| valid_position(*point)),
        _ => None,
    };
    wires::paint(&painter, rect, project, camera, moved.clone(), selected_wire);
    let visible = rect.expand((grid_step * 5.0).max(theme::SCHEMATIC_FIT_MARGIN));
    for symbol in project.symbols() {
        let position = moved.as_ref().filter(|(moving, _)| moving.id() == symbol.id()).map_or(symbol.position, |(_, point)| *point);
        let center = screen(position);
        if visible.contains(center) {
            let reference = (grid_step >= theme::SCHEMATIC_MIN_GRID_SPACING).then(|| project.reference(symbol));
            let label = reference.as_deref().map(|reference| (reference, project.value(symbol)));
            let color = if selected == Some(symbol.id().as_str()) { tokens.accent } else { tokens.text };
            paint_symbol(&painter, &symbol.definition, center, grid_step, symbol.turns(), color, label);
        }
    }

    if let Some(Tool::Wire { start: Some(start), .. }) = tool
        && (input_enabled || response.context_menu_opened())
        && let Some(popup) = menu::context(&response, |ui| {
            let cancel = MenuItem::new("Cancel").shortcut("Esc").show(ui);
            let escape = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
            if escape {
                ui.close();
            }
            cancel || escape
        })
    {
        if popup.inner {
            return Some(Click::CancelWire);
        }
        let anchor = egui::Popup::position_of_id(ui.ctx(), egui::Popup::default_response_id(&response));
        wires::preview(&painter, rect, project, camera, Some(start), draft, anchor);
        return None;
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
        let point = camera.snap(pointer, rect);
        if valid_position(point) {
            paint_symbol(&painter, &drag.definition, screen(point), grid_step, 0, tokens.accent, None);
        }
    }
    if let Some(drag) = response.dnd_release_payload::<PartDrag>()
        && let Some(pointer) = drag_pointer
    {
        let point = camera.snap(pointer, rect);
        return (input_enabled && valid_position(point)).then(|| Click::Drop(PartDrag::clone(&drag), point));
    }

    if tool.is_none() && (input_enabled || response.context_menu_opened()) {
        // A right-click names its symbol this frame; afterwards the menu follows the selection.
        let pressed = response.secondary_clicked().then(|| response.interact_pointer_pos()).flatten();
        let target = match pressed {
            Some(pointer) => symbol_at(project, camera, rect, pointer).map(|(symbol, _)| symbol.id()),
            None => selected.map(str::to_owned),
        };
        let picked = menu::context(&response, |ui| {
            let Some(id) = target.clone() else {
                ui.close();
                return None;
            };
            let rows =
                [("Rotate", "R", SymbolAction::Rotate), ("Edit Fields…", "E", SymbolAction::EditFields), ("Delete", "Del", SymbolAction::Delete)];
            rows.into_iter().find_map(|(label, key, action)| MenuItem::new(label).shortcut(key).show(ui).then(|| Click::Edit(action, id.clone())))
        });
        if let Some(click) = picked.and_then(|shown| shown.inner) {
            return Some(click);
        }
        if pressed.is_some() {
            return Some(Click::Select(target));
        }
    }

    if moving && response.drag_stopped_by(egui::PointerButton::Primary) {
        return Some(Click::FinishMove(moved.map(|(_, point)| point).filter(|_| input_enabled)));
    }
    if input_enabled
        && tool.is_none()
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
    {
        return grab_symbol(project, camera, rect, pointer);
    }

    if !input_enabled || response.dragged() {
        return None;
    }
    if tool.is_none() && response.clicked_by(egui::PointerButton::Primary) {
        let pointer = response.interact_pointer_pos()?;
        let symbol = symbol_at(project, camera, rect, pointer).map(|(symbol, _)| symbol.id());
        if symbol.is_none()
            && let Some(wire) = wires::at(project, camera, rect, pointer)
        {
            return Some(Click::SelectWire(wire));
        }
        return Some(Click::Select(symbol));
    }
    match tool? {
        Tool::PlaceSymbol { definition, turns, .. } => {
            placement_preview(&painter, rect, response, camera, tokens, &definition, turns).map(Click::Place)
        }
        Tool::Wire { start, .. } => {
            wires::pick(&painter, rect, &response, project, camera, start, draft).map(|(anchor, finish)| Click::WirePoint(anchor, finish))
        }
        Tool::Move { .. } => None,
    }
}

/// Pick the nearest symbol body under the original press and preserve the grab offset.
fn grab_symbol(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<Click> {
    let (symbol, center) = symbol_at(project, camera, rect, pointer)?;
    Some(Click::Grab(symbol.clone(), center - pointer))
}

/// The symbol whose body is under the pointer, nearest center first, with its screen center.
pub(super) fn symbol_at<'a>(project: &'a Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<(&'a SymbolPlacement, Pos2)> {
    project
        .symbols()
        .iter()
        .rev()
        .map(|symbol| (symbol, camera.screen(symbol.position, rect)))
        .filter(|(symbol, center)| {
            body(&symbol.definition, symbol.turns())
                .translate(center.to_vec2() / camera.grid_step())
                .contains((pointer.to_vec2() / camera.grid_step()).to_pos2())
        })
        .min_by(|(_, a), (_, b)| a.distance_sq(pointer).total_cmp(&b.distance_sq(pointer)))
}

/// Turn an offset in grid steps by quarter turns counterclockwise, as the engine turns pins.
fn turn(x: f32, y: f32, turns: u8) -> (f32, f32) {
    match turns % 4 {
        0 => (x, y),
        1 => (y, -x),
        2 => (-x, -y),
        _ => (-y, x),
    }
}

/// A rectangle in grid steps after quarter turns: the turned corners' bounds.
fn turn_rect(rect: Rect, turns: u8) -> Rect {
    let (ax, ay) = turn(rect.min.x, rect.min.y, turns);
    let (bx, by) = turn(rect.max.x, rect.max.y, turns);
    Rect::from_two_pos(pos2(ax, ay), pos2(bx, by))
}

/// Keep previews inside the same coordinate bounds the command validates.
fn valid_position(point: Point) -> bool {
    (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.x_nm) && (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.y_nm)
}

/// Draw the pending symbol on its snapped model point until the user commits or cancels.
fn placement_preview(
    painter: &Painter,
    rect: Rect,
    response: Response,
    camera: &Camera,
    tokens: Tokens,
    definition: &SymbolDefinition,
    turns: u8,
) -> Option<Point> {
    let screen = |point| camera.screen(point, rect);
    let pointer = response.hover_pos().or_else(|| response.interact_pointer_pos())?;
    let point = camera.snap(pointer, rect);
    if !valid_position(point) {
        return None;
    }
    paint_symbol(painter, definition, screen(point), camera.grid_step(), turns, tokens.accent, None);
    response.on_hover_cursor(egui::CursorIcon::Crosshair).clicked_by(egui::PointerButton::Primary).then_some(point)
}

/// Draw visible grid intersections around the same origin used for snapping.
fn paint_grid(painter: &Painter, rect: Rect, origin: Pos2, tokens: Tokens, mut step: f32) {
    while step < theme::SCHEMATIC_MIN_GRID_SPACING {
        step *= 5.0;
    }
    let first_x = ((rect.left() - origin.x) / step).ceil() as i32;
    let last_x = ((rect.right() - origin.x) / step).floor() as i32;
    let first_y = ((rect.top() - origin.y) / step).ceil() as i32;
    let last_y = ((rect.bottom() - origin.y) / step).floor() as i32;
    for x in first_x..=last_x {
        for y in first_y..=last_y {
            let center = pos2(origin.x + x as f32 * step, origin.y + y as f32 * step);
            painter.circle_filled(center, theme::SCHEMATIC_GRID_DOT_RADIUS, tokens.canvas_grid);
        }
    }
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
pub(super) fn grid_bounds(definition: &SymbolDefinition, turns: u8) -> Rect {
    turn_rect(unturned_bounds(definition), turns)
}

/// [`grid_bounds`] before rotation.
fn unturned_bounds(definition: &SymbolDefinition) -> Rect {
    let pins = definition.pins().iter().map(|pin| Rect::from_pos(grid_pos(pin.offset)));
    pins.fold(drawing_bounds(definition), Rect::union)
}

/// The bounds of a symbol's strokes alone, in grid steps before rotation, or nothing when it
/// has none.
fn drawing_bounds(definition: &SymbolDefinition) -> Rect {
    let point = |point: &Point| Rect::from_pos(grid_pos(*point));
    definition.drawing.iter().fold(Rect::NOTHING, |bounds, shape| {
        bounds.union(match shape {
            Shape::Line(from, to) | Shape::Rect(from, to) => point(from).union(point(to)),
            Shape::Polyline(points) => points.iter().map(point).fold(Rect::NOTHING, Rect::union),
            Shape::Circle(center, radius) => point(center).expand(*radius as f32 / GRID),
            Shape::Text(at, _, _) => point(at),
        })
    })
}

/// Where an upright label sits on its anchor after the symbol turns. Half a turn swaps start
/// and end so a label inside a body edge stays inside; a quarter turn centers it.
fn text_anchor(align: TextAlign, turns: u8) -> Align2 {
    match (align, turns % 4) {
        (TextAlign::Middle, _) | (_, 1 | 3) => Align2::CENTER_CENTER,
        (TextAlign::Start, 0) | (TextAlign::End, 2) => Align2::LEFT_CENTER,
        _ => Align2::RIGHT_CENTER,
    }
}

/// The grabbable area of a symbol in grid steps around its center, after its turns: a 5 by 4
/// step box at the center, grown to the drawing's strokes so a large part can be picked by its
/// body. Pins stay outside it unless a stroke reaches them.
fn body(definition: &SymbolDefinition, turns: u8) -> Rect {
    let core = Rect::from_center_size(Pos2::ZERO, vec2(5.0, 4.0));
    let drawing = drawing_bounds(definition);
    let unturned = if drawing.is_positive() { core.union(drawing.shrink2(vec2(1.0, 0.0)).intersect(drawing)) } else { core };
    turn_rect(unturned, turns)
}

/// Draw the symbol's library strokes around its center, with one grid step of `step` points,
/// turned with the symbol. With a label, the reference goes above the drawing and the value
/// below it, or both beside an upright part.
/// Horizontal labels follow the rotated drawing's upper and lower clearance, including LED arrows.
fn paint_symbol(
    painter: &Painter,
    definition: &SymbolDefinition,
    center: Pos2,
    step: f32,
    turns: u8,
    color: egui::Color32,
    label: Option<(&str, &str)>,
) {
    let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color);
    let xy = |point: &Point| {
        let (x, y) = grid(*point);
        let (x, y) = turn(x, y, turns);
        pos2(center.x + x * step, center.y + y * step)
    };
    for shape in &definition.drawing {
        match shape {
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
                if step >= theme::SCHEMATIC_MIN_GRID_SPACING {
                    let font = FontId::proportional((step * 1.1).clamp(Tokens::get().font_small * 0.8, Tokens::get().font_button));
                    painter.text(xy(at), text_anchor(*align, turns), text, font, color);
                }
            }
        }
    }
    if let Some((reference, value)) = label {
        // Text clears the turned drawing by a little under half a grid step.
        let drawing = drawing_bounds(definition);
        let bounds = if drawing.is_positive() { turn_rect(drawing, turns) } else { Rect::from_center_size(Pos2::ZERO, vec2(2.0, 2.0)) };
        let t = Tokens::get();
        let font = FontId::proportional(t.font_button);
        if turns % 2 == 1 {
            // Upright parts carry their text at the side, as KiCad places it.
            let side = center + vec2((step * (bounds.max.x + 0.4)).max(t.space_12), step * bounds.center().y);
            painter.text(side - vec2(0.0, t.space_2), Align2::LEFT_BOTTOM, reference, font.clone(), color);
            painter.text(side + vec2(0.0, t.space_2), Align2::LEFT_TOP, value, font, color);
        } else {
            let above = (step * (0.4 - bounds.min.y)).max(t.space_12);
            let below = (step * (bounds.max.y + 0.4)).max(t.space_12);
            painter.text(center - vec2(0.0, above), Align2::CENTER_BOTTOM, reference, font.clone(), color);
            painter.text(center + vec2(0.0, below), Align2::CENTER_TOP, value, font, color);
        }
    }
}
