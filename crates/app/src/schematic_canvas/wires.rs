use egui::{Color32, Painter, PointerButton, Pos2, Rect, Response, Shape, Stroke};
use pcbcraft_chrome::{Tokens, theme};
use pcbcraft_engine::{
    Project,
    schematic::{Point, SymbolPlacement, WireEndpoint, orthogonal_path},
    wire::{Anchor, Reshape},
};

use super::Camera;
use std::cell::RefCell;

mod departure;
mod targets;

#[cfg(test)]
mod tests;

/// Uncommitted route corners and elbow preference; this never changes a project snapshot.
#[derive(Debug)]
pub(crate) struct Draft {
    corners: Vec<Vec<Point>>,
    target_cache: RefCell<targets::Cache>,
    pub(crate) horizontal_first: bool,
    pub(crate) finish_requested: bool,
    /// W begins at the pointer this frame; toolbar activation still waits for a click.
    pub(crate) start_requested: bool,
}

impl Default for Draft {
    fn default() -> Self {
        Self { target_cache: RefCell::default(), corners: Vec::new(), horizontal_first: true, finish_requested: false, start_requested: false }
    }
}

impl Draft {
    /// Resolve the frozen corners and current leg, retracting self-overlap with the same
    /// normalization as commit when enabled. Invalid or overly complex previews are refused.
    pub(crate) fn path(&self, from: Point, to: Point, auto_heal: bool) -> Option<Vec<Point>> {
        let previous = self.corners.last().cloned().unwrap_or_else(|| vec![from]);
        let path = orthogonal_path(previous.into_iter().chain(std::iter::once(to)), self.horizontal_first);
        if auto_heal { pcbcraft_engine::wire::normalized_path(&path).ok() } else { Some(path) }
    }

    /// Freeze a clicked corner if the resulting route fits the shared command's bound.
    pub(crate) fn corner(&mut self, from: Point, to: Point, auto_heal: bool) -> bool {
        let Some(path) = self.path(from, to, auto_heal) else { return false };
        if path.len() < 2 || path.len() > pcbcraft_engine::wire::MAX_BENDS + 2 {
            return false;
        }
        if self.corners.last() != Some(&path) {
            self.corners.push(path);
        }
        true
    }

    /// Remove the last clicked corner; false means the caller should clear the starting point.
    pub(crate) fn back(&mut self) -> bool {
        self.corners.pop().is_some()
    }
}

/// A reshape grabbed on a wire: what moves, the model point grabbed, and the snapped
/// pointer at the press. The grabbed point moves by the snapped pointer's travel, so a wire off
/// the grid stays off it by the same amount, as group moves do.
#[derive(Clone, Debug)]
pub(crate) struct ReshapeDrag {
    /// The wire used to grab the handle, including junctions shared by several routes.
    pub(crate) wire: String,
    pub(crate) edit: Reshape,
    pub(crate) grabbed: Point,
    /// A drag picked through an X retains marker selection rather than selecting its owner.
    pub(crate) marker: Option<pcbcraft_engine::no_connect::WireTarget>,
    /// A connected segment's orientation constrains its travel. Detached straight pieces
    /// have no axis constraint, like corners and free-end handles.
    horizontal: Option<bool>,
    origin: Point,
    /// G/context pickup finishes on a click; pointer dragging finishes on its held release.
    pub(crate) pickup: bool,
    /// Transient suppression of contacts occupied when a detached straight piece is grabbed.
    departure: std::rc::Rc<RefCell<departure::Departure>>,
}

impl ReshapeDrag {
    /// Capture geometry and the departure guard only after an unambiguous handle is picked.
    fn new(project: &Project, wire: &str, edit: Reshape, grabbed: Point, horizontal: Option<bool>, origin: Point) -> Self {
        let departure = std::rc::Rc::new(RefCell::new(departure::Departure::new(project, wire, &edit)));
        Self { wire: wire.to_owned(), edit, grabbed, marker: None, horizontal, origin, pickup: false, departure }
    }

    /// Resolve a drag destination using drawing's unambiguous pin/junction hit test for a
    /// movable endpoint. Other handles and empty space keep their snapped pointer travel.
    /// The source itself is never an attachment target; coincident candidates cancel the drop.
    /// Straight free pieces release their original-contact guard only after leaving its hit region.
    pub(crate) fn destination(&self, project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<Anchor> {
        if !rect.contains(pointer) {
            return None;
        }
        let point = self.target(camera.snap(pointer, rect));
        if !super::valid_position(point) {
            return None;
        }
        self.departure.borrow_mut().update(camera, rect, self.grabbed, point);
        if let Reshape::Junction(id) = self.edit {
            let suppressed = self.departure.borrow().suppressed_wires();
            let suppressed_pins = self.departure.borrow().suppressed_nodes();
            let anchor = targets::Targets::new(project, camera, rect).anchor_excluding(
                camera,
                rect,
                pointer,
                Some(WireEndpoint::Junction(id)),
                true,
                &suppressed,
                &suppressed_pins,
            )?;
            if !matches!(anchor, Anchor::Free(_)) {
                return Some(anchor);
            }
        }
        Some(Anchor::Free(point))
    }

    /// Validate a disposable preview with the same engine operation used on release.
    /// Junction handles use explicit targets; straight pieces can join exact end contacts after
    /// departure. Invalid geometry or ambiguous topology yields no preview.
    pub(crate) fn preview(&self, project: &Project, to: Anchor, revision: u64, auto_heal: bool) -> Option<Project> {
        match (&self.edit, to) {
            (Reshape::Junction(id), Anchor::Existing(_) | Anchor::Split { .. }) => project.attached(*id, to, revision, auto_heal).ok(),
            (_, Anchor::Free(point)) => project
                .reshaped(&self.edit, point)
                .and_then(|next| next.healed_with_connections(project, revision, auto_heal, &self.connection_ends()))
                .ok(),
            _ => None,
        }
    }

    /// Free ends eligible for exact wire contact on this drop. Original contacts are withheld
    /// until that end has left their hit region; returning later can deliberately reconnect.
    pub(crate) fn connection_ends(&self) -> Vec<u64> {
        if matches!(self.edit, Reshape::Segment { .. }) { self.departure.borrow().connection_ends() } else { Vec::new() }
    }

    /// Where the grabbed point goes with the pointer snapped at `pointer`: moved by the snapped
    /// travel. Connected segments move across their direction; detached pieces move in both axes.
    pub(crate) fn target(&self, pointer: Point) -> Point {
        let shift = |grabbed: i64, now: i64, then: i64| grabbed.saturating_add(now.saturating_sub(then));
        let mut to =
            Point { x_nm: shift(self.grabbed.x_nm, pointer.x_nm, self.origin.x_nm), y_nm: shift(self.grabbed.y_nm, pointer.y_nm, self.origin.y_nm) };
        match self.horizontal {
            Some(true) => to.x_nm = self.grabbed.x_nm,
            Some(false) => to.y_nm = self.grabbed.y_nm,
            None => {}
        }
        to
    }
}

/// A wire-end X is a handle for its owning junction. Capture the existing reshape operation,
/// retaining exact identity and grab offset; unknown, unmarked or foreign-sheet targets fail.
pub(super) fn marker_drag(project: &Project, marker: pcbcraft_engine::no_connect::WireTarget, origin: Point) -> Option<ReshapeDrag> {
    let end = marker.endpoint()?;
    if !project.has_no_connect_attachment(&pcbcraft_engine::no_connect::Attachment::WireEnd(marker.clone())) {
        return None;
    }
    let WireEndpoint::Junction(id) = end else {
        return None;
    };
    let point = project.endpoint_position(end)?;
    let wire = project.wires().iter().find(|wire| wire.endpoints().contains(&end))?;
    let mut drag = ReshapeDrag::new(project, &wire.id(), Reshape::Junction(id), point, None, origin);
    drag.marker = Some(marker);
    Some(drag)
}

/// What a press at `pointer` grabs on `wire`: one of its free ends or
/// junctions, then an interior corner, then a leg, each within the pin hit radius (the wire
/// stroke radius for legs). Pins can't be dragged off their parts, so terminal ends grab
/// nothing. `origin` is the snapped press point. `None` when nothing of that wire is there.
pub(super) fn reshape_at(project: &Project, camera: &Camera, rect: Rect, wire: &str, pointer: Pos2, origin: Point) -> Option<ReshapeDrag> {
    let found = project.wires().iter().find(|candidate| candidate.id() == wire)?;
    let route = project.wire_path(found);
    let near = |point: Point| camera.screen(point, rect).distance(pointer) <= theme::SCHEMATIC_HANDLE_HIT_RADIUS;
    let drag = |edit: Reshape, grabbed: Point, horizontal: Option<bool>| ReshapeDrag::new(project, wire, edit, grabbed, horizontal, origin);
    for end in found.endpoints() {
        if let WireEndpoint::Junction(id) = end
            && let Some(point) = project.endpoint_position(end)
            && near(point)
        {
            return Some(drag(Reshape::Junction(id), point, None));
        }
    }
    if let Some(&corner) = route.get(1..route.len().saturating_sub(1)).unwrap_or_default().iter().find(|&&corner| near(corner)) {
        return Some(drag(Reshape::Corner { wire: wire.to_owned(), at: corner }, corner, None));
    }
    let leg = leg_at(project, camera, rect, pointer, 0).filter(|leg| leg.wire == wire)?;
    let (a, b) = (leg.route[leg.leg], leg.route[leg.leg + 1]);
    let at = leg.inside();
    let constraint = (!project.wire_moves_freely(wire)).then_some(a.y_nm == b.y_nm);
    Some(drag(Reshape::Segment { wire: wire.to_owned(), at }, at, constraint))
}

/// Pick the nearest free endpoint, junction or corner across all wires using the node hit area,
/// without requiring a prior stroke selection. Several routes sharing one junction are one
/// handle; tied distinct identities are ambiguous and cannot start a move. Pins stay on parts.
pub(super) fn handle_at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, origin: Point) -> Result<Option<ReshapeDrag>, ()> {
    let mut best: Option<(String, Reshape, Point, f32)> = None;
    let mut ambiguous = false;
    let mut consider = |wire: &str, edit: Reshape, point: Point| {
        let distance = camera.screen(point, rect).distance_sq(pointer);
        if distance > theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2) {
            return;
        }
        if let Some((_, previous, _, nearest)) = &best {
            if (distance - nearest).abs() < 0.01 {
                ambiguous |= previous != &edit;
                return;
            }
            if distance > *nearest {
                return;
            }
        }
        best = Some((wire.to_owned(), edit, point, distance));
        ambiguous = false;
    };
    for wire in project.wires() {
        let id = wire.id();
        for end in wire.endpoints() {
            if let WireEndpoint::Junction(junction) = end
                && let Some(point) = project.endpoint_position(end)
            {
                consider(&id, Reshape::Junction(junction), point);
            }
        }
        let route = project.wire_path(wire);
        for &at in route.get(1..route.len().saturating_sub(1)).unwrap_or_default() {
            consider(&id, Reshape::Corner { wire: id.clone(), at }, at);
        }
    }
    if ambiguous {
        return Err(());
    }
    Ok(best.map(|(wire, edit, point, _)| ReshapeDrag::new(project, &wire, edit, point, None, origin)))
}

/// Draw the handles of the selected wire: a square on each interior corner, and an outlined
/// square on each free end or junction it ends on. Terminal ends have none; pins stay on parts.
pub(super) fn paint_handles(painter: &Painter, rect: Rect, project: &Project, camera: &Camera, wire: &str) {
    let tokens = Tokens::get();
    let Some(found) = project.wires().iter().find(|candidate| candidate.id() == wire) else {
        return;
    };
    let route = project.wire_path(found);
    let square = |point: Point| Rect::from_center_size(camera.screen(point, rect), egui::Vec2::splat(2.0 * theme::SCHEMATIC_HANDLE_HALF_SIZE));
    for &corner in route.get(1..route.len().saturating_sub(1)).unwrap_or_default() {
        painter.rect_filled(square(corner), 0.0, tokens.accent);
    }
    for end in found.endpoints() {
        if let WireEndpoint::Junction(_) = end
            && let Some(point) = project.endpoint_position(end)
        {
            painter.rect_filled(square(point), 0.0, tokens.canvas);
            painter.rect_stroke(square(point), 0.0, Stroke::new(1.5, tokens.accent), egui::StrokeKind::Middle);
        }
    }
}

/// Paint resolved paths, including a disposable symbol move, and free-point markers. As in
/// KiCad, a free point on one wire end is unconnected and drawn as a small open square; where two
/// wire ends meet the route reads as continuous, so nothing is drawn; three or more get a filled
/// junction dot.
/// Local selection takes precedence over a counterpart highlight; neither changes connectivity.
pub(super) fn paint(
    painter: &Painter,
    rect: Rect,
    project: &Project,
    camera: &Camera,
    moved: Option<(SymbolPlacement, Point)>,
    selected: &std::collections::BTreeSet<String>,
    highlighted: &std::collections::BTreeSet<String>,
) {
    let tokens = Tokens::get();
    let preview =
        moved.map(|(symbol, point)| (symbol.terminals(), symbol.unit_definition().pin_positions(point, symbol.turns(), symbol.is_mirrored())));
    let position = |endpoint| {
        if let WireEndpoint::Terminal(pin) = endpoint {
            preview
                .as_ref()
                .and_then(|(terminals, points)| terminals.iter().position(|&terminal| terminal == pin).and_then(|index| points.get(index).copied()))
                .or_else(|| project.terminal_position(pin))
        } else {
            project.endpoint_position(endpoint)
        }
    };
    for wire in project.wires() {
        let [Some(from), Some(to)] = wire.endpoints().map(position) else {
            continue;
        };
        let color = if selected.contains(&wire.id()) {
            tokens.accent
        } else if highlighted.contains(&wire.id()) {
            tokens.schematic_highlight
        } else {
            tokens.schematic_wire
        };
        paint_route(painter, rect, camera, pcbcraft_engine::wire::wire_path(wire, from, to), color);
    }
    let mut ends = std::collections::BTreeMap::<u64, usize>::new();
    for endpoint in project.wires().iter().flat_map(|wire| wire.endpoints()) {
        if let WireEndpoint::Junction(id) = endpoint {
            *ends.entry(id).or_default() += 1;
        }
    }
    for (id, point) in project.junctions() {
        let pos = camera.screen(point, rect);
        if !rect.contains(pos) {
            continue;
        }
        match ends.get(&id).copied().unwrap_or_default() {
            2 => {}
            0 | 1 if !project.net_labels().iter().any(|label| label.at == point) => paint_dangling(painter, pos, tokens.schematic_wire),
            0 | 1 => {}
            _ => {
                painter.circle_filled(pos, theme::SCHEMATIC_JUNCTION_RADIUS, tokens.schematic_wire);
            }
        }
    }
}

/// A wire leg captured under a right-click for the wire context menu.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LegTarget {
    pub(crate) wire: String,
    /// The wire's drawn route when the menu opened.
    pub(crate) route: Vec<Point>,
    /// Index of the leg's first point in `route`.
    pub(crate) leg: usize,
    /// The grid point where Slice splits the wire, if the leg has one away from both ends.
    pub(crate) split: Option<Point>,
    /// Whether the captured leg has a corner, pin or shared junction that Break can release.
    pub(crate) breakable: bool,
    /// The revision the menu was opened at; an edit after another change is refused.
    pub(crate) revision: u64,
}

impl LegTarget {
    /// Rebind a segment after a successful reshape by its new model point. Ambiguous or
    /// collapsed targets return None, so selection can clear without widening Delete's scope.
    pub(crate) fn at_model(project: &Project, wire: &str, at: Point, revision: u64) -> Option<Self> {
        let found = project.wires().iter().find(|candidate| candidate.id() == wire)?;
        let route = project.wire_path(found);
        let contains = |a: Point, b: Point| {
            let between = |a: i64, b: i64, at: i64| a.min(b) < at && at < a.max(b);
            (a.x_nm == b.x_nm && at.x_nm == a.x_nm && between(a.y_nm, b.y_nm, at.y_nm))
                || (a.y_nm == b.y_nm && at.y_nm == a.y_nm && between(a.x_nm, b.x_nm, at.x_nm))
        };
        let mut hits = route.windows(2).enumerate().filter(|(_, pair)| contains(pair[0], pair[1]));
        let leg = hits.next()?.0;
        if hits.next().is_some() || route.contains(&at) {
            return None;
        }
        Some(Self { wire: wire.to_owned(), route, leg, split: Some(at), breakable: project.wire_segment_can_break(wire, at), revision })
    }

    /// A point strictly inside the leg, naming it for Break and Delete Segment. Legs join grid points, so
    /// their midpoint is a whole number of nanometers.
    pub(crate) fn inside(&self) -> Point {
        let (a, b) = (self.route[self.leg], self.route[self.leg + 1]);
        Point { x_nm: a.x_nm + (b.x_nm - a.x_nm) / 2, y_nm: a.y_nm + (b.y_nm - a.y_nm) / 2 }
    }
}

/// Paint only the selected leg at its validated movement offset, including both axes for
/// a detached piece. The rest keeps ordinary ink so Delete's scope is visible before use.
pub(super) fn paint_selected_leg(painter: &Painter, rect: Rect, camera: &Camera, leg: &LegTarget, offset: Point) {
    let points =
        leg.route[leg.leg..=leg.leg + 1].iter().map(|point| Point { x_nm: point.x_nm + offset.x_nm, y_nm: point.y_nm + offset.y_nm }).collect();
    paint_stroke(painter, rect, camera, points, Stroke::new(2.0 * theme::SCHEMATIC_SYMBOL_STROKE, Tokens::get().accent));
}

/// Find the wire leg nearest the pointer, within the wire hit radius, and the grid point on it
/// where Slice would split: the snapped pointer projected onto the leg, kept away from the
/// route's two ends. A split point at a leg's end uses that corner if it is inside the route.
/// Returns `None` off every wire.
pub(super) fn leg_at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, revision: u64) -> Option<LegTarget> {
    let id = at(project, camera, rect, pointer)?;
    let wire = project.wires().iter().find(|wire| wire.id() == id)?;
    let route = project.wire_path(wire);
    let leg = route
        .windows(2)
        .map(|pair| distance_sq(pointer, camera.screen(pair[0], rect), camera.screen(pair[1], rect)))
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(&b.1))?
        .0;
    let (a, b) = (route[leg], route[leg + 1]);
    let snapped = camera.snap(pointer, rect);
    let on_leg = if a.x_nm == b.x_nm {
        Point { x_nm: a.x_nm, y_nm: snapped.y_nm.clamp(a.y_nm.min(b.y_nm), a.y_nm.max(b.y_nm)) }
    } else {
        Point { x_nm: snapped.x_nm.clamp(a.x_nm.min(b.x_nm), a.x_nm.max(b.x_nm)), y_nm: a.y_nm }
    };
    let ends = [route.first().copied(), route.last().copied()];
    let split = (!ends.contains(&Some(on_leg))).then_some(on_leg);
    let midpoint = Point { x_nm: a.x_nm + (b.x_nm - a.x_nm) / 2, y_nm: a.y_nm + (b.y_nm - a.y_nm) / 2 };
    let breakable = project.wire_segment_can_break(&id, midpoint);
    Some(LegTarget { wire: id, route, leg, split, breakable, revision })
}

/// Squared screen distance from `pointer` to the segment `a`–`b`.
fn distance_sq(pointer: Pos2, a: Pos2, b: Pos2) -> f32 {
    let delta = b - a;
    let t = ((pointer - a).dot(delta) / delta.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
    pointer.distance_sq(a + delta * t)
}

/// What the hovered row of a wire's menu will change: the whole route or one leg in the error
/// colour for deletion, a ring where Slice will split it, or the leg in the accent colour with
/// unconnected-end squares where Break will separate it. With no row hovered, the leg under the
/// click is drawn in the accent colour. Previews use a heavier stroke, so they stand out from
/// the selection highlight the right-click also gives the wire.
pub(super) fn paint_leg_preview(painter: &Painter, rect: Rect, camera: &Camera, target: &LegTarget, hovered: Option<super::WireAction>) {
    let tokens = Tokens::get();
    let paint_route = |painter: &Painter, rect: Rect, camera: &Camera, points: Vec<Point>, color: Color32| {
        paint_stroke(painter, rect, camera, points, Stroke::new(2.0 * theme::SCHEMATIC_SYMBOL_STROKE, color));
    };
    let leg = target.route[target.leg..=target.leg + 1].to_vec();
    match hovered {
        Some(super::WireAction::Delete) => paint_route(painter, rect, camera, target.route.clone(), tokens.error),
        Some(super::WireAction::SelectWire) => paint_route(painter, rect, camera, target.route.clone(), tokens.accent),
        Some(super::WireAction::Heal) => paint_route(painter, rect, camera, target.route.clone(), tokens.accent),
        Some(super::WireAction::DeleteSegment) => paint_route(painter, rect, camera, leg, tokens.error),
        Some(super::WireAction::Slice) => {
            if let Some(point) = target.split {
                let pos = camera.screen(point, rect);
                painter.circle_stroke(pos, theme::SCHEMATIC_BREAK_MARK_RADIUS, Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, tokens.accent));
            }
        }
        Some(super::WireAction::Break) => {
            paint_route(painter, rect, camera, leg.clone(), tokens.accent);
            for point in leg {
                paint_dangling(painter, camera.screen(point, rect), tokens.accent);
            }
        }
        None => paint_route(painter, rect, camera, leg, tokens.accent),
    }
}

/// Pick a connection or free point; Ctrl bypasses the placement grid. Enter/double-click finishes.
/// The crosshair tracks the raw pointer while a marker shows the snapped point a click would use.
#[allow(clippy::too_many_arguments)]
pub(super) fn pick(
    painter: &Painter,
    rect: Rect,
    response: &Response,
    project: &Project,
    camera: &Camera,
    start: Option<Anchor>,
    draft: &Draft,
    auto_heal: bool,
    revision: u64,
) -> Option<(Anchor, bool)> {
    let clicked = response.clicked_by(PointerButton::Primary);
    let pointer = if clicked { response.interact_pointer_pos() } else { response.hover_pos() };
    let target = preview(painter, rect, project, camera, start, draft, pointer, auto_heal, revision);
    if response.hovered()
        && let Some(pointer) = pointer
    {
        let point = target.and_then(|anchor| anchor.position(project)).unwrap_or_else(|| {
            if painter.ctx().input(|input| input.modifiers.ctrl) { camera.unsnapped(pointer, rect) } else { camera.snap(pointer, rect) }
        });
        let allowed =
            target.is_none_or(|anchor| draft.target_cache.borrow_mut().get(project, revision, camera, rect).connection_allowed(project, anchor));
        if allowed {
            painter.ctx().set_cursor_icon(egui::CursorIcon::None);
            let connected = target.is_some_and(|anchor| !matches!(anchor, Anchor::Free(_)));
            pcbcraft_chrome::cursor::wire(painter, pointer, camera.screen(point, rect), connected);
        } else {
            painter.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        }
    }
    let finish = draft.finish_requested || response.double_clicked_by(PointerButton::Primary);
    target.filter(|_| clicked || finish || (start.is_none() && draft.start_requested)).map(|target| (target, finish))
}

/// Highlight unambiguous endpoints and preview the full route. Read Ctrl each frame so
/// pressing or releasing the grid override updates the target without a pointer move.
#[allow(clippy::too_many_arguments)]
pub(super) fn preview(
    painter: &Painter,
    rect: Rect,
    project: &Project,
    camera: &Camera,
    start: Option<Anchor>,
    draft: &Draft,
    pointer: Option<Pos2>,
    auto_heal: bool,
    revision: u64,
) -> Option<Anchor> {
    let tokens = Tokens::get();
    let mut cache = draft.target_cache.borrow_mut();
    let targets = cache.get(project, revision, camera, rect);
    let snap_to_grid = !painter.ctx().input(|input| input.modifiers.ctrl);
    let target = pointer.filter(|pos| rect.contains(*pos)).and_then(|at| targets.anchor(camera, rect, at, None, snap_to_grid));
    if let Some(from) = start.and_then(|anchor| anchor.position(project))
        && let Some(to) = target.and_then(|anchor| anchor.position(project))
        && let Some(path) = draft.path(from, to, auto_heal)
    {
        paint_route(
            painter,
            rect,
            camera,
            path,
            if target.is_some_and(|anchor| targets.connection_allowed(project, anchor)) { tokens.accent } else { tokens.error },
        );
    }
    for &(endpoint, pos) in &targets.endpoints {
        let anchor = Anchor::Existing(endpoint);
        if !targets.connection_allowed(project, anchor) {
            continue;
        }
        let active = start == Some(anchor) || target == Some(anchor);
        paint_pin(painter, pos, if active { tokens.schematic_wire } else { tokens.canvas_ink });
    }
    target
}

/// Project visible targets for drawing/dragging. A named marker takes priority at an exact
/// coincident contact, because it deliberately joins those identities; unrelated ties remain ambiguous.
fn endpoints(project: &Project, camera: &Camera, rect: Rect) -> Vec<(WireEndpoint, Pos2)> {
    let mut named = std::collections::BTreeMap::new();
    for label in project.net_labels() {
        named.entry((label.at.x_nm, label.at.y_nm)).or_insert(WireEndpoint::NetLabel(label.id));
    }
    for (end, _, at) in project.power_terminals() {
        named.entry((at.x_nm, at.y_nm)).or_insert(end);
    }
    project
        .symbols()
        .iter()
        .flat_map(|symbol| symbol.terminals().into_iter().map(WireEndpoint::Terminal).zip(symbol.pins()))
        .chain(project.hierarchy_endpoints())
        .chain(project.junctions().map(|(id, point)| (WireEndpoint::Junction(id), point)))
        .filter(|(end, at)| named.get(&(at.x_nm, at.y_nm)).is_none_or(|preferred| preferred == end))
        .map(|(endpoint, point)| (endpoint, camera.screen(point, rect)))
        .filter(|(_, pos)| rect.contains(*pos))
        .collect()
}

/// Find the nearest endpoint inside the magnetic radius. Equal-distance identities return
/// an error; an empty hit returns `Ok(None)` so wiring may try a stroke or grid point.
fn nearest_node(endpoints: &[(WireEndpoint, Pos2)], at: Pos2) -> Result<Option<WireEndpoint>, ()> {
    let mut nearest = None;
    let mut distance = theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2);
    let mut ambiguous = false;
    for &(endpoint, pos) in endpoints {
        let candidate = at.distance_sq(pos);
        if candidate < distance {
            nearest = Some(endpoint);
            distance = candidate;
            ambiguous = false;
        } else if candidate == distance && nearest.is_some() {
            ambiguous = true;
        }
    }
    if ambiguous { Err(()) } else { Ok(nearest) }
}

/// Place a named anchor at an existing contact before falling back to the snap grid. This
/// preserves off-grid pins and wire contacts, reusing the revision-bound target cache.
pub(super) fn label_position(
    project: &Project,
    camera: &Camera,
    rect: Rect,
    pointer: Pos2,
    draft: &Draft,
    revision: u64,
    ignore: Option<WireEndpoint>,
) -> Point {
    draft
        .target_cache
        .borrow_mut()
        .get(project, revision, camera, rect)
        .anchor(camera, rect, pointer, ignore, true)
        .and_then(|anchor| anchor.position(project))
        .unwrap_or_else(|| camera.snap(pointer, rect))
}

/// Align the closest pin of any placement template with an explicit contact. Ambiguous
/// equal-distance targets retain grid snapping; no body or library-name special case is used.
#[allow(clippy::too_many_arguments)]
pub(super) fn symbol_position(
    project: &Project,
    definition: &pcbcraft_engine::schematic::SymbolUnit,
    turns: u8,
    camera: &Camera,
    rect: Rect,
    pointer: Pos2,
    draft: &Draft,
    revision: u64,
) -> Point {
    let mut cache = draft.target_cache.borrow_mut();
    let targets = cache.get(project, revision, camera, rect);
    let mut best = None;
    let mut ambiguous = false;
    for pin in definition.pins() {
        let (x, y) = pcbcraft_engine::schematic::rotate(pin.offset.x_nm, pin.offset.y_nm, turns);
        let offset = egui::vec2(x as f32, y as f32) * (camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32);
        let pin_at = pointer + offset;
        let Some(anchor) = targets.anchor(camera, rect, pin_at, None, true).filter(|a| !matches!(a, Anchor::Free(_))) else { continue };
        let Some(at) = anchor.position(project) else { continue };
        let candidate = Point { x_nm: at.x_nm - x, y_nm: at.y_nm - y };
        let distance = camera.screen(at, rect).distance_sq(pin_at);
        match best {
            Some((old, old_distance)) if distance == old_distance && candidate != old => ambiguous = true,
            Some((_, old_distance)) if distance >= old_distance => {}
            _ => {
                best = Some((candidate, distance));
                ambiguous = false;
            }
        }
    }
    if ambiguous { camera.snap(pointer, rect) } else { best.map_or_else(|| camera.snap(pointer, rect), |(at, _)| at) }
}

/// Resolve an unconnected pin or free wire end for automatic starts. Occupied or marked
/// endpoints keep normal selection behavior. W can start at occupied contacts, while marked
/// endpoints require removing their no-connect intent first.
/// Equal-distance nodes are ambiguous and cannot start a connection.
pub(super) fn node_at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, draft: &Draft, revision: u64) -> Option<Anchor> {
    if !rect.contains(pointer) {
        return None;
    }
    draft.target_cache.borrow_mut().get(project, revision, camera, rect).automatic_node(pointer)
}

/// Mark a validated attachment destination using the same pin ring as the wire drawing tool.
pub(super) fn paint_attachment(painter: &Painter, rect: Rect, project: &Project, camera: &Camera, target: Anchor) {
    if let Some(point) = target.position(project) {
        paint_pin(painter, camera.screen(point, rect), Tokens::get().accent);
    }
}

/// Return the closest wire stroke under the pointer, leaving symbol picking to the caller.
pub(super) fn at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<String> {
    project
        .wires()
        .iter()
        .filter_map(|wire| {
            let points: Vec<_> = project.wire_path(wire).into_iter().map(|point| camera.screen(point, rect)).collect();
            let distance = points.windows(2).map(|pair| distance_sq(pointer, pair[0], pair[1])).min_by(f32::total_cmp)?;
            (distance < theme::SCHEMATIC_WIRE_HIT_RADIUS.powi(2)).then(|| (distance, wire.id()))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// Mark an unconnected wire end with a small open square, as KiCad does.
fn paint_dangling(painter: &Painter, pos: Pos2, color: Color32) {
    let half = theme::SCHEMATIC_DANGLING_HALF_SIZE;
    painter.rect_stroke(Rect::from_center_size(pos, egui::vec2(2.0 * half, 2.0 * half)), 0, Stroke::new(1.0, color), egui::StrokeKind::Middle);
}

/// Mark an explicit connectable endpoint with the shared schematic pin styling.
fn paint_pin(painter: &Painter, pos: Pos2, color: Color32) {
    painter.circle_stroke(pos, theme::SCHEMATIC_PIN_RADIUS, Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color));
}

/// Paint a model-space polyline through the canvas clip using shared wire styling.
fn paint_route(painter: &Painter, rect: Rect, camera: &Camera, points: Vec<Point>, color: Color32) {
    paint_stroke(painter, rect, camera, points, Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color));
}

/// Paint a model-space polyline with `stroke`; fewer than two points paints nothing.
fn paint_stroke(painter: &Painter, rect: Rect, camera: &Camera, points: Vec<Point>, stroke: Stroke) {
    if points.len() >= 2 {
        painter.add(Shape::line(points.into_iter().map(|point| camera.screen(point, rect)).collect(), stroke));
    }
}
