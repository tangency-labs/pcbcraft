//! Board Outline drawing gestures. Previews are view-local; every completed edit is an engine command.
use crate::{canvas_camera::Camera, editor_view::ViewInput};
use egui::{PointerButton, Pos2, Rect, Response, Ui};
use pcbcraft_engine::{
    board::{drawing::Drawing, setup::LayerPurpose, view::BoardView},
    schematic::Point,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// The first canvas drawing tools; both author ordinary graphics on the active Board Outline layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Lines,
    Rectangle,
}

#[derive(Clone, Debug)]
struct Draft {
    source: ViewInput,
    layer: String,
    points: Vec<Point>,
}

#[derive(Clone, Debug)]
struct Edit {
    source: ViewInput,
    drawing: Drawing,
    original: Vec<Point>,
    vertex: Option<usize>,
    anchor: Point,
    drag: bool,
}

/// Independent selection/tool/gesture state for one board view, never stored in project files.
#[derive(Debug, Default)]
pub(super) struct State {
    pub(super) tool: Option<Tool>,
    pub(super) selected: Option<String>,
    pub(super) delete_requested: bool,
    pub(super) finish_requested: bool,
    pub(super) backspace_requested: bool,
    draft: Option<Draft>,
    edit: Option<Edit>,
    layer: Option<String>,
    /// Geometry submitted this frame, painted until the next pass sees the engine's result.
    committed: Option<Drawing>,
}

/// A captured document transaction is dispatched only after the canvas releases its borrows.
pub(super) struct Command {
    pub(super) source: ViewInput,
    pub(super) id: &'static str,
    pub(super) params: Value,
}

#[derive(Default)]
pub(super) struct Interaction {
    pub(super) consumed: bool,
    pub(super) command: Option<Command>,
}

impl State {
    /// Arm a tool with no authored changes; the first canvas click captures its revision and layer.
    pub(super) fn start(&mut self, tool: Tool, layer: Option<String>) {
        self.cancel();
        self.tool = Some(tool);
        self.selected = None;
        self.layer = layer;
    }
    /// Drop all drawing gestures when focus, workspace, layer or project authority changes.
    pub(super) fn cancel(&mut self) {
        self.tool = None;
        self.draft = None;
        self.edit = None;
        self.delete_requested = false;
        self.finish_requested = false;
        self.backspace_requested = false;
    }
    /// Escape cancels unfinished geometry first, then exits a tool, then clears a drawing selection.
    pub(super) fn escape(&mut self) -> bool {
        self.draft.take().is_some() || self.edit.take().is_some() || self.tool.take().is_some() || self.selected.take().is_some()
    }
    /// Return the original edit authority so synchronization can discard stale previews.
    pub(super) fn source(&self) -> Option<ViewInput> {
        self.draft.as_ref().map(|draft| draft.source).or_else(|| self.edit.as_ref().map(|edit| edit.source))
    }
}

/// Whether the active, visible layer accepts these tools. Copper visibility never enables drawing.
pub(super) fn can_draw(view: BoardView<'_>, active: Option<&str>, hidden: &BTreeSet<String>) -> bool {
    view.layers().iter().any(|layer| {
        layer.purpose == LayerPurpose::Outline && active == Some(layer.id.to_string().as_str()) && !hidden.contains(&layer.id.to_string())
    })
}

/// Prefer exact existing edge vertices within the pointer target, then use this view's snap grid.
fn snapped(pointer: Pos2, camera: &Camera, rect: Rect, drawings: &[Drawing]) -> Point {
    drawings
        .iter()
        .flat_map(|drawing| &drawing.points)
        .filter_map(|point| {
            let distance = camera.screen(*point, rect).distance(pointer);
            (distance <= 8.).then_some((distance, *point))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or_else(|| camera.snap(pointer, rect), |(_, point)| point)
}

/// Find a segment by screen distance, allowing vertical/diagonal edges and degenerate projections.
fn hit(pointer: Pos2, camera: &Camera, rect: Rect, drawing: &Drawing) -> bool {
    let points: Vec<_> = drawing.points.iter().map(|point| camera.screen(*point, rect)).collect();
    points.iter().copied().zip(points.iter().copied().cycle().skip(1)).take(points.len().saturating_sub(usize::from(!drawing.closed))).any(
        |(a, b)| {
            let delta = b - a;
            let t = if delta.length_sq() > 0. { ((pointer - a).dot(delta) / delta.length_sq()).clamp(0., 1.) } else { 0. };
            pointer.distance(a + t * delta) <= 6.
        },
    )
}

/// Construct a rectangle from two clicked opposite corners, preserving arbitrary draw direction.
fn rectangle(a: Point, b: Point) -> Option<Vec<Point>> {
    (a.x_nm != b.x_nm && a.y_nm != b.y_nm).then(|| vec![a, Point { x_nm: b.x_nm, y_nm: a.y_nm }, b, Point { x_nm: a.x_nm, y_nm: b.y_nm }])
}

/// Emit one create/update transaction; clicking an open path's first vertex makes closure explicit.
fn command(source: ViewInput, id: &'static str, mut params: Value) -> Command {
    params["expected_revision"] = json!(source.revision);
    Command { source, id, params }
}

/// Route canvas-only drawing/selection/vertex and whole-graphic movement. Release over chrome,
/// right-click, stale input or a hidden/changed layer cannot commit an unfinished edit.
/// `selectable` gates existing graphics; explicitly chosen creation tools remain available.
#[allow(clippy::too_many_arguments)]
pub(super) fn interact(
    ui: &mut Ui,
    response: &Response,
    camera: &Camera,
    view: BoardView<'_>,
    state: &mut State,
    active: Option<&str>,
    hidden: &BTreeSet<String>,
    source: Option<ViewInput>,
    enabled: bool,
    move_requested: bool,
    selectable: bool,
) -> Interaction {
    let mut out = Interaction::default();
    state.committed = None;
    let rect = response.rect;
    let all = view.drawings();
    let drawings: Vec<_> = all.into_iter().filter(|drawing| !hidden.contains(&drawing.layer.to_string())).collect();
    if state.selected.as_ref().is_some_and(|id| !drawings.iter().any(|drawing| drawing.id.to_string() == *id)) {
        state.selected = None;
    }
    if state.edit.as_ref().is_some_and(|edit| hidden.contains(&edit.drawing.layer.to_string())) {
        state.cancel();
    }
    if !enabled || state.source().is_some_and(|old| Some(old) != source) || state.layer.as_deref() != active {
        state.cancel();
        state.layer = active.map(str::to_owned);
    }
    let Some(source) = source.filter(|_| enabled) else { return out };
    let selected = drawings.iter().find(|drawing| state.selected.as_ref() == Some(&drawing.id.to_string()));
    if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) && (state.tool.is_some() || state.edit.is_some()) {
        state.escape();
        out.consumed = true;
        return out;
    }
    if std::mem::take(&mut state.delete_requested)
        && let Some(drawing) = selected
    {
        out.command = Some(command(source, pcbcraft_engine::DELETE_DRAWING, json!({"drawing":drawing.id})));
        out.consumed = true;
        return out;
    }
    let pointer = ui.input(|input| input.pointer.hover_pos());
    if let Some(tool) = state.tool {
        out.consumed = true;
        if !can_draw(view, active, hidden) {
            state.cancel();
            return out;
        }
        if tool == Tool::Lines
            && std::mem::take(&mut state.backspace_requested)
            && let Some(draft) = &mut state.draft
        {
            draft.points.pop();
            if draft.points.is_empty() {
                state.draft = None;
            }
        }
        if std::mem::take(&mut state.finish_requested) {
            if let Some(draft) = state.draft.as_ref().filter(|draft| draft.points.len() >= 2) {
                out.command =
                    Some(command(draft.source, pcbcraft_engine::ADD_DRAWING, json!({"layer":draft.layer,"points":draft.points,"closed":false})));
                state.draft = None;
            }
            return out;
        }
        let Some(pointer) = pointer.filter(|point| rect.contains(*point)) else { return out };
        let at = snapped(pointer, camera, rect, &drawings);
        if response.clicked() {
            if let Some(draft) = &mut state.draft {
                if tool == Tool::Rectangle {
                    if let Some(points) = rectangle(draft.points[0], at) {
                        out.command =
                            Some(command(draft.source, pcbcraft_engine::ADD_DRAWING, json!({"layer":draft.layer,"points":points,"closed":true})));
                        state.draft = None;
                    }
                } else {
                    let close = draft.points.len() >= 3 && camera.screen(draft.points[0], rect).distance(pointer) <= 8.;
                    if !close && draft.points.last() != Some(&at) && draft.points.len() < 256 {
                        draft.points.push(at);
                    }
                    if (close || response.double_clicked()) && draft.points.len() >= 2 {
                        out.command = Some(command(
                            draft.source,
                            pcbcraft_engine::ADD_DRAWING,
                            json!({"layer":draft.layer,"points":draft.points,"closed":close}),
                        ));
                        state.draft = None;
                    }
                }
            } else if response.clicked() {
                state.draft = Some(Draft { source, layer: active.unwrap_or_default().to_owned(), points: vec![at] });
            }
        }
        return out;
    }
    if !selectable {
        return out;
    }
    if state.edit.is_none() && (response.drag_started_by(PointerButton::Primary) || (move_requested && selected.is_some())) {
        let origin = if move_requested {
            pointer
                .filter(|point| rect.contains(*point))
                .or_else(|| selected.and_then(|drawing| drawing.points.first()).map(|point| camera.screen(*point, rect)))
        } else {
            ui.input(|input| input.pointer.press_origin())
        };
        if let Some(origin) = origin {
            let drawing = selected
                .filter(|drawing| move_requested || hit(origin, camera, rect, drawing))
                .or_else(|| drawings.iter().rev().find(|drawing| hit(origin, camera, rect, drawing)));
            if let Some(drawing) = drawing {
                let vertex =
                    (!move_requested).then(|| drawing.points.iter().position(|point| camera.screen(*point, rect).distance(origin) <= 8.)).flatten();
                state.selected = Some(drawing.id.to_string());
                state.edit = Some(Edit {
                    source,
                    drawing: drawing.clone(),
                    original: drawing.points.clone(),
                    vertex,
                    anchor: camera.snap(origin, rect),
                    drag: !move_requested,
                });
            }
        }
    }
    if let Some(edit) = &mut state.edit {
        out.consumed = true;
        if let Some(pointer) = pointer {
            let at = snapped(pointer, camera, rect, &drawings.iter().filter(|drawing| drawing.id != edit.drawing.id).cloned().collect::<Vec<_>>());
            if let Some(index) = edit.vertex {
                edit.drawing.points[index] = at;
            } else {
                let dx = at.x_nm - edit.anchor.x_nm;
                let dy = at.y_nm - edit.anchor.y_nm;
                edit.drawing.points =
                    edit.original.iter().map(|point| Point { x_nm: point.x_nm.saturating_add(dx), y_nm: point.y_nm.saturating_add(dy) }).collect();
            }
        }
        let completed = if edit.drag { response.drag_stopped_by(PointerButton::Primary) } else { response.clicked() && !move_requested };
        if completed {
            if response.contains_pointer() {
                let mut candidate = edit.drawing.clone();
                if !candidate.closed && candidate.points.len() > 3 && candidate.points.first() == candidate.points.last() {
                    candidate.points.pop();
                    candidate.closed = true;
                }
                out.command = Some(command(
                    edit.source,
                    pcbcraft_engine::SET_DRAWING,
                    json!({"drawing":candidate.id,"points":candidate.points,"closed":candidate.closed}),
                ));
                state.committed = Some(candidate);
            }
            state.edit = None;
        } else if edit.drag && !ui.input(|input| input.pointer.primary_down()) {
            state.edit = None;
        }
    } else if response.clicked() {
        state.selected = pointer
            .and_then(|pointer| drawings.iter().rev().find(|drawing| hit(pointer, camera, rect, drawing)))
            .map(|drawing| drawing.id.to_string());
        out.consumed = state.selected.is_some();
    }
    out
}

/// Paint only visible layer graphics and disposable previews; selected vertices are handles,
/// not copper pads or electrical edit selections. No physical stroke width is implied.
pub(super) fn paint(
    ui: &Ui,
    rect: Rect,
    camera: &Camera,
    view: BoardView<'_>,
    state: &State,
    hidden: &BTreeSet<String>,
    appearance: &crate::board_appearance::Appearance,
) {
    let painter = ui.painter().with_clip_rect(rect);
    for mut drawing in view.drawings().into_iter().filter(|drawing| !hidden.contains(&drawing.layer.to_string())) {
        if let Some(edit) = state.edit.as_ref().filter(|edit| edit.drawing.id == drawing.id) {
            drawing = edit.drawing.clone();
        } else if let Some(committed) = state.committed.as_ref().filter(|committed| committed.id == drawing.id) {
            drawing = committed.clone();
        }
        let selected = state.selected.as_ref() == Some(&drawing.id.to_string());
        pcbcraft_chrome::board::drawing(
            &painter,
            drawing.points.iter().map(|point| camera.screen(*point, rect)).collect(),
            drawing.closed,
            selected,
            false,
            view.layers()
                .iter()
                .find(|layer| layer.id == drawing.layer)
                .map_or(pcbcraft_chrome::Tokens::get().board_ink, |layer| appearance.color(view, layer)),
        );
    }
    if let Some(draft) = &state.draft
        && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
    {
        let visible: Vec<_> = view.drawings().into_iter().filter(|drawing| !hidden.contains(&drawing.layer.to_string())).collect();
        let at = snapped(pointer, camera, rect, &visible);
        let points = if state.tool == Some(Tool::Rectangle) {
            rectangle(draft.points[0], at).unwrap_or_else(|| vec![draft.points[0], at])
        } else {
            let mut points = draft.points.clone();
            points.push(at);
            points
        };
        pcbcraft_chrome::board::drawing(
            &painter,
            points.iter().map(|point| camera.screen(*point, rect)).collect(),
            state.tool == Some(Tool::Rectangle),
            false,
            true,
            view.layers()
                .iter()
                .find(|layer| layer.id.to_string() == draft.layer)
                .map_or(pcbcraft_chrome::Tokens::get().board_ink, |layer| appearance.color(view, layer)),
        );
        pcbcraft_chrome::board::drawing_target(&painter, camera.screen(at, rect));
    }
}
