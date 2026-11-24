//! Active-layer drawing gestures. Previews are view-local; every completed edit is an engine command.
use crate::{canvas_camera::Camera, editor_view::ViewInput};
use egui::{PointerButton, Pos2, Rect, Response, Ui};
use pcbcraft_engine::{
    board::{drawing::Drawing, setup::LayerPurpose, view::BoardView},
    schematic::Point,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Lines and rectangle outlines use the active layer. Copper becomes conductive segments;
/// technical layers retain independently editable graphics with physical stroke widths.
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
    pub(super) zone: bool,
    pub(super) zone_choices: Option<(pcbcraft_engine::board::view::BoardTarget, u64, Vec<crate::board_net_choice::NetChoice>)>,
    pub(super) zone_target: Option<pcbcraft_engine::nets::ClassTarget>,
    /// A chosen net owns its revision before the first point, and between repeated creations.
    pub(super) zone_source: Option<ViewInput>,
    pub(super) tool: Option<Tool>,
    pub(super) selected: Option<String>,
    selected_many: BTreeSet<uuid::Uuid>,
    pub(super) delete_requested: bool,
    pub(super) finish_requested: bool,
    pub(super) backspace_requested: bool,
    draft: Option<Draft>,
    edit: Option<Edit>,
    layer: Option<String>,
    /// A press owns its original revision even before movement crosses the drag threshold.
    press: Option<ViewInput>,
    blocked_press: bool,
    /// Geometry submitted this frame, painted until the next pass sees the engine's result.
    committed: Option<Drawing>,
}

/// A captured document transaction is dispatched only after the canvas releases its borrows.
pub(super) struct Command {
    pub(super) source: ViewInput,
    pub(super) id: &'static str,
    pub(super) params: Value,
    /// Keep the original gesture until the engine accepts its creation transaction.
    retry: Option<Draft>,
}

impl Command {
    /// Capture a canvas transaction without attaching a drawing creation gesture.
    pub(super) fn new(source: ViewInput, id: &'static str, params: Value) -> Self {
        Self { source, id, params, retry: None }
    }

    /// Restore a rejected creation gesture after dispatch. Editing/deletion commands have no
    /// creation draft; callers validate captured view authority before applying the command.
    pub(super) fn restore_rejected(self, state: &mut State) {
        if let Some(draft) = self.retry {
            state.draft = Some(draft);
        }
    }
}

#[derive(Default)]
pub(super) struct Interaction {
    pub(super) consumed: bool,
    pub(super) command: Option<Command>,
}

impl State {
    /// Project the selected authored category into the shared outline gesture geometry.
    fn geometry(&self, view: BoardView<'_>) -> Vec<Drawing> {
        if self.zone {
            view.zones().iter().map(|z| Drawing { id: z.id, layer: z.layer, points: z.points.clone(), closed: true, stroke_nm: None }).collect()
        } else {
            view.drawings()
        }
    }

    /// Read explicit graphic selections, including older single-item callers.
    pub(super) fn selection(&self) -> BTreeSet<uuid::Uuid> {
        self.selected_many.iter().copied().chain(self.selected.as_ref().and_then(|id| id.parse().ok())).collect()
    }
    /// Replace all drawing editing targets; a single target also enables its existing inspector.
    pub(super) fn select_many(&mut self, selected: BTreeSet<uuid::Uuid>) {
        self.selected = if selected.len() == 1 { selected.first().map(ToString::to_string) } else { None };
        self.selected_many = selected;
    }
    /// A click or competing category replaces a previous group rather than leaving hidden targets.
    pub(super) fn select(&mut self, selected: Option<String>) {
        self.selected_many.clear();
        self.selected = selected;
    }

    /// Arm a tool with no authored changes; the first canvas click captures its revision and layer.
    pub(super) fn start(&mut self, tool: Tool, layer: Option<String>) {
        self.cancel();
        self.tool = Some(tool);
        self.select(None);
        self.layer = layer;
    }
    /// Drop all drawing gestures when focus, workspace, layer or project authority changes.
    pub(super) fn cancel(&mut self) {
        self.zone_source = None;
        self.tool = None;
        self.press = None;
        self.blocked_press = false;
        self.draft = None;
        self.edit = None;
        self.delete_requested = false;
        self.finish_requested = false;
        self.backspace_requested = false;
    }
    /// Escape cancels unfinished geometry first, then exits a tool, then clears a drawing selection.
    pub(super) fn escape(&mut self) -> bool {
        if self.draft.take().is_some() || self.edit.take().is_some() || self.tool.take().is_some() {
            return true;
        }
        let had = !self.selection().is_empty();
        self.select(None);
        had
    }
    /// Return edit or armed-net authority so even a tool without points rejects stale input.
    pub(super) fn source(&self) -> Option<ViewInput> {
        self.draft.as_ref().map(|draft| draft.source).or_else(|| self.edit.as_ref().map(|edit| edit.source)).or(self.tool.and(self.zone_source))
    }
}

/// Whether the active layer exists and is visible. Copper uses validated conductive paths.
pub(super) fn can_draw(view: BoardView<'_>, active: Option<&str>, hidden: &BTreeSet<String>) -> bool {
    view.layers().iter().any(|layer| active == Some(layer.id.to_string().as_str()) && !hidden.contains(&layer.id.to_string()))
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
            pointer.distance(a + t * delta)
                <= drawing.stroke_nm.map_or(6., |width| (width as f32 * camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32 / 2.).max(6.))
        },
    )
}

/// Gather selectable edges for overlap cycling with the same hit tolerance as a click.
pub(super) fn edge_choices(
    state: &State,
    view: BoardView<'_>,
    hidden: &BTreeSet<String>,
    pointer: Pos2,
    camera: &Camera,
    rect: Rect,
) -> impl Iterator<Item = uuid::Uuid> {
    state
        .geometry(view)
        .into_iter()
        .rev()
        .filter(move |drawing| !hidden.contains(&drawing.layer.to_string()) && hit(pointer, camera, rect, drawing))
        .map(|drawing| drawing.id)
}

/// Construct a rectangle from two clicked opposite corners, preserving arbitrary draw direction.
fn rectangle(a: Point, b: Point) -> Option<Vec<Point>> {
    (a.x_nm != b.x_nm && a.y_nm != b.y_nm).then(|| vec![a, Point { x_nm: b.x_nm, y_nm: a.y_nm }, b, Point { x_nm: a.x_nm, y_nm: b.y_nm }])
}

/// Emit one create/update transaction; clicking an open path's first vertex makes closure explicit.
fn command(source: ViewInput, id: &'static str, mut params: Value) -> Command {
    params["expected_revision"] = json!(source.revision);
    Command::new(source, id, params)
}

/// Choose the existing typed command for the active material. Copper paths become atomic
/// track segments, including closure; other layers retain a graphic with its physical stroke.
/// Zones instead create nonconductive intent with their captured net target. An over-limit
/// copper closure or incomplete zone returns None so the editable draft is retained.
fn create_path(
    source: ViewInput,
    view: BoardView<'_>,
    layer: &str,
    points: &[Point],
    closed: bool,
    width_nm: i64,
    target: Option<&pcbcraft_engine::nets::ClassTarget>,
) -> Option<Command> {
    if let Some(target) = target {
        if !closed || points.len() < 3 {
            return None;
        }
        return Some(command(source, pcbcraft_engine::board::zones::ADD_ZONE, json!({"layer":layer,"points":points,"target":target})));
    }
    if view.layers().iter().any(|record| record.id.to_string() == layer && record.purpose == LayerPurpose::Copper) {
        if points.len() + usize::from(closed) > pcbcraft_engine::board::track::MAX_PATH_POINTS {
            return None;
        }
        let mut path = points.to_vec();
        if closed && let Some(first) = points.first() {
            path.push(*first);
        }
        Some(command(source, pcbcraft_engine::ADD_TRACKS, json!({"layer":layer,"points":path,"width_nm":width_nm})))
    } else {
        Some(command(source, pcbcraft_engine::ADD_DRAWING, json!({"layer":layer,"points":points,"closed":closed,"stroke_nm":width_nm})))
    }
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
    width_nm: i64,
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
    let all = state.geometry(view);
    let drawings: Vec<_> = all.into_iter().filter(|drawing| !hidden.contains(&drawing.layer.to_string())).collect();
    if state.selected.as_ref().is_some_and(|id| !drawings.iter().any(|drawing| drawing.id.to_string() == *id)) {
        state.select(None);
    }
    if !state.selected_many.is_empty() {
        state.selected_many.retain(|id| drawings.iter().any(|drawing| drawing.id == *id));
        state.selected = if state.selected_many.len() == 1 { state.selected_many.first().map(ToString::to_string) } else { None };
    }
    if state.edit.as_ref().is_some_and(|edit| hidden.contains(&edit.drawing.layer.to_string())) {
        state.cancel();
    }
    let primary_down = ui.input(|input| input.pointer.primary_down());
    if state.blocked_press {
        state.blocked_press = primary_down;
        out.consumed = true;
        return out;
    }
    let stale = state.source().or(state.press).is_some_and(|old| Some(old) != source);
    if stale {
        state.cancel();
        state.layer = active.map(str::to_owned);
        state.blocked_press = primary_down;
        out.consumed = true;
        return out;
    }
    if !enabled || state.layer.as_deref() != active {
        state.cancel();
        state.layer = active.map(str::to_owned);
    }
    let Some(source) = source.filter(|_| enabled) else { return out };
    let selected = drawings.iter().find(|drawing| state.selected.as_ref() == Some(&drawing.id.to_string()));
    if response.contains_pointer()
        && ui.input(|input| input.pointer.primary_pressed())
        && (state.tool.is_some()
            || ui.input(|input| input.pointer.press_origin()).is_some_and(|at| drawings.iter().any(|drawing| hit(at, camera, rect, drawing))))
    {
        state.press = Some(source);
    }
    if ui.input(|input| input.pointer.primary_released()) {
        state.press = None;
    }

    if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) && (state.tool.is_some() || state.edit.is_some()) {
        state.escape();
        out.consumed = true;
        return out;
    }
    if std::mem::take(&mut state.delete_requested)
        && let Some(drawing) = selected
    {
        out.command = Some(if state.zone {
            command(source, pcbcraft_engine::board::zones::DELETE_ZONES, json!({"zones":[drawing.id]}))
        } else {
            command(source, pcbcraft_engine::DELETE_DRAWING, json!({"drawing":drawing.id}))
        });
        out.consumed = true;
        return out;
    }
    let pointer = ui.input(|input| input.pointer.hover_pos());
    if let Some(tool) = state.tool {
        out.consumed = true;
        if !can_draw(view, active, hidden)
            || (state.zone
                && !view.layers().iter().any(|layer| Some(layer.id.to_string()).as_deref() == active && layer.purpose == LayerPurpose::Copper))
        {
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
                out.command = create_path(draft.source, view, &draft.layer, &draft.points, state.zone, width_nm, state.zone_target.as_ref());
                if let Some(command) = &mut out.command {
                    command.retry = state.draft.take();
                }
            }
            return out;
        }
        let Some(pointer) = pointer.filter(|point| rect.contains(*point)) else { return out };
        let at = snapped(pointer, camera, rect, &drawings);
        if response.clicked() {
            if let Some(draft) = &mut state.draft {
                if tool == Tool::Rectangle {
                    if let Some(points) = rectangle(draft.points[0], at) {
                        out.command = create_path(draft.source, view, &draft.layer, &points, true, width_nm, state.zone_target.as_ref());
                        if let Some(command) = &mut out.command {
                            command.retry = state.draft.take();
                        }
                    }
                } else {
                    let close = draft.points.len() >= 3 && camera.screen(draft.points[0], rect).distance(pointer) <= 8.;
                    if !close && draft.points.last() != Some(&at) && draft.points.len() < 256 {
                        draft.points.push(at);
                    }
                    if (close || response.double_clicked()) && draft.points.len() >= 2 {
                        out.command =
                            create_path(draft.source, view, &draft.layer, &draft.points, close || state.zone, width_nm, state.zone_target.as_ref());
                        if let Some(command) = &mut out.command {
                            command.retry = state.draft.take();
                        }
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
    let modifiers = ui.input(|input| input.modifiers);
    let extend = modifiers.shift || modifiers.ctrl || modifiers.command;
    if state.edit.is_none() && ((!extend && response.drag_started_by(PointerButton::Primary)) || (move_requested && selected.is_some())) {
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
                state.select(Some(drawing.id.to_string()));
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
                    if state.zone { pcbcraft_engine::board::zones::SET_ZONE } else { pcbcraft_engine::SET_DRAWING },
                    if state.zone {
                        json!({"zone":candidate.id,"points":candidate.points})
                    } else {
                        json!({"drawing":candidate.id,"points":candidate.points,"closed":candidate.closed})
                    },
                ));
                state.committed = Some(candidate);
            }
            state.edit = None;
        } else if edit.drag && !ui.input(|input| input.pointer.primary_down()) {
            state.edit = None;
        }
    } else if response.clicked() {
        let selected =
            pointer.and_then(|pointer| drawings.iter().rev().find(|drawing| hit(pointer, camera, rect, drawing))).map(|drawing| drawing.id);
        if extend {
            if let Some(id) = selected {
                let mut selection = state.selection();
                if !(modifiers.ctrl || modifiers.command) || !selection.remove(&id) {
                    selection.insert(id);
                }
                state.select_many(selection);
            }
        } else {
            state.select(selected.map(|id| id.to_string()));
        }
        out.consumed = selected.is_some();
    }
    out
}

/// Paint only visible layer graphics and disposable previews; selected vertices are handles,
/// not copper pads or electrical edit selections. Physical widths scale with the camera;
/// legacy hairlines retain their original presentation.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    painter: &egui::Painter,
    rect: Rect,
    camera: &Camera,
    view: BoardView<'_>,
    state: &State,
    width_nm: i64,
    hidden: &BTreeSet<String>,
    appearance: &crate::board_appearance::Appearance,
    palette: Option<&std::collections::BTreeMap<uuid::Uuid, egui::Color32>>,
) {
    for mut drawing in state.geometry(view).into_iter().filter(|drawing| !hidden.contains(&drawing.layer.to_string())) {
        if let Some(edit) = state.edit.as_ref().filter(|edit| edit.drawing.id == drawing.id) {
            drawing = edit.drawing.clone();
        } else if let Some(committed) = state.committed.as_ref().filter(|committed| committed.id == drawing.id) {
            drawing = committed.clone();
        }
        let selected = state.selected.as_ref() == Some(&drawing.id.to_string()) || state.selected_many.contains(&drawing.id);
        let net_color = palette.and_then(|palette| {
            let zone = view.zones().iter().find(|zone| zone.id == drawing.id)?;
            (view.zone_net_state(zone.net) == "active").then(|| palette.get(&zone.net)).flatten().copied()
        });
        let color = net_color.unwrap_or_else(|| {
            view.layers()
                .iter()
                .find(|layer| layer.id == drawing.layer)
                .map_or(pcbcraft_chrome::Tokens::get().board_ink, |layer| appearance.color(view, layer))
        });
        if state.zone {
            let points: Vec<_> = drawing.points.iter().map(|point| camera.screen(*point, rect)).collect();
            pcbcraft_chrome::board::zone_hatch(painter, &points, color);
        }
        pcbcraft_chrome::board::drawing(
            painter,
            drawing.points.iter().map(|point| camera.screen(*point, rect)).collect(),
            drawing.closed,
            drawing.stroke_nm.map(|width| width as f32 * camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32),
            selected,
            false,
            color,
        );
    }
    if let Some(draft) = &state.draft
        && let Some(pointer) = painter.ctx().input(|input| input.pointer.hover_pos())
    {
        let visible: Vec<_> = state.geometry(view).into_iter().filter(|drawing| !hidden.contains(&drawing.layer.to_string())).collect();
        let at = snapped(pointer, camera, rect, &visible);
        let points = if state.tool == Some(Tool::Rectangle) {
            rectangle(draft.points[0], at).unwrap_or_else(|| vec![draft.points[0], at])
        } else {
            let mut points = draft.points.clone();
            points.push(at);
            points
        };
        pcbcraft_chrome::board::drawing(
            painter,
            points.iter().map(|point| camera.screen(*point, rect)).collect(),
            state.tool == Some(Tool::Rectangle),
            Some(width_nm as f32 * camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32),
            false,
            true,
            view.layers()
                .iter()
                .find(|layer| layer.id.to_string() == draft.layer)
                .map_or(pcbcraft_chrome::Tokens::get().board_ink, |layer| appearance.color(view, layer)),
        );
        pcbcraft_chrome::board::drawing_target(painter, camera.screen(at, rect));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{click, click_canvas_at, harness};
    use egui_kittest::kittest::Queryable;

    /// Two accepted nets share separate pads on each resistor, so crossing the first part's
    /// pads exercises a real engine refusal without a stale revision or invalid layer.
    fn two_net_board() -> egui_kittest::Harness<'static, crate::PcbApp> {
        use pcbcraft_engine::{CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL};
        let mut h = harness();
        h.set_size(egui::vec2(1400., 900.));
        h.state_mut().run("project.new", &json!({"create_board":true}));
        let mut symbols = Vec::new();
        for index in 0..2 {
            let revision = h.state().session().revision();
            h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index*10_000_000,"y_nm":0,"expected_revision":revision}));
            let symbol = h.state().session().project().unwrap().symbols().last().unwrap().id();
            symbols.push(symbol.clone());
            let revision = h.state().session().revision();
            h.state_mut().run(PLACE_FOOTPRINT, &json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":index*10_160_000,"y_nm":0,"expected_revision":revision}));
        }
        for terminal in ["1", "2"] {
            let revision = h.state().session().revision();
            h.state_mut().run(CONNECT_PINS, &json!({"from":{"symbol":symbols[0],"terminal":terminal},"to":{"symbol":symbols[1],"terminal":terminal},"expected_revision":revision}));
        }
        let revision = h.state().session().revision();
        h.state_mut().run(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":revision}));
        let revision = h.state().session().revision();
        h.state_mut().run(pcbcraft_engine::board::update::INITIALIZE_BASELINE, &json!({"expected_revision":revision}));
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        h.run();
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB Layout");
        click(&mut h, "Activate layer Top");
        h
    }

    /// Enter, path closure and rectangle completion retain their editable gesture when actual
    /// copper validation refuses a net short; correcting that gesture commits it exactly once.
    #[test]
    fn rejected_copper_completion_retains_draft_for_correction() {
        use pcbcraft_engine::persistence::encode_documents;
        for completion in ["enter", "close", "rectangle"] {
            let mut h = two_net_board();
            assert!(h.state_mut().views.active.camera.set_snap_nm(10_000));
            let view = h.state().session().project().unwrap().boards().next().unwrap();
            let part = view.footprints().next().unwrap();
            assert!(part.pads[0].net.is_some() && part.pads[1].net.is_some() && part.pads[0].net != part.pads[1].net);
            let mut pads = [part.pads[0].center, part.pads[1].center];
            pads.sort_by_key(|point| point.x_nm);
            let [a, b] = pads;
            let rectangle = completion == "rectangle";
            click(&mut h, "Draw");
            click(&mut h, if rectangle { "Draw rectangle on active layer" } else { "Draw lines on active layer" });
            let source = h.state().views.active.input(h.state().session()).unwrap();
            let layer = h.state().views.active.board.active_layer.clone().unwrap();
            let points = if rectangle {
                vec![a]
            } else {
                vec![a, Point { x_nm: a.x_nm, y_nm: a.y_nm + 5_000_000 }, Point { x_nm: b.x_nm, y_nm: b.y_nm + 5_000_000 }, b]
            };
            h.state_mut().views.active.board.drawing.draft = Some(Draft { source, layer, points: points.clone() });
            let original = encode_documents(h.state().session().snapshot().unwrap()).unwrap();
            let revision = h.state().session().revision();
            if completion == "enter" {
                h.key_press(egui::Key::Enter);
                h.run();
            } else {
                let at = if rectangle { Point { x_nm: b.x_nm, y_nm: b.y_nm + 5_000_000 } } else { a };
                let screen = h.state().views.active.camera.screen(at, h.get_by_label("PCB canvas").rect());
                click_canvas_at(&mut h, screen);
            }
            assert!(
                h.state().message.as_deref().is_some_and(|message| message.contains("would connect nets")),
                "{completion}: {:?}, pads={pads:?}, state={:?}, tracks={:?}",
                h.state().message,
                h.state().views.active.board.drawing,
                h.state().session().project().unwrap().boards().next().unwrap().tracks()
            );
            assert_eq!(h.state().session().revision(), revision);
            assert_eq!(encode_documents(h.state().session().snapshot().unwrap()).unwrap(), original);
            let draft = h.state().views.active.board.drawing.draft.as_ref().expect("rejected geometry remains editable");
            assert_eq!(draft.points, points);
            assert_eq!(draft.source, source);
            h.run_steps(3);
            assert_eq!(h.state().session().revision(), revision, "no automatic retry");
            if rectangle {
                let at = Point { x_nm: a.x_nm - 5_000_000, y_nm: a.y_nm + 5_000_000 };
                let screen = h.state().views.active.camera.screen(at, h.get_by_label("PCB canvas").rect());
                click_canvas_at(&mut h, screen);
            } else {
                h.key_press(egui::Key::Backspace);
                h.run();
                h.key_press(egui::Key::Enter);
                h.run();
            }
            assert!(h.state().message.is_none(), "{completion}: {:?}", h.state().message);
            assert!(h.state().views.active.board.drawing.draft.is_none());
            assert_eq!(h.state().views.active.board.drawing.tool, Some(if rectangle { Tool::Rectangle } else { Tool::Lines }));
            assert_eq!(h.state().session().revision(), revision + 1);
            assert_eq!(h.state().session().project().unwrap().boards().next().unwrap().tracks().len(), if rectangle { 4 } else { 2 });
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.run();
            assert_eq!(encode_documents(h.state().session().snapshot().unwrap()).unwrap(), original);
        }
    }

    /// A full copper draft cannot close past the engine limit, but remains editable and can
    /// still finish as an open 256-point path without losing the user's geometry.
    #[test]
    fn max_length_copper_closure_retains_draft_until_open_finish() {
        let mut h = harness();
        h.set_size(egui::vec2(1400., 900.));
        h.state_mut().run("project.new", &json!({"create_board":true}));
        h.run();
        let revision = h.state().session().revision();
        h.state_mut().run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":revision}));
        h.run();
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB Layout");
        let layer = h
            .state()
            .session()
            .project()
            .unwrap()
            .boards()
            .next()
            .unwrap()
            .layers()
            .into_iter()
            .find(|layer| layer.purpose == LayerPurpose::Copper)
            .unwrap()
            .id
            .to_string();
        h.state_mut().views.active.board.set_layer(&layer, true, None);
        h.run();
        click(&mut h, "Draw");
        click(&mut h, "Draw lines on active layer");
        let source = h.state().views.active.input(h.state().session()).unwrap();
        let points: Vec<_> = (0..256).map(|index| Point { x_nm: index * 100_000, y_nm: if index == 0 { 0 } else { 10_000_000 } }).collect();
        let first = h.state().views.active.camera.screen(points[0], h.get_by_label("PCB canvas").rect());
        h.state_mut().views.active.board.drawing.draft = Some(Draft { source, layer, points: points.clone() });
        let revision = h.state().session().revision();
        click_canvas_at(&mut h, first);
        assert_eq!(h.state().session().revision(), revision);
        assert_eq!(h.state().views.active.board.drawing.draft.as_ref().unwrap().points, points);
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(h.state().views.active.board.drawing.draft.is_none());
        assert_eq!(h.state().session().project().unwrap().boards().next().unwrap().tracks().len(), 255);
        assert_eq!(h.state().session().revision(), revision + 1);
    }
}
