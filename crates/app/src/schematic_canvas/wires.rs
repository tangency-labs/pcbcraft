use egui::{Color32, Painter, PointerButton, Pos2, Rect, Response, Shape, Stroke};
use pcbcraft_chrome::{Tokens, theme};
use pcbcraft_engine::{
    Project,
    schematic::{Point, SymbolPlacement, WireEndpoint, orthogonal_path},
    wire::{Anchor, Reshape},
};

use super::Camera;

#[cfg(test)]
mod tests;

/// Uncommitted route corners and elbow preference; this never changes a project snapshot.
#[derive(Debug)]
pub(crate) struct Draft {
    corners: Vec<Vec<Point>>,
    pub(crate) horizontal_first: bool,
    pub(crate) finish_requested: bool,
}

impl Default for Draft {
    fn default() -> Self {
        Self { corners: Vec::new(), horizontal_first: true, finish_requested: false }
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
    /// For a segment, whether its leg runs horizontally, so it moves only vertically.
    horizontal: Option<bool>,
    origin: Point,
}

impl ReshapeDrag {
    /// Resolve a drag destination using drawing's unambiguous pin/junction hit test for a
    /// movable endpoint. Other handles and empty space keep their snapped pointer travel.
    /// The source itself is never an attachment target; coincident candidates cancel the drop.
    pub(crate) fn destination(&self, project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<Anchor> {
        if !rect.contains(pointer) {
            return None;
        }
        if let Reshape::Junction(id) = self.edit {
            let anchor = anchor_at(&endpoints(project, camera, rect), project, camera, rect, pointer, Some(WireEndpoint::Junction(id)))?;
            if !matches!(anchor, Anchor::Free(_)) && anchor != Anchor::Existing(WireEndpoint::Junction(id)) {
                return Some(anchor);
            }
        }
        let point = self.target(camera.snap(pointer, rect));
        super::valid_position(point).then_some(Anchor::Free(point))
    }

    /// Validate a disposable preview with the same engine operation used on release.
    /// Only a junction handle can attach; invalid geometry or topology yields no preview.
    pub(crate) fn preview(&self, project: &Project, to: Anchor, revision: u64, auto_heal: bool) -> Option<Project> {
        match (&self.edit, to) {
            (Reshape::Junction(id), Anchor::Existing(_) | Anchor::Split { .. }) => project.attached(*id, to, revision, auto_heal).ok(),
            (_, Anchor::Free(point)) => project.reshaped(&self.edit, point).and_then(|next| next.healed(project, revision, auto_heal)).ok(),
            _ => None,
        }
    }

    /// Where the grabbed point goes with the pointer snapped at `pointer`: moved by the snapped
    /// travel, and only across a segment's direction.
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

/// What a press at `pointer` grabs on `wire`: one of its free ends or
/// junctions, then an interior corner, then a leg, each within the pin hit radius (the wire
/// stroke radius for legs). Pins can't be dragged off their parts, so terminal ends grab
/// nothing. `origin` is the snapped press point. `None` when nothing of that wire is there.
pub(super) fn reshape_at(project: &Project, camera: &Camera, rect: Rect, wire: &str, pointer: Pos2, origin: Point) -> Option<ReshapeDrag> {
    let found = project.wires().iter().find(|candidate| candidate.id() == wire)?;
    let route = project.wire_path(found);
    let near = |point: Point| camera.screen(point, rect).distance(pointer) <= theme::SCHEMATIC_HANDLE_HIT_RADIUS;
    let drag = |edit: Reshape, grabbed: Point, horizontal: Option<bool>| ReshapeDrag { wire: wire.to_owned(), edit, grabbed, horizontal, origin };
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
    Some(drag(Reshape::Segment { wire: wire.to_owned(), at }, at, Some(a.y_nm == b.y_nm)))
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
    let preview = moved.map(|(symbol, point)| (symbol.terminals(), symbol.definition.pin_positions(point, symbol.turns())));
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
            0 | 1 => paint_dangling(painter, pos, tokens.schematic_wire),
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
        Some(Self { wire: wire.to_owned(), route, leg, split: Some(at), revision })
    }

    /// Whether this captured leg slides vertically rather than horizontally.
    pub(crate) fn horizontal(&self) -> bool {
        self.route[self.leg].y_nm == self.route[self.leg + 1].y_nm
    }
    /// A point strictly inside the leg, naming it for Break and Delete Segment. Legs join grid points, so
    /// their midpoint is a whole number of nanometers.
    pub(crate) fn inside(&self) -> Point {
        let (a, b) = (self.route[self.leg], self.route[self.leg + 1]);
        Point { x_nm: a.x_nm + (b.x_nm - a.x_nm) / 2, y_nm: a.y_nm + (b.y_nm - a.y_nm) / 2 }
    }
}

/// Paint only the selected leg, optionally displaced by a validated slide preview. The rest
/// of the route keeps its ordinary ink so Delete's scope is visible before it is pressed.
pub(super) fn paint_selected_leg(painter: &Painter, rect: Rect, camera: &Camera, leg: &LegTarget, offset_nm: i64) {
    let points = leg.route[leg.leg..=leg.leg + 1]
        .iter()
        .map(
            |point| {
                if leg.horizontal() { Point { y_nm: point.y_nm + offset_nm, ..*point } } else { Point { x_nm: point.x_nm + offset_nm, ..*point } }
            },
        )
        .collect();
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
    Some(LegTarget { wire: id, route, leg, split, revision })
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
            let last = target.route.len() - 1;
            for (index, point) in [(target.leg, leg[0]), (target.leg + 1, leg[1])] {
                if index != 0 && index != last {
                    paint_dangling(painter, camera.screen(point, rect), tokens.accent);
                }
            }
        }
        None => paint_route(painter, rect, camera, leg, tokens.accent),
    }
}

/// Pick a pin, an explicit free endpoint, or a snapped empty point. Enter/double-click finishes.
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
) -> Option<(Anchor, bool)> {
    let clicked = response.clicked_by(PointerButton::Primary);
    let pointer = if clicked { response.interact_pointer_pos() } else { response.hover_pos() };
    let target = preview(painter, rect, project, camera, start, draft, pointer, auto_heal);
    if response.hovered()
        && let Some(pointer) = pointer
    {
        let point = target.and_then(|anchor| anchor.position(project)).unwrap_or_else(|| camera.snap(pointer, rect));
        painter.ctx().set_cursor_icon(egui::CursorIcon::None);
        pcbcraft_chrome::cursor::wire(painter, camera.screen(point, rect), target.is_some_and(|anchor| !matches!(anchor, Anchor::Free(_))));
    }
    let finish = draft.finish_requested || response.double_clicked_by(PointerButton::Primary);
    target.filter(|_| clicked || finish).map(|target| (target, finish))
}

/// Highlight unambiguous existing endpoints and preview the full route to the current target.
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
) -> Option<Anchor> {
    let tokens = Tokens::get();
    let endpoints = endpoints(project, camera, rect);
    let target = pointer.filter(|pos| rect.contains(*pos)).and_then(|at| anchor_at(&endpoints, project, camera, rect, at, None));
    if let Some(from) = start.and_then(|anchor| anchor.position(project))
        && let Some(to) = target.and_then(|anchor| anchor.position(project))
        && let Some(path) = draft.path(from, to, auto_heal)
    {
        paint_route(painter, rect, camera, path, tokens.accent);
    }
    for (endpoint, pos) in endpoints {
        let anchor = Anchor::Existing(endpoint);
        let active = start == Some(anchor) || target == Some(anchor);
        paint_pin(painter, pos, if active { tokens.accent } else { tokens.canvas_ink });
    }
    target
}

/// Project visible, explicitly connectable identities to screen space for drawing and dragging.
fn endpoints(project: &Project, camera: &Camera, rect: Rect) -> Vec<(WireEndpoint, Pos2)> {
    project
        .symbols()
        .iter()
        .flat_map(|symbol| symbol.terminals().into_iter().map(WireEndpoint::Terminal).zip(symbol.pins()))
        .chain(project.junctions().map(|(id, point)| (WireEndpoint::Junction(id), point)))
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

/// Resolve the nearest node, wire stroke or free grid point without resolving ties
/// arbitrarily. Interior wire targets use their projected model coordinate.
fn anchor_at(
    endpoints: &[(WireEndpoint, Pos2)],
    project: &Project,
    camera: &Camera,
    rect: Rect,
    at: Pos2,
    ignore: Option<WireEndpoint>,
) -> Option<Anchor> {
    if let Some(endpoint) = nearest_node(endpoints, at).ok()? {
        Some(Anchor::Existing(endpoint))
    } else {
        let mut hit = None;
        let mut nearest = theme::SCHEMATIC_WIRE_HIT_RADIUS.powi(2);
        let mut ambiguous = false;
        for wire in project.wires().iter().filter(|w| ignore.is_none_or(|end| !w.endpoints().contains(&end))) {
            for pair in project.wire_path(wire).windows(2) {
                let distance = distance_sq(at, camera.screen(pair[0], rect), camera.screen(pair[1], rect));
                let snapped = camera.snap(at, rect);
                let point = if pair[0].x_nm == pair[1].x_nm {
                    Point { x_nm: pair[0].x_nm, y_nm: snapped.y_nm.clamp(pair[0].y_nm.min(pair[1].y_nm), pair[0].y_nm.max(pair[1].y_nm)) }
                } else {
                    Point { x_nm: snapped.x_nm.clamp(pair[0].x_nm.min(pair[1].x_nm), pair[0].x_nm.max(pair[1].x_nm)), y_nm: pair[0].y_nm }
                };
                let anchor = Anchor::Split { wire: wire.ordinal(), at: point };
                if distance < nearest {
                    hit = Some(anchor);
                    nearest = distance;
                    ambiguous = false;
                } else if distance == nearest && hit.is_some_and(|old| old != anchor) {
                    ambiguous = true;
                }
            }
        }
        if ambiguous {
            None
        } else {
            hit.or_else(|| {
                let point = camera.snap(at, rect);
                super::valid_position(point).then_some(Anchor::Free(point))
            })
        }
    }
}

/// Resolve a directly clickable terminal or explicit wire node using drawing's magnetic
/// target rules. Interior wire strokes remain available for segment selection and dragging.
/// Equal-distance nodes are ambiguous and cannot start a connection.
pub(super) fn node_at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<Anchor> {
    if !rect.contains(pointer) {
        return None;
    }
    nearest_node(&endpoints(project, camera, rect), pointer).ok()?.map(Anchor::Existing)
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
