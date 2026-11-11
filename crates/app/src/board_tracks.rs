//! Copper track and via routing and selection. Previews are view-local; every completed edit is an engine command.
use crate::{
    board_appearance::{Appearance, CopperFocus, RouteContact},
    board_drawing::Command,
    canvas_camera::Camera,
    editor_view::ViewInput,
};
use egui::{PointerButton, Pos2, Rect, Response, Ui};
use pcbcraft_chrome::{
    MenuItem,
    menu::{self, MenuIcon},
    workspace::ToolIcon,
};
use pcbcraft_engine::{
    board::{
        drc::DEFAULT_CLEARANCE_NM,
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
/// A free routing point follows the pointer when the nearest grid point is farther away.
const GRID_SNAP_PX: f32 = 8.;
/// Minimum screen half-width used to pick a thin track or small via.
const PICK_PX: f32 = 4.;

/// An unfinished route captured with its input authority. Segment `i` runs from `points[i]` to
/// `points[i + 1]` on the layer in effect after every change at or before vertex `i`.
#[derive(Clone, Debug)]
struct Draft {
    source: ViewInput,
    /// Existing copper that stays emphasized until the draft ends.
    contact: Option<RouteContact>,
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

/// D: one segment follows the pointer while segments touching its ends on the same layer
/// stretch to stay attached, until a click commits the new ends.
#[derive(Clone, Debug)]
struct SegmentDrag {
    source: ViewInput,
    track: Uuid,
    anchor: Point,
    offset: (i64, i64),
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
    /// D over a segment: drag it with its attached neighbors stretching.
    pub(super) drag_segment_requested: bool,
    segment: Option<SegmentDrag>,
    /// New ends just submitted by a segment drag, painted until the engine applies them.
    committed_ends: Vec<(Uuid, Point, Point)>,
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
            drag_segment_requested: false,
            segment: None,
            committed_ends: Vec::new(),
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
        self.drag_segment_requested = false;
        self.segment = None;
    }
    /// Escape drops an unfinished route first, then exits the tool, then clears the selection.
    /// Returns false when there was nothing to escape.
    pub(super) fn escape(&mut self) -> bool {
        if self.draft.take().is_some() || self.moving.take().is_some() || self.segment.take().is_some() {
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

    /// The contact at the start of the unfinished route, if it started on copper.
    pub(super) fn route_contact(&self) -> Option<&RouteContact> {
        self.draft.as_ref()?.contact.as_ref()
    }
    /// Whether a tool or an unfinished route owns canvas input.
    pub(super) fn busy(&self) -> bool {
        self.tool || self.via_tool || self.draft.is_some() || self.moving.is_some() || self.segment.is_some()
    }
    /// The displacement to paint selected copper with: a move in progress, or one just submitted.
    fn shift(&self) -> Option<(i64, i64)> {
        self.moving.as_ref().map(|moving| moving.offset).or(self.committed_move)
    }
}

/// Which copper the selection filters allow picking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Kinds {
    pub(super) tracks: bool,
    pub(super) vias: bool,
}

impl Kinds {
    /// Every kind, for lookups that are not selection, such as net highlighting.
    pub(super) const ALL: Self = Self { tracks: true, vias: true };
}

/// The topmost visible via or track under a screen point, among the allowed kinds.
pub(super) fn copper_at(pointer: Pos2, camera: &Camera, rect: Rect, view: BoardView<'_>, hidden: &BTreeSet<String>, kinds: Kinds) -> Option<Uuid> {
    let via = view.vias().iter().rev().filter(|_| kinds.vias).find(|via| hit_via(pointer, camera, rect, via)).map(|via| via.id);
    via.or_else(|| {
        view.tracks()
            .iter()
            .rev()
            .filter(|track| kinds.tracks && !hidden.contains(&track.layer.to_string()))
            .find(|track| hit(pointer, camera, rect, track))
            .map(|track| track.id)
    })
}

/// Whether a press here would start a copper drag rather than a box selection.
pub(super) fn copper_under(pointer: Pos2, camera: &Camera, rect: Rect, view: BoardView<'_>, hidden: &BTreeSet<String>, kinds: Kinds) -> bool {
    copper_at(pointer, camera, rect, view, hidden, kinds).is_some()
}

/// Where `p` projects onto `track`, as a fraction from its start (0) to its end (1), and how far
/// it lies from that point in nanometers.
fn projection(p: Point, track: &Track) -> (f64, f64) {
    let (ax, ay, bx, by) = (track.start.x_nm as f64, track.start.y_nm as f64, track.end.x_nm as f64, track.end.y_nm as f64);
    let (dx, dy, px, py) = (bx - ax, by - ay, p.x_nm as f64 - ax, p.y_nm as f64 - ay);
    let length = dx * dx + dy * dy;
    let t = if length > 0. { ((px * dx + py * dy) / length).clamp(0., 1.) } else { 0. };
    (t, (px - t * dx).hypot(py - t * dy))
}

/// Whether `p`, on copper `width_nm` wide, overlaps `track`'s copper.
fn touches(p: Point, width_nm: i64, track: &Track) -> bool {
    projection(p, track).1 <= (width_nm + track.width_nm) as f64 / 2.
}

/// New ends for a segment drag of `track` by `offset`, listing only tracks that change.
///
/// The dragged segment moves by `offset`, except that an end meeting the middle of another
/// track on its layer is held there, so the segment pivots and that junction stays attached.
/// Every other track on the layer whose end touches the dragged segment, at one of its ends or
/// along it, follows that point to the same place on the moved segment. Empty when nothing
/// would move, such as when both ends are held.
fn dragged_ends(view: BoardView<'_>, track: Uuid, offset: (i64, i64)) -> Vec<(Uuid, Point, Point)> {
    let tracks = view.tracks();
    let Some(dragged) = tracks.iter().find(|t| t.id == track) else { return Vec::new() };
    let others: Vec<&Track> = tracks.iter().filter(|t| t.id != track && t.layer == dragged.layer).collect();
    // Ends of other tracks that touch the dragged segment stretch with it rather than hold it.
    let attached = |other: &Track| touches(other.start, other.width_nm, dragged) || touches(other.end, other.width_nm, dragged);
    let held = |p: Point| others.iter().any(|other| !attached(other) && touches(p, dragged.width_nm, other));
    let shift = |p: Point| if held(p) { (0, 0) } else { offset };
    let (from, to) = (shift(dragged.start), shift(dragged.end));
    if (from, to) == ((0, 0), (0, 0)) {
        return Vec::new();
    }
    let add = |p: Point, (dx, dy): (i64, i64)| Point { x_nm: p.x_nm.saturating_add(dx), y_nm: p.y_nm.saturating_add(dy) };
    // A point on the dragged segment moves by the blend of its ends' moves at its position.
    let follow = |p: Point| {
        let t = if p == dragged.start {
            0.
        } else if p == dragged.end {
            1.
        } else {
            projection(p, dragged).0
        };
        let blend = |a: i64, b: i64| a + ((b - a) as f64 * t).round() as i64;
        add(p, (blend(from.0, to.0), blend(from.1, to.1)))
    };
    let mut out = vec![(dragged.id, add(dragged.start, from), add(dragged.end, to))];
    for other in others {
        let ends = [other.start, other.end].map(|p| if touches(p, other.width_nm, dragged) { follow(p) } else { p });
        if ends != [other.start, other.end] {
            out.push((other.id, ends[0], ends[1]));
        }
    }
    out
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

/// Resolve the click target for a route. Nearby pads and track ends on its layer and through
/// vias keep their exact positions. A new route can also start along the body of a track.
/// Free points use a nearby grid point unless Ctrl is held.
fn route_target(
    pointer: Pos2,
    camera: &Camera,
    rect: Rect,
    view: BoardView<'_>,
    layer: &str,
    snap_to_grid: bool,
    starting: bool,
) -> (Point, Option<RouteContact>) {
    let layer = Uuid::try_parse(layer).ok();
    let on_layer = |layers: &Option<CopperLayers>| matches!((layer, layers), (Some(layer), Some(CopperLayers::Resolved { layers, .. })) if layers.contains(&layer));
    let pads = view.footprints().flat_map(|footprint| footprint.pads).filter(|pad| on_layer(&pad.copper_layers)).filter_map(|pad| {
        let land = pad.copper?.map(|corner| camera.screen(corner, rect));
        let over = egui::Rect::from_points(&land).contains(pointer);
        let distance = camera.screen(pad.center, rect).distance(pointer);
        (over || distance <= SNAP_PX).then_some((if over { 0. } else { distance }, pad.center, RouteContact::Pad(pad.net)))
    });
    let tracks = view.tracks().iter().filter(|track| Some(track.layer) == layer);
    let ends = tracks
        .clone()
        .flat_map(|track| [(track.start, track.id), (track.end, track.id)])
        .chain(view.vias().iter().map(|via| (via.at, via.id)))
        .filter_map(|(point, id)| {
            let distance = camera.screen(point, rect).distance(pointer);
            (distance <= SNAP_PX).then_some((distance, point, RouteContact::Copper(id)))
        });
    if let Some((_, point, contact)) = pads.chain(ends).min_by(|a, b| a.0.total_cmp(&b.0)) {
        return (point, Some(contact));
    }
    let free = camera.unsnapped(pointer, rect);
    if starting && let Some(track) = tracks.rev().find(|track| hit(pointer, camera, rect, track)) {
        let (along, _) = projection(free, track);
        let at = |a: i64, b: i64| a + ((b - a) as f64 * along).round() as i64;
        return (
            Point { x_nm: at(track.start.x_nm, track.end.x_nm), y_nm: at(track.start.y_nm, track.end.y_nm) },
            Some(RouteContact::Copper(track.id)),
        );
    }
    let grid = camera.snap(pointer, rect);
    (if snap_to_grid && camera.screen(grid, rect).distance(pointer) <= GRID_SNAP_PX { grid } else { free }, None)
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum RouteAction {
    Finish,
    Back,
    Via,
    Posture,
    Cancel,
}

/// Present the route's existing commands. Escape cancels the same step as the canvas shortcut.
fn route_menu(ui: &mut Ui, draft: Option<&Draft>, via: bool) -> Option<RouteAction> {
    let keys = egui::Modifiers { ctrl: ui.input(|input| input.modifiers.ctrl), ..egui::Modifiers::NONE };
    if ui.input_mut(|input| input.consume_key(keys, egui::Key::Escape)) {
        ui.close();
        return Some(RouteAction::Cancel);
    }
    if MenuItem::new("Finish track").tool_icon(ToolIcon::Track).shortcut("Enter").enabled(draft.is_some_and(|draft| draft.points.len() >= 2)).show(ui)
    {
        return Some(RouteAction::Finish);
    }
    if MenuItem::new("Remove last vertex").icon(MenuIcon::Undo).shortcut("Backspace").enabled(draft.is_some()).show(ui) {
        return Some(RouteAction::Back);
    }
    if MenuItem::new("Place through via").tool_icon(ToolIcon::Via).shortcut("V").enabled(draft.is_some() && via).show(ui) {
        return Some(RouteAction::Via);
    }
    if MenuItem::new("Flip track posture").tool_icon(ToolIcon::Track).shortcut("/").show(ui) {
        return Some(RouteAction::Posture);
    }
    menu::separator(ui);
    MenuItem::new("Cancel").icon(MenuIcon::Close).shortcut("Esc").show(ui).then_some(RouteAction::Cancel)
}

/// Where the routing popup opened, in the same screen coordinates as the canvas response.
fn menu_anchor(response: &Response) -> Option<Pos2> {
    let pointer = egui::Popup::position_of_id(&response.ctx, egui::Popup::default_response_id(response))?;
    Some(response.ctx.layer_transform_from_global(response.layer_id).map_or(pointer, |transform| transform * pointer))
}

/// Route on the active copper layer, or select tracks and vias when no tool owns input. V drops a
/// via at the pointer and continues on the paired layer, reported through `switch_layer`.
/// Release over chrome, stale input, disabled input or a hidden/changed layer never
/// commits an unfinished route. Key requests (Delete, Enter, Backspace, `/`, V) are consumed every
/// frame and ignored while input is disabled. `over_footprint` gives footprints priority for picks.
/// Selection and connected copper selection honor `kinds`; routing uses the full board geometry.
/// The route popup owns input through dismissal and uses its opening point for via placement.
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
    kinds: Kinds,
) -> Interaction {
    let mut out = Interaction::default();
    state.committed = None;
    let rect = response.rect;
    let visible: Vec<_> = view.tracks().iter().filter(|track| !hidden.contains(&track.layer.to_string())).collect();
    let vias = view.vias();
    let vias_visible = view.layers().iter().any(|layer| layer.purpose == LayerPurpose::Copper && !hidden.contains(&layer.id.to_string()));
    let selectable = |id: &Uuid| {
        (kinds.tracks && visible.iter().any(|track| track.id == *id)) || (kinds.vias && vias_visible && vias.iter().any(|via| via.id == *id))
    };
    state.selected.retain(selectable);
    let stale_route = state.draft.as_ref().is_some_and(|draft| Some(draft.source) != source || Some(draft.layer.as_str()) != active);
    let route_available = !state.tool || can_route(view, active, hidden);
    if state.tool && (!enabled || source.is_none() || stale_route || !route_available) && response.context_menu_opened() {
        egui::Popup::close_id(ui.ctx(), egui::Popup::default_response_id(response));
    }
    if !enabled || stale_route {
        state.draft = None;
    }
    if !route_available {
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
    let drag_segment = std::mem::take(&mut state.drag_segment_requested);
    state.committed_move = None;
    state.committed_ends.clear();
    if !enabled || state.segment.as_ref().is_some_and(|segment| Some(segment.source) != source) {
        state.segment = None;
    }
    if !enabled || state.moving.as_ref().is_some_and(|moving| Some(moving.source) != source) {
        state.moving = None;
    }
    let Some(source) = source.filter(|_| enabled) else { return out };
    if !state.tool && state.busy() && ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) {
        if state.moving.take().is_none() && state.segment.take().is_none() {
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
        let menu_open = response.context_menu_opened();
        let anchor = menu_anchor(response);
        let action = menu::context(response, |ui| route_menu(ui, state.draft.as_ref(), paired_layer(view, layer, hidden).is_some()))
            .and_then(|menu| menu.inner);
        if action == Some(RouteAction::Cancel) {
            state.escape();
            return out;
        }
        if action.is_none() && (menu_open || response.secondary_clicked()) {
            return out;
        }
        let pointer = if action == Some(RouteAction::Via) {
            anchor
        } else if response.clicked() {
            response.interact_pointer_pos()
        } else {
            response.hover_pos()
        };
        let pointer = pointer.filter(|point| rect.contains(*point));
        let snap_to_grid = !ui.input(|input| input.modifiers.ctrl);
        if posture || action == Some(RouteAction::Posture) {
            state.diagonal_first = !state.diagonal_first;
        }
        if (backspace || action == Some(RouteAction::Back))
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
        if (via || via_side.is_some() || action == Some(RouteAction::Via))
            && let Some(pointer) = pointer
            && let Some(draft) = &mut state.draft
            && let Some(next) = via_side.map_or_else(|| paired_layer(view, &draft.layer, hidden), |side| side_layer(view, side, hidden))
            && next != draft.layer
        {
            let (at, _) = route_target(pointer, camera, rect, view, &draft.layer, snap_to_grid, false);
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
        if finish || action == Some(RouteAction::Finish) {
            if let Some(draft) = state.draft.take() {
                out.command = add(state, draft);
            }
            return out;
        }
        let Some(pointer) = pointer else { return out };
        if action.is_some() || !response.clicked() {
            return out;
        }
        let (at, contact) = route_target(pointer, camera, rect, view, layer, snap_to_grid, state.draft.is_none());
        match state.draft.take() {
            None => {
                state.draft = Some(Draft { source, contact, start: layer.to_owned(), layer: layer.to_owned(), points: vec![at], changes: Vec::new() })
            }
            Some(mut draft) => {
                let path = extended(&draft.points, at, state.diagonal_first);
                if path.len() <= MAX_PATH_POINTS {
                    draft.points = path;
                }
                if (contact.is_some() || response.double_clicked()) && draft.points.len() >= 2 {
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
            .and_then(|pointer| copper_at(pointer, camera, rect, view, hidden, kinds))
            .map_or_else(|| state.selected.clone(), |id| BTreeSet::from([id]));
        if !seeds.is_empty() {
            state.selected = connected(view, &seeds);
            state.selected.retain(selectable);
            out.consumed = true;
            return out;
        }
    }
    if drag_segment
        && state.moving.is_none()
        && state.segment.is_none()
        && let Some(pointer) = pointer.filter(|_| !over_footprint)
        && let Some(track) = copper_at(pointer, camera, rect, view, hidden, Kinds { vias: false, ..kinds })
    {
        state.selected = BTreeSet::from([track]);
        state.segment = Some(SegmentDrag { source, track, anchor: camera.snap(pointer, rect), offset: (0, 0) });
        out.consumed = true;
        return out;
    }
    if let Some(segment) = &mut state.segment {
        out.consumed = true;
        if let Some(pointer) = pointer {
            let at = camera.snap(pointer, rect);
            segment.offset = (at.x_nm - segment.anchor.x_nm, at.y_nm - segment.anchor.y_nm);
        }
        if response.clicked() {
            let segment = segment.clone();
            state.segment = None;
            let ends = dragged_ends(view, segment.track, segment.offset);
            if !ends.is_empty() {
                state.committed_ends = ends.clone();
                let tracks: Vec<_> = ends.iter().map(|(id, start, end)| json!({"track":id,"start":start,"end":end})).collect();
                out.command = Some(Command {
                    source: segment.source,
                    id: pcbcraft_engine::RESHAPE_TRACKS,
                    params: json!({"tracks":tracks,"expected_revision":segment.source.revision}),
                });
            }
        }
        return out;
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
        && let Some(id) = copper_at(origin, camera, rect, view, hidden, kinds)
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
        let picked = (!over_footprint).then(|| copper_at(pointer, camera, rect, view, hidden, kinds)).flatten();
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

/// Resolve the layer's personal or default color, falling back to board ink for unknown IDs.
fn layer_color(view: BoardView<'_>, appearance: &Appearance, layer: &str) -> egui::Color32 {
    Uuid::try_parse(layer)
        .ok()
        .and_then(|id| view.setup()?.layers().iter().find(|record| record.id == id))
        .map_or_else(|| pcbcraft_chrome::Tokens::get().board_ink, |record| appearance.color(view, record))
}

/// Paint visible tracks with round ends at their physical width, inactive copper first so the
/// active layer reads on top, then vias above all copper. A route committed this frame is
/// painted where it will land, and `carried` tracks (stretched by a footprint move preview)
/// are painted at their carried ends.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &Ui,
    rect: Rect,
    camera: &Camera,
    view: BoardView<'_>,
    state: &State,
    active: Option<&str>,
    hidden: &BTreeSet<String>,
    appearance: &Appearance,
    focus: Option<CopperFocus<'_>>,
    carried: &[Track],
) {
    let painter = ui.painter().with_clip_rect(rect);
    let mut tracks: Vec<_> = view.tracks().iter().filter(|track| !hidden.contains(&track.layer.to_string())).collect();
    tracks.sort_by_key(|track| active == Some(track.layer.to_string().as_str()));
    let segment = |layer: &str, a: Point, b: Point, width_nm: i64, selected: bool, emphasized| {
        pcbcraft_chrome::board::track(
            &painter,
            [camera.screen(a, rect), camera.screen(b, rect)],
            width_px(camera, rect, width_nm),
            pcbcraft_chrome::board::copper_emphasis(layer_color(view, appearance, layer), emphasized),
            selected,
            false,
        );
    };
    let shift = state.shift();
    let moved = |point: Point, selected: bool| match shift.filter(|_| selected) {
        Some((dx, dy)) => Point { x_nm: point.x_nm.saturating_add(dx), y_nm: point.y_nm.saturating_add(dy) },
        None => point,
    };
    // A segment drag in progress, or one just submitted, draws its tracks at their new ends.
    let mut reshaped: Vec<(Uuid, Point, Point)> =
        state.segment.as_ref().map_or_else(|| state.committed_ends.clone(), |drag| dragged_ends(view, drag.track, drag.offset));
    reshaped.extend(carried.iter().map(|track| (track.id, track.start, track.end)));
    for track in tracks {
        let selected = state.selected.contains(&track.id);
        let (start, end) = reshaped
            .iter()
            .find(|(id, _, _)| *id == track.id)
            .map_or((moved(track.start, selected), moved(track.end, selected)), |(_, start, end)| (*start, *end));
        segment(&track.layer.to_string(), start, end, track.width_nm, selected, focus.map(|focus| focus.contains_copper(&track.id)));
    }
    let via = |at: Point, diameter_nm: i64, drill_nm: i64, selected: bool, emphasized| {
        pcbcraft_chrome::board::via(
            &painter,
            camera.screen(at, rect),
            width_px(camera, rect, diameter_nm) / 2.,
            width_px(camera, rect, drill_nm) / 2.,
            selected,
            false,
            emphasized,
        );
    };
    if let Some(committed) = &state.committed {
        for (layer, a, b) in &committed.segments {
            if !hidden.contains(layer) {
                segment(layer, *a, *b, committed.width_nm, false, None);
            }
        }
        for at in &committed.vias {
            via(*at, committed.via_diameter_nm, committed.via_drill_nm, false, None);
        }
    }
    for item in view.vias() {
        let selected = state.selected.contains(&item.id);
        via(moved(item.at, selected), item.diameter_nm, item.drill_nm, selected, focus.map(|focus| focus.contains_copper(&item.id)));
    }
}

/// Preview routing or standalone via placement while the tool owns the hovered canvas. Routing
/// replaces the system pointer with a live crosshair and marks the target used by the route leg.
/// While the menu is open the preview stays at its opening point and the system pointer returns.
/// Paint the clearance envelope behind the draft copper at the same width used by DRC.
pub(super) fn paint_draft(
    painter: &egui::Painter,
    response: &Response,
    camera: &Camera,
    view: BoardView<'_>,
    state: &State,
    active: Option<&str>,
    appearance: &Appearance,
) {
    let rect = response.rect;
    let menu_anchor = menu_anchor(response);
    let Some(pointer) = menu_anchor.or_else(|| response.hover_pos()).filter(|point| rect.contains(*point)) else { return };
    if state.via_tool {
        let at = camera.screen(copper_target(pointer, camera, rect, view), rect);
        pcbcraft_chrome::board::via(
            painter,
            at,
            width_px(camera, rect, state.via_diameter_nm) / 2.,
            width_px(camera, rect, state.via_drill_nm) / 2.,
            false,
            true,
            None,
        );
        pcbcraft_chrome::board::drawing_target(painter, at);
        return;
    }
    let Some(layer) = state.draft.as_ref().map(|draft| draft.layer.as_str()).or(active.filter(|_| state.tool)) else { return };
    let snap_to_grid = !painter.ctx().input(|input| input.modifiers.ctrl);
    let (at, contact) = route_target(pointer, camera, rect, view, layer, snap_to_grid, state.draft.is_none());
    if let Some(draft) = &state.draft {
        let path = extended(&draft.points, at, state.diagonal_first);
        let layers = segment_layers(&draft.start, &draft.changes, path.len());
        let path: Vec<_> = path.into_iter().map(|point| camera.screen(point, rect)).collect();
        let width = width_px(camera, rect, state.width_nm);
        let clearance = width_px(camera, rect, DEFAULT_CLEARANCE_NM);
        let via_width = width_px(camera, rect, state.via_diameter_nm);
        for pair in path.windows(2) {
            pcbcraft_chrome::board::routing_clearance(painter, [pair[0], pair[1]], width, clearance);
        }
        for (index, _) in &draft.changes {
            pcbcraft_chrome::board::routing_clearance(painter, [path[*index]; 2], via_width.max(4.0), clearance);
        }
        for (pair, layer) in path.windows(2).zip(layers) {
            pcbcraft_chrome::board::track(painter, [pair[0], pair[1]], width, layer_color(view, appearance, layer), false, true);
        }
        for (index, _) in &draft.changes {
            pcbcraft_chrome::board::via(painter, path[*index], via_width / 2., width_px(camera, rect, state.via_drill_nm) / 2., false, true, None);
        }
    }
    if menu_anchor.is_none() {
        painter.ctx().set_cursor_icon(egui::CursorIcon::None);
        pcbcraft_chrome::cursor::track(painter, pointer, camera.screen(at, rect), contact.is_some());
    }
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
