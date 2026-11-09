//! Copper track and via routing and selection. Previews are view-local; every completed edit is an engine command.
use crate::{board_drawing::Command, canvas_camera::Camera, editor_view::ViewInput};
use egui::{PointerButton, Pos2, Rect, Response, Ui};
use pcbcraft_engine::{
    board::{
        setup::{LayerPurpose, Side},
        track::{DEFAULT_VIA_DIAMETER_NM, DEFAULT_VIA_DRILL_NM, DEFAULT_WIDTH_NM, MAX_PATH_POINTS, Track, Via},
        view::{BoardView, CopperLayers},
    },
    schematic::Point,
};
use serde_json::json;
use std::collections::BTreeSet;
use uuid::Uuid;

/// Screen distance within which the pointer snaps to a pad center, track end or via.
const SNAP_PX: f32 = 8.;
/// Minimum screen half-width used to pick a thin track or small via.
const PICK_PX: f32 = 4.;

/// An unfinished route captured with its input authority. Segment `i` runs from `points[i]` to
/// `points[i + 1]` on the layer in effect after every change at or before vertex `i`.
#[derive(Clone, Debug)]
struct Draft {
    source: ViewInput,
    /// The layer the first segment runs on.
    start: String,
    /// The layer the next segment will run on.
    layer: String,
    points: Vec<Point>,
    /// Vias at vertex indices, in increasing order, each switching to the given layer.
    changes: Vec<(usize, String)>,
}

/// The layer each segment of a `points`-vertex path runs on, starting on `start` and switching at
/// each change's vertex. Changes must be in increasing vertex order.
fn segment_layers<'a>(start: &'a str, changes: &'a [(usize, String)], points: usize) -> Vec<&'a str> {
    let mut layer = start;
    let mut changes = changes.iter().peekable();
    (0..points.saturating_sub(1))
        .map(|index| {
            if let Some((_, next)) = changes.next_if(|(at, _)| *at == index) {
                layer = next;
            }
            layer
        })
        .collect()
}

/// A route submitted this frame, painted until the next pass sees the engine's result.
#[derive(Clone, Debug)]
struct Committed {
    width_nm: i64,
    via_diameter_nm: i64,
    via_drill_nm: i64,
    segments: Vec<(String, Point, Point)>,
    vias: Vec<Point>,
}

/// Independent route tool, sizes and track/via selection for one board view, never saved in files.
#[derive(Debug)]
pub(super) struct State {
    pub(super) tool: bool,
    pub(super) width_nm: i64,
    pub(super) via_diameter_nm: i64,
    pub(super) via_drill_nm: i64,
    /// Selected track and via IDs; the two kinds never share an ID.
    pub(super) selected: BTreeSet<Uuid>,
    pub(super) delete_requested: bool,
    pub(super) finish_requested: bool,
    pub(super) backspace_requested: bool,
    pub(super) posture_requested: bool,
    pub(super) via_requested: bool,
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
            via_diameter_nm: DEFAULT_VIA_DIAMETER_NM,
            via_drill_nm: DEFAULT_VIA_DRILL_NM,
            selected: BTreeSet::new(),
            delete_requested: false,
            finish_requested: false,
            backspace_requested: false,
            posture_requested: false,
            via_requested: false,
            diagonal_first: false,
            draft: None,
            committed: None,
        }
    }
}

/// What the canvas should do after routing input: whether it owned the input, any edit, and a
/// layer the view should make active because a via moved the route there.
#[derive(Default)]
pub(super) struct Interaction {
    pub(super) consumed: bool,
    pub(super) command: Option<Command>,
    pub(super) switch_layer: Option<String>,
}

impl State {
    /// Arm the route tool with no authored changes; the first click captures revision and layer.
    pub(super) fn start(&mut self) {
        self.cancel();
        self.tool = true;
        self.selected.clear();
    }
    /// Drop the tool, any unfinished route and pending key requests. Selection and sizes remain.
    pub(super) fn cancel(&mut self) {
        self.tool = false;
        self.draft = None;
        self.delete_requested = false;
        self.finish_requested = false;
        self.backspace_requested = false;
        self.posture_requested = false;
        self.via_requested = false;
    }
    /// Escape drops an unfinished route first, then exits the tool, then clears the selection.
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

/// The visible copper layer a via from `layer` leads to: back from front, otherwise front.
/// None when the board has no such distinct visible layer.
fn paired_layer(view: BoardView<'_>, layer: &str, hidden: &BTreeSet<String>) -> Option<String> {
    let layers = view.setup()?.layers();
    let side = |side| layers.iter().find(|record| record.purpose == LayerPurpose::Copper && record.side == Some(side));
    let current = layers.iter().find(|record| record.id.to_string() == layer)?;
    let target = if current.side == Some(Side::Front) { side(Side::Back) } else { side(Side::Front) }?;
    let id = target.id.to_string();
    (id != layer && !hidden.contains(&id)).then_some(id)
}

/// Snap to a pad center, track end or via when the pointer is over or near it, otherwise to this
/// view's grid. Pads and track ends count only on the routing layer; vias reach every layer.
/// The flag reports a copper target, which finishes a route.
fn snapped(pointer: Pos2, camera: &Camera, rect: Rect, view: BoardView<'_>, layer: &str) -> (Point, bool) {
    let on_layer = |layers: &Option<CopperLayers>| matches!(layers, Some(CopperLayers::Resolved { layers, .. }) if layers.iter().any(|id| id.to_string() == layer));
    let pads = view.footprints().flat_map(|footprint| footprint.pads).filter(|pad| on_layer(&pad.copper_layers)).filter_map(|pad| {
        let land = pad.copper?.map(|corner| camera.screen(corner, rect));
        let over = egui::Rect::from_points(&land).contains(pointer);
        let distance = camera.screen(pad.center, rect).distance(pointer);
        (over || distance <= SNAP_PX).then_some((if over { 0. } else { distance }, pad.center))
    });
    let ends = view
        .tracks()
        .iter()
        .filter(|track| track.layer.to_string() == layer)
        .flat_map(|track| [track.start, track.end])
        .chain(view.vias().iter().map(|via| via.at))
        .filter_map(|point| {
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

/// Whether the pointer lies on a via's pad, with a minimum pick radius for small vias.
fn hit_via(pointer: Pos2, camera: &Camera, rect: Rect, via: &Via) -> bool {
    camera.screen(via.at, rect).distance(pointer) <= (width_px(camera, rect, via.diameter_nm) / 2.).max(PICK_PX)
}

/// A board length in screen points at this view's zoom.
fn width_px(camera: &Camera, rect: Rect, width_nm: i64) -> f32 {
    (camera.screen(Point { x_nm: width_nm, y_nm: 0 }, rect) - camera.screen(Point { x_nm: 0, y_nm: 0 }, rect)).length()
}

/// Build the one route transaction for a finished path, with its vias and the current sizes.
/// A path without a single segment produces nothing.
fn add(state: &mut State, draft: Draft) -> Option<Command> {
    if draft.points.len() < 2 {
        return None;
    }
    let layers = segment_layers(&draft.start, &draft.changes, draft.points.len());
    state.committed = Some(Committed {
        width_nm: state.width_nm,
        via_diameter_nm: state.via_diameter_nm,
        via_drill_nm: state.via_drill_nm,
        segments: draft.points.windows(2).zip(&layers).map(|(pair, layer)| ((*layer).to_owned(), pair[0], pair[1])).collect(),
        vias: draft.changes.iter().map(|(at, _)| draft.points[*at]).collect(),
    });
    let mut params = json!({"layer":draft.start,"width_nm":state.width_nm,"points":draft.points,"expected_revision":draft.source.revision});
    if !draft.changes.is_empty() {
        params["layer_changes"] = json!(draft.changes.iter().map(|(at, layer)| json!({"at":at,"layer":layer})).collect::<Vec<_>>());
        params["via_diameter_nm"] = json!(state.via_diameter_nm);
        params["via_drill_nm"] = json!(state.via_drill_nm);
    }
    Some(Command { source: draft.source, id: pcbcraft_engine::ADD_TRACKS, params })
}

/// Route on the active copper layer, or select tracks and vias when no tool owns input. V drops a
/// via at the pointer and continues on the paired layer, reported through `switch_layer`.
/// Release over chrome, right-click, stale input, disabled input or a hidden/changed layer never
/// commits an unfinished route. `over_footprint` gives footprints priority for plain picks.
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
    let vias = view.vias();
    state.selected.retain(|id| visible.iter().any(|track| track.id == *id) || vias.iter().any(|via| via.id == *id));
    if !enabled || state.draft.as_ref().is_some_and(|draft| Some(draft.source) != source || Some(draft.layer.as_str()) != active) {
        state.draft = None;
    }
    if state.tool && !can_route(view, active, hidden) {
        state.cancel();
    }
    let Some(source) = source.filter(|_| enabled) else { return out };
    if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) && state.busy() {
        if state.draft.take().is_none() {
            state.tool = false;
        }
        out.consumed = true;
        return out;
    }
    if std::mem::take(&mut state.delete_requested) && !state.selected.is_empty() {
        let (gone_vias, gone_tracks): (Vec<Uuid>, Vec<Uuid>) = state.selected.iter().partition(|id| vias.iter().any(|via| via.id == **id));
        let mut params = json!({"expected_revision":source.revision});
        if !gone_tracks.is_empty() {
            params["tracks"] = json!(gone_tracks);
        }
        if !gone_vias.is_empty() {
            params["vias"] = json!(gone_vias);
        }
        out.command = Some(Command { source, id: pcbcraft_engine::DELETE_TRACKS, params });
        state.selected.clear();
        out.consumed = true;
        return out;
    }
    let pointer = ui.input(|input| input.pointer.hover_pos()).filter(|point| rect.contains(*point));
    if state.tool {
        out.consumed = true;
        let Some(layer) = active else { return out };
        if std::mem::take(&mut state.posture_requested) {
            state.diagonal_first = !state.diagonal_first;
        }
        if std::mem::take(&mut state.backspace_requested)
            && let Some(draft) = &mut state.draft
        {
            // Undo the most recent step: a via at the last vertex first, then the vertex itself.
            if draft.changes.last().is_some_and(|(at, _)| *at + 1 == draft.points.len()) {
                draft.changes.pop();
                draft.layer = draft.changes.last().map_or_else(|| draft.start.clone(), |(_, layer)| layer.clone());
                out.switch_layer = Some(draft.layer.clone());
            } else {
                draft.points.pop();
                if draft.points.is_empty() {
                    state.draft = None;
                }
            }
        }
        if std::mem::take(&mut state.via_requested)
            && let Some(pointer) = pointer
            && let Some(draft) = &mut state.draft
            && let Some(next) = paired_layer(view, &draft.layer, hidden)
        {
            let (at, _) = snapped(pointer, camera, rect, view, &draft.layer);
            let path = extended(&draft.points, at, state.diagonal_first);
            let index = path.len() - 1;
            if index > 0 && path.len() <= MAX_PATH_POINTS && !draft.changes.iter().any(|(existing, _)| *existing == index) {
                draft.points = path;
                draft.changes.push((index, next.clone()));
                draft.layer = next.clone();
                out.switch_layer = Some(next);
            }
            return out;
        }
        if std::mem::take(&mut state.finish_requested) {
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
            None => state.draft = Some(Draft { source, start: layer.to_owned(), layer: layer.to_owned(), points: vec![at], changes: Vec::new() }),
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
        let picked = (!over_footprint)
            .then(|| {
                vias.iter()
                    .rev()
                    .find(|via| hit_via(pointer, camera, rect, via))
                    .map(|via| via.id)
                    .or_else(|| visible.iter().rev().find(|track| hit(pointer, camera, rect, track)).map(|track| track.id))
            })
            .flatten();
        let modifiers = ui.input(|input| input.modifiers);
        let extend = modifiers.shift || modifiers.ctrl || modifiers.command;
        if !extend {
            state.selected.clear();
        }
        if let Some(id) = picked {
            if (modifiers.ctrl || modifiers.command) && state.selected.contains(&id) {
                state.selected.remove(&id);
            } else {
                state.selected.insert(id);
            }
            out.consumed = true;
        }
    }
    out
}

/// Paint visible tracks with round ends at their physical width, inactive copper first so the
/// active layer reads on top, then vias above all copper. A route committed this frame is
/// painted where it will land.
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
    let segment = |layer: &str, a: Point, b: Point, width_nm: i64, selected: bool| {
        pcbcraft_chrome::board::track(
            &painter,
            [camera.screen(a, rect), camera.screen(b, rect)],
            width_px(camera, rect, width_nm),
            color(layer),
            selected,
            false,
        );
    };
    for track in tracks {
        segment(&track.layer.to_string(), track.start, track.end, track.width_nm, state.selected.contains(&track.id));
    }
    let via = |at: Point, diameter_nm: i64, drill_nm: i64, selected: bool, preview: bool| {
        pcbcraft_chrome::board::via(
            &painter,
            camera.screen(at, rect),
            width_px(camera, rect, diameter_nm) / 2.,
            width_px(camera, rect, drill_nm) / 2.,
            selected,
            preview,
        );
    };
    if let Some(committed) = &state.committed {
        for (layer, a, b) in &committed.segments {
            if !hidden.contains(layer) {
                segment(layer, *a, *b, committed.width_nm, false);
            }
        }
        for at in &committed.vias {
            via(*at, committed.via_diameter_nm, committed.via_drill_nm, false, false);
        }
    }
    for item in view.vias() {
        via(item.at, item.diameter_nm, item.drill_nm, state.selected.contains(&item.id), false);
    }
}

/// Paint the unfinished route above footprints: fixed vertices on their layers, vias, the posture
/// leg to the snapped pointer, and a target marker. The preview implies no clearance check.
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
        for (index, _) in &draft.changes {
            pcbcraft_chrome::board::via(
                &painter,
                camera.screen(draft.points[*index], rect),
                width_px(camera, rect, state.via_diameter_nm) / 2.,
                width_px(camera, rect, state.via_drill_nm) / 2.,
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

    /// Each segment takes the layer of the most recent via at or before its start vertex.
    #[test]
    fn segments_follow_their_layer_changes() {
        let changes = vec![(1, "bottom".to_owned()), (3, "top".to_owned())];
        assert_eq!(segment_layers("top", &changes, 5), vec!["top", "bottom", "bottom", "top"]);
        assert_eq!(segment_layers("top", &changes, 1), Vec::<&str>::new());
    }
}
