//! Copper track routing and selection. Previews are view-local; every completed edit is an engine command.
use crate::{board_drawing::Command, canvas_camera::Camera, editor_view::ViewInput};
use egui::{PointerButton, Pos2, Rect, Response, Ui};
use pcbcraft_engine::{
    board::{
        setup::LayerPurpose,
        track::{DEFAULT_WIDTH_NM, MAX_PATH_POINTS, Track},
        view::{BoardView, CopperLayers},
    },
    schematic::Point,
};
use serde_json::json;
use std::collections::BTreeSet;
use uuid::Uuid;

/// Screen distance within which the pointer snaps to a pad center or a track end.
const SNAP_PX: f32 = 8.;
/// Minimum screen half-width used to pick a thin track.
const PICK_PX: f32 = 4.;

/// An unfinished route: fixed vertices on one layer, captured with its input authority.
#[derive(Clone, Debug)]
struct Draft {
    source: ViewInput,
    layer: String,
    points: Vec<Point>,
}

/// A route submitted this frame, painted until the next pass sees the engine's result.
#[derive(Clone, Debug)]
struct Committed {
    layer: String,
    width_nm: i64,
    points: Vec<Point>,
}

/// Independent route tool, width and track selection for one board view, never saved in files.
#[derive(Debug)]
pub(super) struct State {
    pub(super) tool: bool,
    pub(super) width_nm: i64,
    pub(super) selected: BTreeSet<Uuid>,
    pub(super) delete_requested: bool,
    pub(super) finish_requested: bool,
    pub(super) backspace_requested: bool,
    pub(super) posture_requested: bool,
    /// Leave each vertex on a 45° diagonal first, then run straight. Toggled with `/`.
    diagonal_first: bool,
    draft: Option<Draft>,
    committed: Option<Committed>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            tool: false,
            width_nm: DEFAULT_WIDTH_NM,
            selected: BTreeSet::new(),
            delete_requested: false,
            finish_requested: false,
            backspace_requested: false,
            posture_requested: false,
            diagonal_first: false,
            draft: None,
            committed: None,
        }
    }
}

/// What the canvas should do after routing input: whether it owned the input, and any edit.
#[derive(Default)]
pub(super) struct Interaction {
    pub(super) consumed: bool,
    pub(super) command: Option<Command>,
}

impl State {
    /// Arm the route tool with no authored changes; the first click captures revision and layer.
    pub(super) fn start(&mut self) {
        self.cancel();
        self.tool = true;
        self.selected.clear();
    }
    /// Drop the tool, any unfinished route and pending key requests. Selection and width remain.
    pub(super) fn cancel(&mut self) {
        self.tool = false;
        self.draft = None;
        self.delete_requested = false;
        self.finish_requested = false;
        self.backspace_requested = false;
        self.posture_requested = false;
    }
    /// Escape drops an unfinished route first, then exits the tool, then clears track selection.
    /// Returns false when there was nothing to escape.
    pub(super) fn escape(&mut self) -> bool {
        if self.draft.take().is_some() {
            return true;
        }
        if std::mem::take(&mut self.tool) {
            return true;
        }
        !std::mem::take(&mut self.selected).is_empty()
    }
    /// Whether the tool or an unfinished route owns canvas input.
    pub(super) fn busy(&self) -> bool {
        self.tool || self.draft.is_some()
    }
}

/// Whether the active layer is a visible copper layer of the board setup.
pub(super) fn can_route(view: BoardView<'_>, active: Option<&str>, hidden: &BTreeSet<String>) -> bool {
    view.setup().is_some_and(|setup| {
        setup.layers().iter().any(|layer| {
            layer.purpose == LayerPurpose::Copper && active == Some(layer.id.to_string().as_str()) && !hidden.contains(&layer.id.to_string())
        })
    })
}

/// Snap to a pad center or track end on the routing layer when the pointer is over or near it,
/// otherwise to this view's grid. The flag reports a copper target, which finishes a route.
fn snapped(pointer: Pos2, camera: &Camera, rect: Rect, view: BoardView<'_>, layer: &str) -> (Point, bool) {
    let on_layer = |layers: &Option<CopperLayers>| matches!(layers, Some(CopperLayers::Resolved { layers, .. }) if layers.iter().any(|id| id.to_string() == layer));
    let pads = view.footprints().flat_map(|footprint| footprint.pads).filter(|pad| on_layer(&pad.copper_layers)).filter_map(|pad| {
        let land = pad.copper?.map(|corner| camera.screen(corner, rect));
        let over = egui::Rect::from_points(&land).contains(pointer);
        let distance = camera.screen(pad.center, rect).distance(pointer);
        (over || distance <= SNAP_PX).then_some((if over { 0. } else { distance }, pad.center))
    });
    let ends = view.tracks().iter().filter(|track| track.layer.to_string() == layer).flat_map(|track| [track.start, track.end]).filter_map(|point| {
        let distance = camera.screen(point, rect).distance(pointer);
        (distance <= SNAP_PX).then_some((distance, point))
    });
    pads.chain(ends).min_by(|a, b| a.0.total_cmp(&b.0)).map_or_else(|| (camera.snap(pointer, rect), false), |(_, point)| (point, true))
}

/// The KiCad-style two-leg posture from `a` to `b`: one straight run along the major axis and one
/// 45° diagonal, in either order. Returns the new vertices after `a`, without duplicates.
fn leg(a: Point, b: Point, diagonal_first: bool) -> Vec<Point> {
    let (dx, dy) = (b.x_nm - a.x_nm, b.y_nm - a.y_nm);
    let (adx, ady) = (dx.abs(), dy.abs());
    if adx == 0 || ady == 0 || adx == ady {
        return vec![b];
    }
    let diagonal = adx.min(ady);
    let diag = (dx.signum() * diagonal, dy.signum() * diagonal);
    let corner = if diagonal_first { (a.x_nm + diag.0, a.y_nm + diag.1) } else { (b.x_nm - diag.0, b.y_nm - diag.1) };
    vec![Point { x_nm: corner.0, y_nm: corner.1 }, b]
}

/// Extend fixed vertices with a leg, skipping repeated points.
fn extended(points: &[Point], to: Point, diagonal_first: bool) -> Vec<Point> {
    let mut path = points.to_vec();
    if let Some(&last) = points.last() {
        for point in leg(last, to, diagonal_first) {
            if path.last() != Some(&point) {
                path.push(point);
            }
        }
    }
    path
}

/// Whether the pointer lies on a track's painted copper, with a minimum pick width for thin tracks.
fn hit(pointer: Pos2, camera: &Camera, rect: Rect, track: &Track) -> bool {
    let (a, b) = (camera.screen(track.start, rect), camera.screen(track.end, rect));
    let delta = b - a;
    let t = if delta.length_sq() > 0. { ((pointer - a).dot(delta) / delta.length_sq()).clamp(0., 1.) } else { 0. };
    pointer.distance(a + t * delta) <= (width_px(camera, rect, track.width_nm) / 2.).max(PICK_PX)
}

/// A board width in screen points at this view's zoom.
fn width_px(camera: &Camera, rect: Rect, width_nm: i64) -> f32 {
    (camera.screen(Point { x_nm: width_nm, y_nm: 0 }, rect) - camera.screen(Point { x_nm: 0, y_nm: 0 }, rect)).length()
}

/// Build the one route transaction for a finished path on the captured layer and width.
fn add(state: &mut State, draft: Draft) -> Option<Command> {
    if draft.points.len() < 2 {
        return None;
    }
    let width_nm = state.width_nm;
    state.committed = Some(Committed { layer: draft.layer.clone(), width_nm, points: draft.points.clone() });
    Some(Command {
        source: draft.source,
        id: pcbcraft_engine::ADD_TRACKS,
        params: json!({"layer":draft.layer,"width_nm":width_nm,"points":draft.points,"expected_revision":draft.source.revision}),
    })
}

/// Route on the active copper layer, or select tracks when no tool owns input. Release over
/// chrome, right-click, stale input, disabled input or a hidden/changed layer never commits an
/// unfinished route. Delete, Enter, Backspace and `/` requests are consumed every frame and
/// ignored while input is disabled, so they never act on a later frame. `over_footprint` gives footprints priority over tracks for plain picks.
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
    over_footprint: bool,
) -> Interaction {
    let mut out = Interaction::default();
    state.committed = None;
    let rect = response.rect;
    let visible: Vec<_> = view.tracks().iter().filter(|track| !hidden.contains(&track.layer.to_string())).collect();
    state.selected.retain(|id| visible.iter().any(|track| track.id == *id));
    if !enabled || state.draft.as_ref().is_some_and(|draft| Some(draft.source) != source || Some(draft.layer.as_str()) != active) {
        state.draft = None;
    }
    if state.tool && !can_route(view, active, hidden) {
        state.cancel();
    }
    // Key requests apply to this frame only. Taking them before the disabled-input return keeps
    // a Delete pressed while another tool owned input from deleting on a later frame.
    let delete = std::mem::take(&mut state.delete_requested);
    let posture = std::mem::take(&mut state.posture_requested);
    let backspace = std::mem::take(&mut state.backspace_requested);
    let finish = std::mem::take(&mut state.finish_requested);
    let Some(source) = source.filter(|_| enabled) else { return out };
    if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) && state.busy() {
        if state.draft.take().is_none() {
            state.tool = false;
        }
        out.consumed = true;
        return out;
    }
    if delete && !state.selected.is_empty() {
        out.command = Some(Command {
            source,
            id: pcbcraft_engine::DELETE_TRACKS,
            params: json!({"tracks":state.selected,"expected_revision":source.revision}),
        });
        state.selected.clear();
        out.consumed = true;
        return out;
    }
    let pointer = ui.input(|input| input.pointer.hover_pos()).filter(|point| rect.contains(*point));
    if state.tool {
        out.consumed = true;
        let Some(layer) = active else { return out };
        if posture {
            state.diagonal_first = !state.diagonal_first;
        }
        if backspace && let Some(draft) = &mut state.draft {
            draft.points.pop();
            if draft.points.is_empty() {
                state.draft = None;
            }
        }
        if finish {
            if let Some(draft) = state.draft.take() {
                out.command = add(state, draft);
            }
            return out;
        }
        let Some(pointer) = pointer else { return out };
        if !response.clicked() {
            return out;
        }
        let (at, copper) = snapped(pointer, camera, rect, view, layer);
        match state.draft.take() {
            None => state.draft = Some(Draft { source, layer: layer.to_owned(), points: vec![at] }),
            Some(mut draft) => {
                let path = extended(&draft.points, at, state.diagonal_first);
                if path.len() <= MAX_PATH_POINTS {
                    draft.points = path;
                }
                if (copper || response.double_clicked()) && draft.points.len() >= 2 {
                    out.command = add(state, draft);
                } else {
                    state.draft = Some(draft);
                }
            }
        }
        return out;
    }
    if response.clicked()
        && let Some(pointer) = pointer
    {
        let picked = (!over_footprint).then(|| visible.iter().rev().find(|track| hit(pointer, camera, rect, track))).flatten();
        let modifiers = ui.input(|input| input.modifiers);
        let extend = modifiers.shift || modifiers.ctrl || modifiers.command;
        if !extend {
            state.selected.clear();
        }
        if let Some(track) = picked {
            if (modifiers.ctrl || modifiers.command) && state.selected.contains(&track.id) {
                state.selected.remove(&track.id);
            } else {
                state.selected.insert(track.id);
            }
            out.consumed = true;
        }
    }
    out
}

/// Paint visible tracks with round ends at their physical width, inactive copper first so the
/// active layer reads on top. A route committed this frame is painted where it will land.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &Ui,
    rect: Rect,
    camera: &Camera,
    view: BoardView<'_>,
    state: &State,
    active: Option<&str>,
    hidden: &BTreeSet<String>,
    appearance: &crate::board_appearance::Appearance,
) {
    let painter = ui.painter().with_clip_rect(rect);
    let layers = view.layers();
    let color = |layer: &str| {
        layers
            .iter()
            .find(|record| record.id.to_string() == layer)
            .map_or(pcbcraft_chrome::Tokens::get().board_ink, |record| appearance.color(view, record))
    };
    let mut tracks: Vec<_> = view.tracks().iter().filter(|track| !hidden.contains(&track.layer.to_string())).collect();
    tracks.sort_by_key(|track| active == Some(track.layer.to_string().as_str()));
    for track in tracks {
        let layer = track.layer.to_string();
        pcbcraft_chrome::board::track(
            &painter,
            [camera.screen(track.start, rect), camera.screen(track.end, rect)],
            width_px(camera, rect, track.width_nm),
            color(&layer),
            state.selected.contains(&track.id),
            false,
        );
    }
    if let Some(committed) = &state.committed {
        for pair in committed.points.windows(2) {
            pcbcraft_chrome::board::track(
                &painter,
                [camera.screen(pair[0], rect), camera.screen(pair[1], rect)],
                width_px(camera, rect, committed.width_nm),
                color(&committed.layer),
                false,
                false,
            );
        }
    }
}

/// Paint the unfinished route above footprints: fixed vertices, the posture leg to the snapped
/// pointer, and a target marker. The preview does not imply any clearance check has passed.
pub(super) fn paint_draft(ui: &Ui, rect: Rect, camera: &Camera, view: BoardView<'_>, state: &State, active: Option<&str>) {
    let painter = ui.painter().with_clip_rect(rect);
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()).filter(|point| rect.contains(*point)) else { return };
    let Some(layer) = state.draft.as_ref().map(|draft| draft.layer.as_str()).or(active.filter(|_| state.tool)) else { return };
    let (at, _) = snapped(pointer, camera, rect, view, layer);
    if let Some(draft) = &state.draft {
        let path = extended(&draft.points, at, state.diagonal_first);
        for pair in path.windows(2) {
            pcbcraft_chrome::board::track(
                &painter,
                [camera.screen(pair[0], rect), camera.screen(pair[1], rect)],
                width_px(camera, rect, state.width_nm),
                pcbcraft_chrome::Tokens::get().accent,
                false,
                true,
            );
        }
    }
    pcbcraft_chrome::board::drawing_target(&painter, camera.screen(at, rect));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Legs are straight when aligned or exactly diagonal, and otherwise turn once at 45°.
    #[test]
    fn posture_legs_turn_once_at_45_degrees() {
        let p = |x, y| Point { x_nm: x, y_nm: y };
        assert_eq!(leg(p(0, 0), p(10, 0), false), vec![p(10, 0)]);
        assert_eq!(leg(p(0, 0), p(10, 10), false), vec![p(10, 10)]);
        assert_eq!(leg(p(0, 0), p(10, 4), false), vec![p(6, 0), p(10, 4)]);
        assert_eq!(leg(p(0, 0), p(10, 4), true), vec![p(4, 4), p(10, 4)]);
        assert_eq!(leg(p(0, 0), p(-3, -10), false), vec![p(0, -7), p(-3, -10)]);
        assert_eq!(extended(&[p(0, 0)], p(0, 0), false), vec![p(0, 0)], "a repeated click adds nothing");
    }
}
