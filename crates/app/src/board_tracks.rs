//! Copper track and via routing and selection. Previews are view-local; every completed edit is an engine command.
use crate::{
    board_appearance::{Appearance, CopperFocus, RouteContact},
    board_drawing::Command,
    canvas_camera::{Camera, pointer_release},
    editor_view::ViewInput,
};
use egui::{PointerButton, Pos2, Rect, Response, Ui};
use pcbcraft_chrome::{
    MenuItem,
    menu::{self, MenuIcon},
    workspace::ToolIcon,
};
use pcbcraft_engine::{
    CommandError,
    board::{
        class_rules::Applied,
        route::{Mode, ViaMove},
        setup::{LayerPurpose, Side},
        track::replacements::Replacement,
        track::{DEFAULT_VIA_DIAMETER_NM, DEFAULT_VIA_DRILL_NM, DEFAULT_WIDTH_NM, MAX_PATH_POINTS, Track, Via},
        view::{BoardTarget, BoardView, CopperLayers},
    },
    schematic::Point,
};
use serde_json::json;
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

mod planning;
mod preparation;

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
    mode: Mode,
    /// Backspace edits the fixed path without adopting a displacement for it.
    shove_prefix_dirty: bool,
    merge_same_net: bool,
    replacements: Vec<Replacement>,
    via_moves: Vec<ViaMove>,
    /// Existing copper that stays emphasized until the draft ends.
    contact: Option<RouteContact>,
    /// The layer the first segment runs on.
    start: String,
    /// The layer the next segment will run on.
    layer: String,
    points: Vec<Point>,
    /// Vias at vertex indices, in increasing order, each switching to the given layer.
    changes: Vec<(usize, String)>,
    /// Captured class spacing for the preview, invalidated with the originating revision.
    clearance_nm: i64,
    /// Source-contact via policy remains attached to this draft's revision. A track can start
    /// with unresolved via defaults, but each actual via must resolve them before changing layer.
    via_rules: Option<Arc<Applied>>,
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

/// A keyboard or held-pointer segment drag owns the captured policy and proposed ends.
#[derive(Clone, Debug)]
struct SegmentDrag {
    source: ViewInput,
    layer: Option<String>,
    track: Uuid,
    mode: Mode,
    held: bool,
    previewed: bool,
    anchor: Point,
    offset: (i64, i64),
    ends: Vec<(Uuid, Point, Point)>,
}

/// A view-owned copy of the router settings for the modal.
#[derive(Debug)]
pub(super) struct RouterSettings {
    pub source: ViewInput,
    pub mode: Mode,
    pub merge_same_net: bool,
}

/// Independent route tool, sizes and track/via selection for one board view, never saved in files.
#[derive(Debug)]
pub(super) struct State {
    preparation: preparation::Preparation,
    planning: planning::Planning,
    mode: Mode,
    merge_same_net: bool,
    preview_target: Option<(Point, bool)>,
    snap: Option<(String, Point, RouteContact)>,
    snap_source: Option<ViewInput>,
    retry_route: Option<Draft>,
    rules_source: Option<(BoardTarget, Option<pcbcraft_engine::board::rules::Settings>)>,
    class_defaults: bool,
    width_override: Option<i64>,
    via_diameter_override: Option<i64>,
    via_drill_override: Option<i64>,
    pub(super) tool: bool,
    /// Place › Via: each click places one standalone via until Esc or right-click.
    pub(super) via_tool: bool,
    pub(super) via_target: Option<pcbcraft_engine::nets::ClassTarget>,
    pub(super) via_choices: Option<(BoardTarget, u64, Vec<crate::board_net_choice::NetChoice>)>,
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
            preparation: Default::default(),
            planning: Default::default(),
            mode: Mode::default(),
            merge_same_net: false,
            preview_target: None,
            snap: None,
            snap_source: None,
            retry_route: None,
            rules_source: None,
            class_defaults: false,
            width_override: None,
            via_diameter_override: None,
            via_drill_override: None,
            tool: false,
            via_tool: false,
            via_target: None,
            via_choices: None,
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
    pub(super) error: Option<String>,
}

/// Track commands queued by shortcuts for one canvas frame, even when input is disabled.
pub(super) struct KeyRequests {
    delete: bool,
    posture: bool,
    backspace: bool,
    finish: bool,
    via: bool,
    via_side: Option<Side>,
    move_copper: bool,
    select_connected: bool,
    drag_segment: bool,
}

impl State {
    /// Take this frame's shortcut requests without changing the active tool or selection.
    pub(super) fn take_key_requests(&mut self) -> KeyRequests {
        KeyRequests {
            delete: std::mem::take(&mut self.delete_requested),
            posture: std::mem::take(&mut self.posture_requested),
            backspace: std::mem::take(&mut self.backspace_requested),
            finish: std::mem::take(&mut self.finish_requested),
            via: std::mem::take(&mut self.via_requested),
            via_side: self.via_side_requested.take(),
            move_copper: std::mem::take(&mut self.move_requested),
            select_connected: std::mem::take(&mut self.connected_requested),
            drag_segment: std::mem::take(&mut self.drag_segment_requested),
        }
    }

    /// Change this view's routing policy before fixing vertices. Existing plans and prepared
    /// candidates are revoked together; a fixed prefix cannot silently change policy.
    pub(super) fn select_mode(&mut self, mode: Mode) -> bool {
        if !self.can_change_policy() {
            return false;
        }
        self.mode = mode;
        self.planning.cancel();
        self.preparation.cancel();
        if let Some(draft) = &mut self.draft {
            draft.mode = mode;
            draft.replacements.clear();
            draft.via_moves.clear();
        }
        true
    }
    /// Revoke speculative work when the upcoming gesture's same-net policy changes.
    pub(super) fn select_same_net_merge(&mut self, merge: bool) -> bool {
        if !self.can_change_policy() {
            return false;
        }
        self.merge_same_net = merge;
        self.planning.cancel();
        self.preparation.cancel();
        if let Some(draft) = &mut self.draft {
            draft.merge_same_net = merge;
            draft.replacements.clear();
            draft.via_moves.clear();
        }
        true
    }
    /// The current policy of this pane, independent of board document history.
    pub(super) fn policy(&self) -> (Mode, bool) {
        (self.mode, self.merge_same_net)
    }
    /// A held gesture, fixed prefix or latched publication keeps its captured policy.
    pub(super) fn can_change_policy(&self) -> bool {
        self.segment.is_none()
            && self.moving.is_none()
            && !self.preparation.committing()
            && !self.planning.committing()
            && self.draft.as_ref().is_none_or(|draft| draft.points.len() <= 1)
    }
    /// The visible gesture owns both its replacement tracks and moved via centers. A final
    /// route keeps its captured displacements until its single copper edit is published.
    pub(super) fn displacements(&self) -> (&[Replacement], &[ViaMove]) {
        if self.segment.is_some() || !self.committed_ends.is_empty() {
            return self.preparation.drag_displacements();
        }
        let publishing = self.retry_route.as_ref().filter(|_| self.preparation.committing());
        self.draft.as_ref().or(publishing).map_or((&[], &[]), |draft| self.planning.displacements(draft, self.width_nm))
    }
    /// Expose the displayed track paths for editor geometry assertions.
    #[cfg(test)]
    pub(super) fn replacements(&self) -> &[Replacement] {
        self.displacements().0
    }
    /// Count all displaced copper objects in the completed preview for replay assertions.
    #[cfg(any(test, feature = "performance-harness"))]
    pub(super) fn displacement_preview(&self) -> usize {
        let (tracks, vias) = self.displacements();
        tracks.len() + vias.len()
    }
    /// A held drag is ready to release once a frame has displayed the validated candidate.
    #[cfg(feature = "performance-harness")]
    pub(super) fn held_drag_previewed(&self) -> bool {
        self.segment.as_ref().is_some_and(|drag| drag.held && drag.previewed)
    }
    /// Retain an explicit ribbon width across pickups until board/default-rule replacement.
    pub(super) fn select_width(&mut self, width: i64) {
        self.width_nm = width;
        self.width_override = Some(width);
    }
    /// The current paired via control explicitly chooses both operands independently of width.
    pub(super) fn select_via(&mut self, diameter: i64, drill: i64) {
        self.via_diameter_nm = diameter;
        self.via_drill_nm = drill;
        self.via_diameter_override = Some(diameter);
        self.via_drill_override = Some(drill);
    }
    /// Adopt the contacted net's defaults once at route pickup, keeping geometry/rule traversal
    /// outside pointer-motion frames. Explicit dimensions retain priority and must meet hard
    /// limits; a later unclassified pickup restores only dimensions that still use defaults.
    fn contact_defaults(
        &mut self,
        board: BoardView<'_>,
        at: Point,
        layer: Option<Uuid>,
        via: bool,
    ) -> Result<(i64, Option<Arc<Applied>>), CommandError> {
        let base = board.effective_rules().values;
        let policy = if via && let Some(target) = &self.via_target { board.via_target_rules(target)? } else { board.route_rules_at(at, layer)? }
            .filter(|policy| policy.has_dimensions());
        let clearance = if let Some(policy) = &policy {
            if via {
                (self.via_diameter_nm, self.via_drill_nm) = policy.via_sizes(self.via_diameter_override, self.via_drill_override)?;
            } else {
                self.width_nm = policy.track_width(self.width_override)?;
                // Vias use explicit local sizes on commit; an unrelated via preference choice
                // does not prevent starting a track with a resolved width.
                if let Ok((diameter, drill)) = policy.via_sizes(self.via_diameter_override, self.via_drill_override) {
                    (self.via_diameter_nm, self.via_drill_nm) = (diameter, drill);
                }
            }
            policy.clearance_nm()?
        } else {
            if self.class_defaults {
                self.width_nm = self.width_override.unwrap_or(base.preferred_track_width_nm);
                self.via_diameter_nm = self.via_diameter_override.unwrap_or(base.preferred_via_diameter_nm);
                self.via_drill_nm = self.via_drill_override.unwrap_or(base.preferred_via_drill_nm);
            }
            base.clearance_nm
        };
        self.class_defaults = policy.is_some();
        Ok((clearance, policy.map(Arc::new)))
    }
    /// Refresh the shared board-net chooser once per source revision, outside repeated form painting.
    pub(super) fn refresh_net_choices(&mut self, board: BoardView<'_>, revision: u64) {
        if self.via_choices.as_ref().is_none_or(|(b, r, _)| *b != board.target || *r != revision) {
            self.via_choices =
                Some((board.target, revision, board.inspect_zone_nets().map(|rows| crate::board_net_choice::net_choices(&rows)).unwrap_or_default()));
        }
    }

    /// Refresh preferred sizes when switching boards or changing saved rules. Explicit local
    /// size choices survive ordinary edits; a rule change resets them and invalidates drafts.
    pub(super) fn sync_rules(&mut self, board: BoardView<'_>) {
        let key = (board.target, board.authored_rules().cloned());
        if self.rules_source.as_ref() == Some(&key) {
            return;
        }
        let target_changed = self.rules_source.as_ref().is_some_and(|(target, _)| *target != key.0);
        if target_changed {
            self.via_target = None;
            self.via_choices = None;
        }
        if target_changed || key.1.is_some() || self.rules_source.as_ref().is_some_and(|(_, rules)| rules.is_some()) {
            let values = board.effective_rules().values;
            self.width_nm = values.preferred_track_width_nm;
            self.via_diameter_nm = values.preferred_via_diameter_nm;
            self.via_drill_nm = values.preferred_via_drill_nm;
            self.width_override = None;
            self.via_diameter_override = None;
            self.via_drill_override = None;
            self.class_defaults = false;
        }
        self.rules_source = Some(key);
    }
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
        self.snap = None;
        self.planning.cancel();
        self.preparation.cancel();
        self.retry_route = None;
        self.committed = None;
        self.committed_ends.clear();
        self.committed_move = None;
        self.tool = false;
        self.via_tool = false;
        self.via_target = None;
        self.via_choices = None;
        self.draft = None;
        self.take_key_requests();
        self.moving = None;
        self.segment = None;
    }
    /// Escape drops an unfinished route first, then exits the tool, then clears the selection.
    /// Returns false when there was nothing to escape.
    pub(super) fn escape(&mut self) -> bool {
        self.snap = None;
        self.planning.cancel();
        if self.preparation.committing() {
            self.preparation.cancel();
            self.retry_route = None;
            self.committed = None;
            self.committed_ends.clear();
            self.committed_move = None;
            return true;
        }
        self.preparation.cancel();
        if self.draft.take().is_some() || self.moving.take().is_some() || self.segment.take().is_some() {
            return true;
        }
        if std::mem::take(&mut self.tool) | std::mem::take(&mut self.via_tool) {
            return true;
        }
        !std::mem::take(&mut self.selected).is_empty()
    }
    /// Reject a rule-invalid first click without disarming routing. A valid fixed prefix stays
    /// owned by its gesture even when the current head/target becomes obstructed. Revoking the
    /// planning and preparation generations prevents a late completion from adopting the click.
    fn release_blocked_pickup(&mut self) -> bool {
        if !self.draft.as_ref().is_some_and(|draft| draft.points.len() == 1) || !self.planning.start_blocked() {
            return false;
        }
        self.draft = None;
        self.snap = None;
        self.preview_target = None;
        self.planning.cancel();
        self.preparation.cancel();
        self.retry_route = None;
        self.committed = None;
        true
    }
    /// Whether a route is in progress, so layer keys should place a via instead.
    pub(super) fn routing(&self) -> bool {
        self.draft.is_some()
    }

    /// Fixed draft vertices, for regressions that distinguish placement from a guide preview.
    #[cfg(test)]
    pub(crate) fn fixed_route_points(&self) -> Option<&[Point]> {
        self.draft.as_ref().map(|draft| draft.points.as_slice())
    }

    /// The contact at the start of the unfinished route, if it started on copper.
    pub(super) fn route_contact(&self) -> Option<&RouteContact> {
        self.draft.as_ref()?.contact.as_ref()
    }
    /// Whether a tool, unfinished route or accepted pending commit owns canvas input.
    pub(super) fn busy(&self) -> bool {
        self.tool
            || self.via_tool
            || self.draft.is_some()
            || self.moving.is_some()
            || self.segment.is_some()
            || self.preparation.committing()
            || self.planning.committing()
    }
    /// Current preparation state for the focused route's status bar.
    pub(super) fn preparation_status(&self) -> Option<String> {
        if self.planning.status().is_some() && !self.planning.current_ready() {
            self.planning.status()
        } else {
            self.preparation.status().or_else(|| self.planning.status())
        }
    }
    /// Property controls remain read-only until the accepted copper edit is published or canceled.
    pub(super) fn committing(&self) -> bool {
        self.preparation.committing() || self.planning.committing()
    }
    /// Whether either route planning or copper preparation has an active job to advance.
    #[cfg(all(test, feature = "performance-harness"))]
    pub(crate) fn route_queue_busy(&self) -> bool {
        self.planning.queue_busy() || self.preparation.queue_busy()
    }

    /// The displacement to paint selected copper with: a move in progress, or one just submitted.
    fn shift(&self) -> Option<(i64, i64)> {
        self.moving.as_ref().map(|moving| moving.offset).or(self.committed_move)
    }
}

impl State {
    /// A retained contact cannot survive an edit, reopen or input-owner replacement.
    fn sync_snap_source(&mut self, source: Option<ViewInput>) {
        if self.snap_source != source {
            self.snap = None;
            self.snap_source = source;
        }
    }
    /// Keep a magnetic copper target through a wider release radius than its pickup radius.
    /// Revision/layer cancellation clears the latch, and free/grid points never latch.
    fn snap_target(
        &mut self,
        candidate: (Point, Option<RouteContact>),
        pointer: Pos2,
        camera: &Camera,
        rect: Rect,
        layer: &str,
    ) -> (Point, Option<RouteContact>) {
        if let Some((previous_layer, at, contact)) = &self.snap
            && previous_layer == layer
            && camera.screen(*at, rect).distance(pointer) <= SNAP_PX * 1.5
        {
            return (*at, Some(contact.clone()));
        }
        self.snap = candidate.1.as_ref().map(|contact| (layer.to_owned(), candidate.0, contact.clone()));
        candidate
    }
}

/// Which copper the selection filters allow picking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Kinds {
    pub(super) tracks: bool,
    vias: bool,
    include_locked: bool,
}

impl Kinds {
    /// Every kind, for lookups that are not selection, such as net highlighting.
    pub(super) const ALL: Self = Self { tracks: true, vias: true, include_locked: true };

    /// Copper kinds and lock states allowed by the addressed view.
    pub(super) fn from(filters: crate::selection_filters::Filters) -> Self {
        Self { tracks: filters.tracks, vias: filters.vias, include_locked: filters.include_locked }
    }

    /// Whether a track is selectable, independently of its layer visibility.
    fn track(self, track: &Track) -> bool {
        self.tracks && (self.include_locked || !track.locked)
    }

    /// Whether a via is selectable, independently of layer visibility.
    fn via(self, via: &Via) -> bool {
        self.vias && (self.include_locked || !via.locked)
    }
}

/// Check a selected ID in one ID-sorted kind. Disabled kinds need no record probes;
/// only a matched record needs its visibility or lock state checked.
fn selected_copper_is_selectable<T>(
    id: &Uuid,
    ordered: &[T],
    enabled: bool,
    id_of: impl FnMut(&T) -> Uuid,
    allowed: impl FnOnce(&T) -> bool,
) -> bool {
    enabled && ordered.binary_search_by_key(id, id_of).is_ok_and(|i| ordered.get(i).is_some_and(allowed))
}

/// Visible copper at a point, using paint order within each group and the active track layer first.
pub(super) fn copper_choices(
    pointer: Pos2,
    camera: &Camera,
    rect: Rect,
    view: BoardView<'_>,
    hidden: &BTreeSet<String>,
    kinds: Kinds,
    active: Option<&str>,
) -> impl Iterator<Item = Uuid> {
    let active = active.and_then(|id| Uuid::try_parse(id).ok());
    let track_hits = move |on_active| {
        view.tracks()
            .iter()
            .rev()
            .filter(move |track| {
                (Some(track.layer) == active) == on_active
                    && kinds.track(track)
                    && !hidden.contains(&track.layer.to_string())
                    && hit(pointer, camera, rect, track)
            })
            .map(|track| track.id)
    };
    view.vias()
        .iter()
        .rev()
        .filter(move |via| kinds.via(via) && hit_via(pointer, camera, rect, via))
        .map(|via| via.id)
        .chain(track_hits(true))
        .chain(track_hits(false))
}

/// Select the first visible copper candidate using the same order as keyboard cycling.
pub(super) fn copper_at(
    pointer: Pos2,
    camera: &Camera,
    rect: Rect,
    view: BoardView<'_>,
    hidden: &BTreeSet<String>,
    kinds: Kinds,
    active: Option<&str>,
) -> Option<Uuid> {
    copper_choices(pointer, camera, rect, view, hidden, kinds, active).next()
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
fn connected(view: BoardView<'_>, seeds: &BTreeSet<Uuid>) -> Result<BTreeSet<Uuid>, String> {
    let copper = view.copper()?;
    let islands: BTreeSet<_> = seeds.iter().filter_map(|id| copper.tracks.get(id).or_else(|| copper.vias.get(id))).copied().collect();
    Ok(copper.tracks.iter().chain(&copper.vias).filter(|(_, island)| islands.contains(island)).map(|(id, _)| *id).collect())
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
#[cfg(test)]
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
#[cfg(test)]
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
    state.planning.cancel();
    if draft.points.len() < 2 {
        return None;
    }
    state.retry_route = Some(draft.clone());
    let layers = segment_layers(&draft.start, &draft.changes, draft.points.len());
    state.committed = Some(Committed {
        width_nm: state.width_nm,
        via_diameter_nm: state.via_diameter_nm,
        via_drill_nm: state.via_drill_nm,
        segments: draft.points.windows(2).zip(&layers).map(|(pair, layer)| ((*layer).to_owned(), pair[0], pair[1])).collect(),
        vias: draft.changes.iter().map(|(at, _)| draft.points[*at]).collect(),
    });
    let mut params = json!({"layer":draft.start,"width_nm":state.width_nm,"points":draft.points,"expected_revision":draft.source.revision});
    if draft.mode != Mode::Walkaround {
        params["replacements"] = json!(draft.replacements);
        if !draft.via_moves.is_empty() {
            params["via_moves"] = json!(draft.via_moves);
        }
        params["merge_same_net"] = json!(draft.merge_same_net);
    }
    if !draft.changes.is_empty() {
        params["layer_changes"] = json!(draft.changes.iter().map(|(at, layer)| json!({"at":at,"layer":layer})).collect::<Vec<_>>());
        params["via_diameter_nm"] = json!(state.via_diameter_nm);
        params["via_drill_nm"] = json!(state.via_drill_nm);
    }
    Some(Command::new(draft.source, pcbcraft_engine::ADD_TRACKS, params))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RouteAction {
    Finish,
    Back,
    Via,
    Posture,
    Cancel,
    Highlight,
    Walkaround,
    Shove,
    ShoveVias,
    ToggleMerge,
}

/// Present the route's existing commands. Escape cancels the same step as the canvas shortcut.
fn route_menu(ui: &mut Ui, draft: Option<&Draft>, via: bool, mode: Mode, merge_same_net: bool) -> Option<RouteAction> {
    let keys = egui::Modifiers { ctrl: ui.input(|input| input.modifiers.ctrl), ..egui::Modifiers::NONE };
    if ui.input_mut(|input| input.consume_key(keys, egui::Key::Escape)) {
        ui.close();
        return Some(RouteAction::Cancel);
    }
    if MenuItem::new("Finish track").tool_icon(ToolIcon::Track).enabled(draft.is_some()).show(ui) {
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
    let can_change = draft.is_none_or(|draft| draft.points.len() == 1);
    if MenuItem::new("Highlight collisions").choice(mode == Mode::Highlight).enabled(can_change).show(ui) {
        return Some(RouteAction::Highlight);
    }
    if MenuItem::new("Walk around obstacles").choice(mode == Mode::Walkaround).enabled(can_change).show(ui) {
        return Some(RouteAction::Walkaround);
    }
    if MenuItem::new("Shove tracks").choice(mode == Mode::Shove).enabled(can_change).show(ui) {
        return Some(RouteAction::Shove);
    }
    if MenuItem::new("Shove tracks and vias").choice(mode == Mode::ShoveVias).enabled(can_change).show(ui) {
        return Some(RouteAction::ShoveVias);
    }
    if mode.shoves() && MenuItem::new("Merge same-net copper").toggle(merge_same_net).enabled(can_change).show(ui) {
        return Some(RouteAction::ToggleMerge);
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
/// frame and ignored while input is disabled. `over_pad` gives actual pad copper priority for picks.
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
    over_pad: bool,
    kinds: Kinds,
) -> Interaction {
    state.sync_rules(view);
    state.sync_snap_source(source);
    let mut out = Interaction::default();
    if !state.preparation.committing() {
        state.committed = None;
    }
    let rect = response.rect;
    let tracks = view.tracks();
    let vias = view.vias();
    let vias_visible =
        kinds.vias && view.layers().iter().any(|layer| layer.purpose == LayerPurpose::Copper && !hidden.contains(&layer.id.to_string()));
    let selectable = |id: &Uuid| {
        selected_copper_is_selectable(
            id,
            tracks,
            kinds.tracks,
            |track| track.id,
            |track| kinds.track(track) && !hidden.contains(&track.layer.to_string()),
        ) || selected_copper_is_selectable(id, vias, vias_visible, |via| via.id, |via| kinds.via(via))
    };
    state.selected.retain(selectable);
    let stale_route = state.draft.as_ref().is_some_and(|draft| Some(draft.source) != source || Some(draft.layer.as_str()) != active);
    let route_available = !state.tool || can_route(view, active, hidden);
    if state.tool && (!enabled || source.is_none() || stale_route || !route_available) && response.context_menu_opened() {
        egui::Popup::close_id(ui.ctx(), egui::Popup::default_response_id(response));
    }
    if !enabled || stale_route {
        state.planning.cancel();
        state.preparation.cancel();
        state.snap = None;
        state.draft = None;
        state.retry_route = None;
    }
    if !route_available {
        state.cancel();
    }
    // Key requests apply to this frame only. Taking them before the disabled-input return keeps
    // a Delete pressed while another gesture owned input from deleting on a later frame.
    let KeyRequests { delete, posture, backspace, finish, via, via_side, move_copper, select_connected, drag_segment } = state.take_key_requests();
    if state.preparation.committing() || state.planning.committing() {
        if finish || response.double_clicked() {
            state.planning.finish_pending_vertex();
        }
        if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) {
            state.escape();
        }
        out.consumed = true;
        return out;
    }
    state.committed_move = None;
    state.committed_ends.clear();
    if state.segment.as_ref().is_some_and(|segment| {
        !enabled || Some(segment.source) != source || segment.layer.as_deref() != active || active.is_some_and(|layer| hidden.contains(layer))
    }) {
        state.segment = None;
        state.preparation.cancel();
    }
    if !enabled || state.moving.as_ref().is_some_and(|moving| Some(moving.source) != source) {
        state.moving = None;
    }
    let Some(source) = source.filter(|_| enabled) else { return out };
    if !state.tool && state.busy() && ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) {
        state.preparation.cancel();
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
        out.command = Some(Command::new(source, pcbcraft_engine::DELETE_TRACKS, params));
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
            if let Err(error) = state.contact_defaults(view, at, None, true) {
                out.error = Some(error.to_string());
                return out;
            }
            state.committed = Some(Committed {
                width_nm: state.width_nm,
                via_diameter_nm: state.via_diameter_nm,
                via_drill_nm: state.via_drill_nm,
                segments: Vec::new(),
                vias: vec![at],
            });
            let mut params =
                json!({"at":at,"via_diameter_nm":state.via_diameter_nm,"via_drill_nm":state.via_drill_nm,"expected_revision":source.revision});
            if let Some(target) = &state.via_target {
                params["target"] = json!(target);
            }
            out.command = Some(Command::new(source, pcbcraft_engine::ADD_VIA, params));
        }
        return out;
    }
    if state.tool {
        out.consumed = true;
        let Some(layer) = active else { return out };
        let menu_open = response.context_menu_opened();
        let anchor = menu_anchor(response);
        let action = menu::context(response, |ui| {
            route_menu(ui, state.draft.as_ref(), paired_layer(view, layer, hidden).is_some(), state.mode, state.merge_same_net)
        })
        .and_then(|menu| menu.inner);
        if let Some(mode) = match action {
            Some(RouteAction::Highlight) => Some(Mode::Highlight),
            Some(RouteAction::Walkaround) => Some(Mode::Walkaround),
            Some(RouteAction::Shove) => Some(Mode::Shove),
            Some(RouteAction::ShoveVias) => Some(Mode::ShoveVias),
            _ => None,
        } {
            state.select_mode(mode);
            return out;
        }
        if action == Some(RouteAction::ToggleMerge) {
            state.select_same_net_merge(!state.merge_same_net);
            return out;
        }
        if action == Some(RouteAction::Cancel) {
            state.escape();
            return out;
        }
        if action.is_none() && (menu_open || response.secondary_clicked()) {
            return out;
        }
        // Routing owns this gesture, so a release that exceeded the generic click tolerance
        // still fixes a vertex. Other canvas tools retain their ordinary drag behavior.
        let place = response.clicked() || response.drag_stopped_by(PointerButton::Primary);
        let released = (place && action.is_none())
            .then(|| {
                ui.input(|input| {
                    input.events.iter().rev().find_map(|event| match event {
                        egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers } => Some((*pos, *modifiers)),
                        _ => None,
                    })
                })
            })
            .flatten();
        let pointer = if matches!(action, Some(RouteAction::Via | RouteAction::Finish)) {
            anchor
        } else if place {
            released.map(|(pos, _)| pos).or_else(|| response.interact_pointer_pos())
        } else {
            response.hover_pos()
        };
        let pointer = pointer.filter(|point| rect.contains(*point));
        let snap_to_grid = !released.map_or_else(|| ui.input(|input| input.modifiers.ctrl), |(_, modifiers)| modifiers.ctrl);
        if posture || action == Some(RouteAction::Posture) {
            state.diagonal_first = !state.diagonal_first;
        }
        if (backspace || action == Some(RouteAction::Back))
            && let Some(draft) = &mut state.draft
        {
            state.planning.cancel();
            state.preparation.cancel();
            draft.replacements.clear();
            draft.via_moves.clear();
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
            if let Some(draft) = &mut state.draft {
                draft.shove_prefix_dirty = draft.mode.shoves() && draft.points.len() >= 2;
            }
        }
        if (via || via_side.is_some() || action == Some(RouteAction::Via))
            && let Some(pointer) = pointer
            && let Some(draft) = &state.draft
            && let Some(next) = via_side.map_or_else(|| paired_layer(view, &draft.layer, hidden), |side| side_layer(view, side, hidden))
            && next != draft.layer
        {
            let layer = draft.layer.clone();
            let candidate = route_target(pointer, camera, rect, view, &layer, snap_to_grid, false);
            let (at, contact) = state.snap_target(candidate, pointer, camera, rect, &layer);
            state.preview_target = Some((at, contact.is_some()));
            let Some(draft) = &mut state.draft else { return out };
            if let Some(policy) = &draft.via_rules {
                match policy.via_sizes(state.via_diameter_override, state.via_drill_override) {
                    Ok((diameter, drill)) => {
                        state.via_diameter_nm = diameter;
                        state.via_drill_nm = drill;
                    }
                    Err(error) => {
                        out.error = Some(error.to_string());
                        return out;
                    }
                }
            }
            state.planning.request(
                draft,
                view.target,
                at,
                state.diagonal_first,
                (state.width_nm, state.via_diameter_nm, state.via_drill_nm),
                Some(planning::Action::Via(next)),
            );
            return out;
        }

        if action == Some(RouteAction::Finish)
            && let Some(pointer) = pointer
            && state.draft.is_some()
        {
            // The menu was opened on the canvas preview. Use the same target and
            // validation as a finishing click before adopting any displacements.
            let candidate = route_target(pointer, camera, rect, view, layer, snap_to_grid, false);
            let (at, _) = state.snap_target(candidate, pointer, camera, rect, layer);
            if let Some(draft) = &state.draft
                && draft.points.last() != Some(&at)
            {
                state.planning.request(
                    draft,
                    view.target,
                    at,
                    state.diagonal_first,
                    (state.width_nm, state.via_diameter_nm, state.via_drill_nm),
                    Some(planning::Action::Vertex { finish: true }),
                );
                state.planning.finish_pending_vertex();
                return out;
            }
        }

        if finish || action == Some(RouteAction::Finish) {
            if let Some(draft) = state.draft.take() {
                if draft.shove_prefix_dirty
                    && let Some(&head) = draft.points.last()
                {
                    state.planning.request(
                        &draft,
                        view.target,
                        head,
                        state.diagonal_first,
                        (state.width_nm, state.via_diameter_nm, state.via_drill_nm),
                        Some(planning::Action::Vertex { finish: true }),
                    );
                    state.draft = Some(draft);
                } else {
                    out.command = add(state, draft);
                }
            }
            return out;
        }
        let Some(pointer) = pointer else { return out };
        if action.is_some() || !place {
            return out;
        }
        let (at, contact) = route_target(pointer, camera, rect, view, layer, snap_to_grid, state.draft.is_none());
        let (at, contact) = state.snap_target((at, contact), pointer, camera, rect, layer);
        match state.draft.take() {
            None => {
                match Uuid::parse_str(layer)
                    .map_err(|_| "Unknown routing layer.".to_owned())
                    .and_then(|layer| state.contact_defaults(view, at, Some(layer), false).map_err(|error| error.to_string()))
                {
                    Ok((clearance_nm, via_rules)) => {
                        state.draft = Some(Draft {
                            source,
                            mode: state.mode,
                            shove_prefix_dirty: false,
                            merge_same_net: state.merge_same_net,
                            via_moves: Vec::new(),
                            replacements: Vec::new(),
                            contact,
                            start: layer.to_owned(),
                            layer: layer.to_owned(),
                            points: vec![at],
                            changes: Vec::new(),
                            clearance_nm,
                            via_rules,
                        })
                    }
                    Err(error) => out.error = Some(error),
                }
            }
            Some(draft) => {
                state.planning.request(
                    &draft,
                    view.target,
                    at,
                    state.diagonal_first,
                    (state.width_nm, state.via_diameter_nm, state.via_drill_nm),
                    Some(planning::Action::Vertex { finish: contact.is_some() || response.double_clicked() }),
                );
                if response.double_clicked() {
                    state.planning.finish_pending_vertex();
                }
                state.draft = Some(draft);
            }
        }
        return out;
    }
    if select_connected {
        let seeds = pointer
            .filter(|_| !over_pad)
            .and_then(|pointer| copper_at(pointer, camera, rect, view, hidden, kinds, active))
            .map_or_else(|| state.selected.clone(), |id| BTreeSet::from([id]));
        if !seeds.is_empty() {
            match connected(view, &seeds) {
                Ok(selected) => state.selected = selected,
                Err(error) => {
                    out.error = Some(error);
                    return out;
                }
            }
            state.selected.retain(selectable);
            out.consumed = true;
            return out;
        }
    }
    let held_pick = if state.moving.is_none() && state.segment.is_none() && response.drag_started_by(PointerButton::Primary) && !over_pad {
        ui.input(|input| input.pointer.press_origin())
            .and_then(|origin| copper_at(origin, camera, rect, view, hidden, kinds, active).map(|id| (origin, id)))
    } else {
        None
    };
    let track_pick = if drag_segment && state.moving.is_none() && state.segment.is_none() {
        pointer
            .filter(|_| !over_pad)
            .and_then(|pointer| copper_at(pointer, camera, rect, view, hidden, Kinds { vias: false, ..kinds }, active).map(|id| (pointer, id)))
    } else {
        held_pick
            .filter(|(_, id)| (state.selected.len() < 2 || !state.selected.contains(id)) && tracks.binary_search_by_key(id, |track| track.id).is_ok())
    };
    if let Some((origin, track)) = track_pick {
        state.selected = BTreeSet::from([track]);
        state.segment = Some(SegmentDrag {
            source,
            layer: active.map(str::to_owned),
            track,
            mode: state.mode,
            held: !drag_segment,
            previewed: false,
            anchor: camera.snap(origin, rect),
            offset: (0, 0),
            ends: Vec::new(),
        });
    }
    if let Some(segment) = &mut state.segment {
        out.consumed = true;
        let release = if segment.held { pointer_release(response, PointerButton::Primary) } else { None };
        let offset = release.or(pointer).map_or(segment.offset, |pointer| {
            let at = camera.snap(pointer, rect);
            (at.x_nm - segment.anchor.x_nm, at.y_nm - segment.anchor.y_nm)
        });
        let changed = segment.offset != offset;
        if changed {
            segment.offset = offset;
            segment.ends = dragged_ends(view, segment.track, offset);
            segment.previewed = false;
        }
        let released = release.is_some();
        let lost_pointer = segment.held && !released && !ui.input(|input| input.pointer.primary_down());
        let release_over_canvas = release.is_some_and(|position| {
            let ctx = &response.ctx;
            let global = ctx.layer_transform_to_global(response.layer_id).map_or(position, |transform| transform * position);
            response.interact_rect.contains(position) && ctx.layer_id_at(global) == Some(response.layer_id)
        });
        if lost_pointer || released && (!release_over_canvas || !segment.previewed || !state.preparation.ready()) {
            if release_over_canvas && !segment.ends.is_empty() {
                out.error = Some(state.preparation.refusal().unwrap_or("Hold the drag until its preview is validated.").into());
            }
            state.segment = None;
            state.preparation.cancel();
            return out;
        }
        let commit = if segment.held { released } else { response.clicked() };
        if commit || changed {
            if segment.ends.is_empty() {
                state.preparation.cancel();
            } else {
                let tracks: Vec<_> = segment.ends.iter().map(|(id, start, end)| json!({"track":id,"start":start,"end":end})).collect();
                let params = json!({"board":view.target.board,"tracks":tracks,"mode":segment.mode,"expected_revision":segment.source.revision});
                let edit = Command::new(segment.source, pcbcraft_engine::RESHAPE_TRACKS, params);
                if commit {
                    state.committed_ends = std::mem::take(&mut segment.ends);
                    out.command = Some(edit);
                } else {
                    let sizes = (state.width_nm, state.via_diameter_nm, state.via_drill_nm);
                    let _ = state.preparation.request(edit, active.map(str::to_owned), sizes, false);
                }
            }
            if commit {
                state.segment = None;
            }
        } else if state.preparation.ready() {
            // Copper painting follows interaction in this frame.
            segment.previewed = true;
        }
        return out;
    }
    if state.moving.is_none() && !state.selected.is_empty() && move_copper {
        // Without a pointer over the canvas (the ribbon was used), anchor on the selection itself.
        let anchor = pointer.map(|pointer| camera.snap(pointer, rect)).unwrap_or_else(|| {
            vias.iter()
                .find(|via| state.selected.contains(&via.id))
                .map(|via| via.at)
                .or_else(|| tracks.iter().find(|track| state.selected.contains(&track.id)).map(|track| track.start))
                .unwrap_or(Point { x_nm: 0, y_nm: 0 })
        });
        state.moving = Some(MoveDraft { source, anchor, offset: (0, 0), drag: false });
    }
    if let Some((origin, id)) = held_pick.filter(|_| state.moving.is_none()) {
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
                out.command = Some(Command::new(moving.source, pcbcraft_engine::MOVE_TRACKS, params));
            }
        }
        return out;
    }
    if response.clicked()
        && let Some(pointer) = pointer
    {
        let picked = (!over_pad).then(|| copper_at(pointer, camera, rect, view, hidden, kinds, active)).flatten();
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

/// Paint visible tracks with round ends at their physical width and draw the active layer above
/// other layers. Selected segments rise above ordinary tracks so overlapping hits can be seen.
/// Via rings remain above track copper. A route committed this frame is
/// painted where it will land, and `carried` tracks (stretched by a footprint move preview)
/// are painted at their carried ends. Net focus takes precedence over selection tinting,
/// matching pads and avoiding a second brightening of selected copper. Optional names come
/// from the view's immutable connectivity cache; paint never traverses schematic connectivity.
/// Returned annotations let later footprint copper suppress obscured names as whole words.
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
    names: Option<&std::collections::BTreeMap<Uuid, String>>,
    net_colors: Option<crate::board_appearance::NetColors<'_>>,
) -> pcbcraft_chrome::board::TrackLabels {
    let painter = ui.painter().with_clip_rect(rect);
    let mut labels = pcbcraft_chrome::board::TrackLabels::new(painter.clone());
    let mut tracks: Vec<_> = view.tracks().iter().filter(|track| !hidden.contains(&track.layer.to_string())).collect();
    let active_layer = active.and_then(|id| Uuid::try_parse(id).ok());
    tracks.sort_by_key(|track| (state.selected.contains(&track.id), Some(track.layer) == active_layer));
    let segment = |color, a: Point, b: Point, width_nm: i64, selected: bool, emphasized| {
        pcbcraft_chrome::board::track(
            &painter,
            [camera.screen(a, rect), camera.screen(b, rect)],
            width_px(camera, rect, width_nm),
            pcbcraft_chrome::board::copper_emphasis(color, emphasized),
            selected && emphasized.is_none(),
            false,
        );
    };
    let shift = state.shift();
    let moved = |point: Point, selected: bool| match shift.filter(|_| selected) {
        Some((dx, dy)) => Point { x_nm: point.x_nm.saturating_add(dx), y_nm: point.y_nm.saturating_add(dy) },
        None => point,
    };
    // A segment drag in progress, or one just submitted, draws its tracks at their new ends.
    let ends = state.segment.as_ref().map_or(state.committed_ends.as_slice(), |drag| drag.ends.as_slice());
    let reshaped = ends.iter().copied().chain(carried.iter().map(|track| (track.id, track.start, track.end)));
    let (replacements, visible_moves) = state.displacements();
    let replacements: std::collections::BTreeMap<_, _> = replacements.iter().map(|r| (r.track, r)).collect();
    for track in tracks {
        let color =
            net_colors.and_then(|palette| palette.track(&track.id)).unwrap_or_else(|| layer_color(view, appearance, &track.layer.to_string()));
        if let Some(replacement) = replacements.get(&track.id) {
            for leg in replacement.points.windows(2) {
                let ends = [camera.screen(leg[0], rect), camera.screen(leg[1], rect)];
                let width = width_px(camera, rect, track.width_nm);
                labels.cover_track(ends, width);
                pcbcraft_chrome::board::track(&painter, ends, width, color, false, true);
            }
            continue;
        }
        let selected = state.selected.contains(&track.id);
        let reshaped_track = reshaped.clone().find(|(id, _, _)| *id == track.id);
        let (start, end) = reshaped_track.map_or((moved(track.start, selected), moved(track.end, selected)), |(_, start, end)| (start, end));
        let ends = [camera.screen(start, rect), camera.screen(end, rect)];
        let width = width_px(camera, rect, track.width_nm);
        labels.cover_track(ends, width);
        if state.segment.is_some() && reshaped_track.is_some() && state.preparation.refusal().is_some() {
            pcbcraft_chrome::board::blocked_routing_track(&painter, ends, width);
        } else {
            segment(color, start, end, track.width_nm, selected, focus.map(|focus| focus.contains_copper(&track.id)));
        }
        if let Some(name) = names.and_then(|names| names.get(&track.id)) {
            labels.track(ends, width, name, color);
        }
    }
    if let Some(committed) = &state.committed {
        for (layer, a, b) in &committed.segments {
            if !hidden.contains(layer) {
                labels.cover_track([camera.screen(*a, rect), camera.screen(*b, rect)], width_px(camera, rect, committed.width_nm));
                pcbcraft_chrome::board::track(
                    &painter,
                    [camera.screen(*a, rect), camera.screen(*b, rect)],
                    width_px(camera, rect, committed.width_nm),
                    pcbcraft_chrome::Tokens::get().text_dim,
                    false,
                    true,
                );
            }
        }
        for at in &committed.vias {
            labels.cover_disc(camera.screen(*at, rect), width_px(camera, rect, committed.via_diameter_nm) / 2.);
            pcbcraft_chrome::board::via(
                &painter,
                camera.screen(*at, rect),
                width_px(camera, rect, committed.via_diameter_nm) / 2.,
                width_px(camera, rect, committed.via_drill_nm) / 2.,
                false,
                true,
                None,
            );
        }
    }
    let centers: std::collections::BTreeMap<_, _> = visible_moves.iter().map(|m| (m.via, m.at)).collect();
    for item in view.vias() {
        let selected = state.selected.contains(&item.id);
        let displaced = centers.get(&item.id);
        let center = displaced.copied().unwrap_or_else(|| moved(item.at, selected));
        labels.cover_disc(camera.screen(center, rect), width_px(camera, rect, item.diameter_nm) / 2.);
        pcbcraft_chrome::board::via(
            &painter,
            camera.screen(center, rect),
            width_px(camera, rect, item.diameter_nm) / 2.,
            width_px(camera, rect, item.drill_nm) / 2.,
            selected,
            displaced.is_some(),
            focus.map(|focus| focus.contains_copper(&item.id)),
        );
        let visible = view.layers().iter().any(|layer| layer.purpose == LayerPurpose::Copper && !hidden.contains(&layer.id.to_string()));
        if visible && let Some(name) = names.and_then(|names| names.get(&item.id)) {
            labels.via(camera.screen(center, rect), width_px(camera, rect, item.diameter_nm), name);
        }
    }
    labels
}

/// Resolve requests before copper painting, keeping displaced originals and replacement
/// paths in the same frame. Cold work stays in the bounded snapshot queue.
pub(super) fn prepare_preview(response: &Response, camera: &Camera, view: BoardView<'_>, state: &mut State, active: Option<&str>) {
    state.preview_target = None;
    let rect = response.rect;
    let Some(draft) = &state.draft else {
        return;
    };
    if let Some(pointer) = menu_anchor(response).or_else(|| response.hover_pos()).filter(|p| rect.contains(*p)) {
        let layer = draft.layer.clone();
        let snap_to_grid = !response.ctx.input(|input| input.modifiers.ctrl);
        let candidate = route_target(pointer, camera, rect, view, &layer, snap_to_grid, false);
        let (at, contact) = state.snap_target(candidate, pointer, camera, rect, &layer);
        state.preview_target = Some((at, contact.is_some()));
        if let Some(draft) = &state.draft {
            state.planning.request(draft, view.target, at, state.diagonal_first, (state.width_nm, state.via_diameter_nm, state.via_drill_nm), None);
        }
    } else if !state.planning.committing() {
        return;
    }
    if let Some(draft) = &state.draft {
        let Some(path) = state.planning.current_path() else {
            if !state.preparation.committing() {
                state.preparation.cancel();
            }
            return;
        };
        let mut changes = draft.changes.clone();
        if let Some(next) = state.planning.via_layer()
            && path.len() > 1
        {
            let at = path.len() - 1;
            if !changes.iter().any(|(existing, _)| *existing == at) {
                changes.push((at, next.to_owned()));
            }
        }
        // Queue the exact geometry already resolved for painting; do not project every pad
        // a second time merely to prepare the same pointer request.
        if let Some(plan) = state.planning.current_plan()
            && (2..=MAX_PATH_POINTS).contains(&path.len())
            && Some(draft.layer.as_str()) == active
        {
            let mut params = json!({"board":view.target.board,"layer":draft.start,"width_nm":state.width_nm,"points":path,"expected_revision":draft.source.revision});
            if draft.mode != Mode::Walkaround {
                params["replacements"] = json!(plan.replacements);
                if !plan.via_moves.is_empty() {
                    params["via_moves"] = json!(plan.via_moves);
                }
                params["merge_same_net"] = json!(draft.merge_same_net);
            }
            if !changes.is_empty() {
                params["layer_changes"] = json!(changes.iter().map(|(at, layer)| json!({"at":at,"layer":layer})).collect::<Vec<_>>());
                params["via_diameter_nm"] = json!(state.via_diameter_nm);
                params["via_drill_nm"] = json!(state.via_drill_nm);
            }
            if state
                .preparation
                .request(
                    Command::new(draft.source, pcbcraft_engine::ADD_TRACKS, params),
                    active.map(str::to_owned),
                    (state.width_nm, state.via_diameter_nm, state.via_drill_nm),
                    false,
                )
                .is_ok()
            {
                state.planning.mark_preparation_requested();
            }
        } else if !state.preparation.committing() {
            state.preparation.cancel();
        }
    }
}

/// Preview routing or standalone via placement while the tool owns the hovered canvas. Routing
/// replaces the system pointer with a live crosshair and marks the target used by the route leg.
/// While the menu is open the preview stays at its opening point and the system pointer returns.
/// Paint the clearance envelope behind the draft copper at the same width used by DRC.
/// The draft retains its layer color while preparation is pending; readiness is status, not
/// a different copper material that flashes on every pointer change. Validation uses the
/// current reached path and its endpoint via, independently of the retained display preview.
pub(super) fn paint_draft(
    painter: &egui::Painter,
    response: &Response,
    camera: &Camera,
    view: BoardView<'_>,
    state: &mut State,
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
    let layer = layer.to_owned();
    let (at, contacted) = state.preview_target.filter(|_| state.draft.is_some()).unwrap_or_else(|| {
        let snap_to_grid = !painter.ctx().input(|input| input.modifiers.ctrl);
        let candidate = route_target(pointer, camera, rect, view, &layer, snap_to_grid, state.draft.is_none());
        let (at, contact) = state.snap_target(candidate, pointer, camera, rect, &layer);
        (at, contact.is_some())
    });
    if let Some(draft) = &state.draft {
        let (path, plan_blocked) = state.planning.preview(draft, state.width_nm);
        let blocked =
            draft.mode == Mode::Highlight && (plan_blocked || (state.planning.validation_current() && state.preparation.refusal().is_some()));
        let mut changes = draft.changes.clone();
        if let Some(next) = state.planning.via_layer()
            && path.len() > 1
        {
            let at = path.len() - 1;
            if !changes.iter().any(|(existing, _)| *existing == at) {
                changes.push((at, next.to_owned()));
            }
        }
        let layers = segment_layers(&draft.start, &changes, path.len());
        let path: Vec<_> = path.into_iter().map(|point| camera.screen(point, rect)).collect();
        let width = width_px(camera, rect, state.width_nm);
        let clearance = width_px(camera, rect, draft.clearance_nm);
        let via_width = width_px(camera, rect, state.via_diameter_nm);
        for pair in path.windows(2) {
            pcbcraft_chrome::board::routing_clearance(painter, [pair[0], pair[1]], width, clearance);
        }
        for (index, _) in &changes {
            pcbcraft_chrome::board::routing_clearance(painter, [path[*index]; 2], via_width.max(4.0), clearance);
        }
        for (index, (pair, layer)) in path.windows(2).zip(layers).enumerate() {
            let ends = [pair[0], pair[1]];
            if blocked && index >= draft.points.len().saturating_sub(1) {
                pcbcraft_chrome::board::blocked_routing_track(painter, ends, width);
            } else {
                pcbcraft_chrome::board::track(painter, ends, width, layer_color(view, appearance, layer), false, true);
            }
        }
        for (index, _) in &changes {
            pcbcraft_chrome::board::via(painter, path[*index], via_width / 2., width_px(camera, rect, state.via_drill_nm) / 2., false, true, None);
        }
    }
    if menu_anchor.is_none() {
        painter.ctx().set_cursor_icon(egui::CursorIcon::None);
        pcbcraft_chrome::cursor::track(painter, pointer, camera.screen(at, rect), contacted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// The frame keeper must not probe a disabled kind or scan an enabled kind's sorted slice.
    #[test]
    fn frame_selection_probes_only_enabled_copper_by_id() {
        let copper: Vec<_> = (1..=4096).map(Uuid::from_u128).collect();
        let probes = Cell::new(0);
        let permissions = Cell::new(0);
        let keep = |id, enabled| {
            probes.set(0);
            permissions.set(0);
            selected_copper_is_selectable(
                &id,
                copper.as_slice(),
                enabled,
                |item| {
                    probes.set(probes.get() + 1);
                    *item
                },
                |item| {
                    permissions.set(permissions.get() + 1);
                    *item != Uuid::from_u128(4095)
                },
            )
        };

        assert!(!keep(Uuid::from_u128(4096), false));
        assert_eq!((probes.get(), permissions.get()), (0, 0), "a disabled kind must not be traversed");
        assert!(keep(Uuid::from_u128(4096), true));
        assert!((1..=16).contains(&probes.get()));
        assert_eq!(permissions.get(), 1);
        assert!(!keep(Uuid::from_u128(4095), true), "an existing item can be disallowed");
        assert!((1..=16).contains(&probes.get()));
        assert_eq!(permissions.get(), 1);
        assert!(!keep(Uuid::from_u128(4097), true), "an ID can belong to the other kind");
        assert!((1..=16).contains(&probes.get()));
        assert_eq!(permissions.get(), 0);
    }

    /// The target stays latched in the release band and changes only beyond it or on a new layer.
    #[test]
    fn magnetic_route_target_has_pickup_and_release_hysteresis() {
        let camera = Camera::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000., 800.));
        let a = Point { x_nm: 0, y_nm: 0 };
        let b = Point { x_nm: 1_000_000, y_nm: 0 };
        let at = camera.screen(a, rect);
        let mut state = State::default();
        state.snap_target((a, Some(RouteContact::Pad(Some("Net A".into())))), at, &camera, rect, "front");
        assert_eq!(state.snap_target((b, None), at + egui::vec2(10., 0.), &camera, rect, "front").0, a);
        assert_eq!(state.snap_target((b, None), at + egui::vec2(13., 0.), &camera, rect, "front").0, b);
        state.snap_target((a, Some(RouteContact::Pad(None))), at, &camera, rect, "front");
        assert_eq!(state.snap_target((b, None), at, &camera, rect, "back").0, b);
        state.snap_target((a, Some(RouteContact::Pad(None))), at, &camera, rect, "front");
        state.escape();
        assert_eq!(state.snap_target((b, None), at, &camera, rect, "front").0, b);
        let mut session = pcbcraft_engine::Session::default();
        session.apply("project.new", &json!({"create_board":true})).unwrap();
        let mut view = crate::editor_view::EditorView::new(crate::editor_view::ViewId(10));
        view.bind_opened_project(&session);
        state.sync_snap_source(view.input(&session));
        state.snap_target((a, Some(RouteContact::Pad(None))), at, &camera, rect, "front");
        session.apply("project.rename", &json!({"name":"Changed while hovering"})).unwrap();
        state.sync_snap_source(view.input(&session));
        assert_eq!(state.snap_target((b, None), at, &camera, rect, "front").0, b);
    }

    /// Board transitions reset local sizes without authored rules; initial synchronization and
    /// ordinary edits on the same board preserve an explicitly selected upcoming size.
    #[test]
    fn board_target_changes_reset_sizes_and_same_board_edits_preserve_choices() {
        let mut first = pcbcraft_engine::Session::default();
        let mut second = pcbcraft_engine::Session::default();
        for session in [&mut first, &mut second] {
            session.apply("project.new", &json!({"create_board":true})).unwrap();
        }
        let mut state = State { width_nm: 750_000, via_diameter_nm: 800_000, via_drill_nm: 400_000, ..Default::default() };
        let target = first.project().unwrap().board_target().unwrap();
        state.sync_rules(first.project().unwrap().board_view(target).unwrap());
        assert_eq!((state.width_nm, state.via_diameter_nm, state.via_drill_nm), (750_000, 800_000, 400_000));
        first.apply("project.rename", &json!({"name":"Same board"})).unwrap();
        state.sync_rules(first.project().unwrap().board_view(target).unwrap());
        assert_eq!((state.width_nm, state.via_diameter_nm, state.via_drill_nm), (750_000, 800_000, 400_000));
        let target = second.project().unwrap().board_target().unwrap();
        state.sync_rules(second.project().unwrap().board_view(target).unwrap());
        assert_eq!((state.width_nm, state.via_diameter_nm, state.via_drill_nm), (DEFAULT_WIDTH_NM, DEFAULT_VIA_DIAMETER_NM, DEFAULT_VIA_DRILL_NM));
        state.width_nm = 500_000;
        state.sync_rules(second.project().unwrap().board_view(target).unwrap());
        assert_eq!(state.width_nm, 500_000);
    }

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
