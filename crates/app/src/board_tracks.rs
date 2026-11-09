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

/// A rigid move of the selected tracks and vias, captured with its input authority. A drag
/// completes on release; an M move completes on the next click.
#[derive(Clone, Debug)]
struct MoveDraft {
    source: ViewInput,
    anchor: Point,
    offset: (i64, i64),
    drag: bool,
}

/// Independent route tool, sizes and track/via selection for one board view, never saved in files.
#[derive(Debug)]
pub(super) struct State {
    pub(super) tool: bool,
    /// Place › Via: each click places one standalone via until Esc or right-click.
    pub(super) via_tool: bool,
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
    /// PgUp/PgDn mid-route: drop a via and continue on that side's copper.
    pub(super) via_side_requested: Option<Side>,
    /// M with copper selected and no footprint selected: move the selected tracks and vias.
    pub(super) move_requested: bool,
    /// U: select every track and via joined by copper to the hovered or selected copper.
    pub(super) connected_requested: bool,
    moving: Option<MoveDraft>,
    /// The displacement just submitted for the selection, painted until the engine applies it.
    committed_move: Option<(i64, i64)>,
    /// Leave each vertex on a 45° diagonal first, then run straight. Toggled with `/`.
    diagonal_first: bool,
    draft: Option<Draft>,
    committed: Option<Committed>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            tool: false,
            via_tool: false,
            width_nm: DEFAULT_WIDTH_NM,
            via_diameter_nm: DEFAULT_VIA_DIAMETER_NM,
            via_drill_nm: DEFAULT_VIA_DRILL_NM,
            selected: BTreeSet::new(),
            delete_requested: false,
            finish_requested: false,
            backspace_requested: false,
            posture_requested: false,
            via_requested: false,
            via_side_requested: None,
            move_requested: false,
            connected_requested: false,
            moving: None,
            committed_move: None,
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
    /// Arm Place › Via with no authored changes; each click captures the current revision.
    pub(super) fn start_via(&mut self) {
        self.cancel();
        self.via_tool = true;
        self.selected.clear();
    }
    /// Drop the tool, any unfinished route and pending key requests. Selection and sizes remain.
    pub(super) fn cancel(&mut self) {
        self.tool = false;
        self.via_tool = false;
        self.draft = None;
        self.delete_requested = false;
        self.finish_requested = false;
        self.backspace_requested = false;
        self.posture_requested = false;
        self.via_requested = false;
        self.via_side_requested = None;
        self.move_requested = false;
        self.connected_requested = false;
        self.moving = None;
    }
    /// Escape drops an unfinished route first, then exits the tool, then clears the selection.
    /// Returns false when there was nothing to escape.
    pub(super) fn escape(&mut self) -> bool {
        if self.draft.take().is_some() || self.moving.take().is_some() {
            return true;
        }
        if std::mem::take(&mut self.tool) | std::mem::take(&mut self.via_tool) {
            return true;
        }
        !std::mem::take(&mut self.selected).is_empty()
    }
    /// Whether a route is in progress, so layer keys should place a via instead.
    pub(super) fn routing(&self) -> bool {
        self.draft.is_some()
    }
    /// Whether a tool or an unfinished route owns canvas input.
    pub(super) fn busy(&self) -> bool {
        self.tool || self.via_tool || self.draft.is_some() || self.moving.is_some()
    }
    /// The displacement to paint selected copper with: a move in progress, or one just submitted.
    fn shift(&self) -> Option<(i64, i64)> {
        self.moving.as_ref().map(|moving| moving.offset).or(self.committed_move)
    }
}

/// The topmost visible via or track under a screen point.
fn copper_at(pointer: Pos2, camera: &Camera, rect: Rect, view: BoardView<'_>, hidden: &BTreeSet<String>) -> Option<Uuid> {
    view.vias().iter().rev().find(|via| hit_via(pointer, camera, rect, via)).map(|via| via.id).or_else(|| {
        view.tracks()
            .iter()
            .rev()
            .filter(|track| !hidden.contains(&track.layer.to_string()))
            .find(|track| hit(pointer, camera, rect, track))
            .map(|track| track.id)
    })
}

/// Whether a press here would start a copper drag rather than a box selection.
pub(super) fn copper_under(pointer: Pos2, camera: &Camera, rect: Rect, view: BoardView<'_>, hidden: &BTreeSet<String>) -> bool {
    copper_at(pointer, camera, rect, view, hidden).is_some()
}

/// Every track and via sharing a copper island with any of `seeds`.
fn connected(view: BoardView<'_>, seeds: &BTreeSet<Uuid>) -> BTreeSet<Uuid> {
    let copper = view.copper();
    let islands: BTreeSet<_> = seeds.iter().filter_map(|id| copper.tracks.get(id).or_else(|| copper.vias.get(id))).copied().collect();
    copper.tracks.iter().chain(&copper.vias).filter(|(_, island)| islands.contains(island)).map(|(id, _)| *id).collect()
}

/// Whether the board has the two copper layers a through via needs.
pub(super) fn can_place_via(view: BoardView<'_>) -> bool {
    view.setup().is_some_and(|setup| setup.layers().iter().filter(|layer| layer.purpose == LayerPurpose::Copper).count() >= 2)
}

/// Snap to the nearest pad center, track end or via on any copper layer within reach of the
/// pointer, otherwise to this view's grid. Used to place vias and to measure.
pub(super) fn copper_target(pointer: Pos2, camera: &Camera, rect: Rect, view: BoardView<'_>) -> Point {
    view.footprints()
        .flat_map(|footprint| footprint.pads)
        .filter(|pad| matches!(pad.copper_layers, Some(CopperLayers::Resolved { .. })))
        .map(|pad| pad.center)
        .chain(view.tracks().iter().flat_map(|track| [track.start, track.end]))
        .chain(view.vias().iter().map(|via| via.at))
        .filter_map(|point| {
            let distance = camera.screen(point, rect).distance(pointer);
            (distance <= SNAP_PX).then_some((distance, point))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or_else(|| camera.snap(pointer, rect), |(_, point)| point)
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

/// The visible copper layer on one side of the board, if the setup has one.
fn side_layer(view: BoardView<'_>, side: Side, hidden: &BTreeSet<String>) -> Option<String> {
    view.setup()?
        .layers()
        .iter()
        .find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(side))
        .map(|layer| layer.id.to_string())
        .filter(|id| !hidden.contains(id))
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
/// commits an unfinished route. Key requests (Delete, Enter, Backspace, `/`, V) are consumed every
/// frame and ignored while input is disabled. `over_footprint` gives footprints priority for picks.
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
    let vias_visible = view.layers().iter().any(|layer| layer.purpose == LayerPurpose::Copper && !hidden.contains(&layer.id.to_string()));
    state.selected.retain(|id| visible.iter().any(|track| track.id == *id) || vias_visible && vias.iter().any(|via| via.id == *id));
    if !enabled || state.draft.as_ref().is_some_and(|draft| Some(draft.source) != source || Some(draft.layer.as_str()) != active) {
        state.draft = None;
    }
    if state.tool && !can_route(view, active, hidden) {
        state.cancel();
    }
    // Key requests apply to this frame only. Taking them before the disabled-input return keeps
    // a Delete pressed while another gesture owned input from deleting on a later frame.
    let delete = std::mem::take(&mut state.delete_requested);
    let posture = std::mem::take(&mut state.posture_requested);
    let backspace = std::mem::take(&mut state.backspace_requested);
    let finish = std::mem::take(&mut state.finish_requested);
    let via = std::mem::take(&mut state.via_requested);
    let via_side = state.via_side_requested.take();
    let move_copper = std::mem::take(&mut state.move_requested);
    let select_connected = std::mem::take(&mut state.connected_requested);
    state.committed_move = None;
    if !enabled || state.moving.as_ref().is_some_and(|moving| Some(moving.source) != source) {
        state.moving = None;
    }
    let Some(source) = source.filter(|_| enabled) else { return out };
    if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) && state.busy() {
        // Right-click ends the innermost gesture: a route or a copper move, else the tool.
        if state.draft.take().is_none() && state.moving.take().is_none() {
            state.tool = false;
            state.via_tool = false;
        }
        out.consumed = true;
        return out;
    }
    if delete && !state.selected.is_empty() {
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
    if state.via_tool {
        out.consumed = true;
        if !can_place_via(view) {
            state.cancel();
            return out;
        }
        if response.clicked()
            && let Some(pointer) = pointer
        {
            let at = copper_target(pointer, camera, rect, view);
            state.committed = Some(Committed {
                width_nm: state.width_nm,
                via_diameter_nm: state.via_diameter_nm,
                via_drill_nm: state.via_drill_nm,
                segments: Vec::new(),
                vias: vec![at],
            });
            out.command = Some(Command {
                source,
                id: pcbcraft_engine::ADD_VIA,
                params: json!({"at":at,"via_diameter_nm":state.via_diameter_nm,"via_drill_nm":state.via_drill_nm,"expected_revision":source.revision}),
            });
        }
        return out;
    }
    if state.tool {
        out.consumed = true;
        let Some(layer) = active else { return out };
        if posture {
            state.diagonal_first = !state.diagonal_first;
        }
        if backspace && let Some(draft) = &mut state.draft {
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
        if (via || via_side.is_some())
            && let Some(pointer) = pointer
            && let Some(draft) = &mut state.draft
            && let Some(next) = via_side.map_or_else(|| paired_layer(view, &draft.layer, hidden), |side| side_layer(view, side, hidden))
            && next != draft.layer
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
    if select_connected {
        let seeds = pointer
            .filter(|_| !over_footprint)
            .and_then(|pointer| copper_at(pointer, camera, rect, view, hidden))
            .map_or_else(|| state.selected.clone(), |id| BTreeSet::from([id]));
        if !seeds.is_empty() {
            state.selected = connected(view, &seeds);
            out.consumed = true;
            return out;
        }
    }
    if state.moving.is_none() && !state.selected.is_empty() && move_copper {
        // Without a pointer over the canvas (the ribbon was used), anchor on the selection itself.
        let anchor = pointer.map(|pointer| camera.snap(pointer, rect)).unwrap_or_else(|| {
            vias.iter()
                .find(|via| state.selected.contains(&via.id))
                .map(|via| via.at)
                .or_else(|| visible.iter().find(|track| state.selected.contains(&track.id)).map(|track| track.start))
                .unwrap_or(Point { x_nm: 0, y_nm: 0 })
        });
        state.moving = Some(MoveDraft { source, anchor, offset: (0, 0), drag: false });
    }
    if state.moving.is_none()
        && response.drag_started_by(PointerButton::Primary)
        && let Some(origin) = ui.input(|input| input.pointer.press_origin())
        && !over_footprint
        && let Some(id) = copper_at(origin, camera, rect, view, hidden)
    {
        if !state.selected.contains(&id) {
            state.selected = BTreeSet::from([id]);
        }
        state.moving = Some(MoveDraft { source, anchor: camera.snap(origin, rect), offset: (0, 0), drag: true });
    }
    if let Some(moving) = &mut state.moving {
        out.consumed = true;
        if let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) {
            let at = camera.snap(pointer, rect);
            moving.offset = (at.x_nm - moving.anchor.x_nm, at.y_nm - moving.anchor.y_nm);
        }
        let completed = if moving.drag { response.drag_stopped_by(PointerButton::Primary) } else { response.clicked() && !move_copper };
        let abandoned = !completed && moving.drag && !ui.input(|input| input.pointer.primary_down());
        // Copy the finished gesture out so the borrow ends before the move is cleared.
        if let Some(moving) = (completed || abandoned).then(|| moving.clone()) {
            state.moving = None;
            if completed && response.contains_pointer() && moving.offset != (0, 0) {
                let (gone_vias, gone_tracks): (Vec<Uuid>, Vec<Uuid>) = state.selected.iter().partition(|id| vias.iter().any(|via| via.id == **id));
                let mut params = json!({"dx_nm":moving.offset.0,"dy_nm":moving.offset.1,"expected_revision":moving.source.revision});
                if !gone_tracks.is_empty() {
                    params["tracks"] = json!(gone_tracks);
                }
                if !gone_vias.is_empty() {
                    params["vias"] = json!(gone_vias);
                }
                state.committed_move = Some(moving.offset);
                out.command = Some(Command { source: moving.source, id: pcbcraft_engine::MOVE_TRACKS, params });
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
    let shift = state.shift();
    let moved = |point: Point, selected: bool| match shift.filter(|_| selected) {
        Some((dx, dy)) => Point { x_nm: point.x_nm.saturating_add(dx), y_nm: point.y_nm.saturating_add(dy) },
        None => point,
    };
    for track in tracks {
        let selected = state.selected.contains(&track.id);
        segment(&track.layer.to_string(), moved(track.start, selected), moved(track.end, selected), track.width_nm, selected);
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
        let selected = state.selected.contains(&item.id);
        via(moved(item.at, selected), item.diameter_nm, item.drill_nm, selected, false);
    }
}

/// Paint the unfinished route above footprints: fixed vertices on their layers, vias, the posture
/// leg to the snapped pointer, and a target marker. The preview implies no clearance check.
pub(super) fn paint_draft(ui: &Ui, rect: Rect, camera: &Camera, view: BoardView<'_>, state: &State, active: Option<&str>) {
    let painter = ui.painter().with_clip_rect(rect);
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()).filter(|point| rect.contains(*point)) else { return };
    if state.via_tool {
        let at = camera.screen(copper_target(pointer, camera, rect, view), rect);
        pcbcraft_chrome::board::via(
            &painter,
            at,
            width_px(camera, rect, state.via_diameter_nm) / 2.,
            width_px(camera, rect, state.via_drill_nm) / 2.,
            false,
            true,
        );
        pcbcraft_chrome::board::drawing_target(&painter, at);
        return;
    }
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
