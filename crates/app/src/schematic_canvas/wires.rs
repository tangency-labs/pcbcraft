use egui::{Color32, Painter, PointerButton, Pos2, Rect, Response, Shape, Stroke};
use pcbcraft_chrome::{Tokens, theme};
use pcbcraft_engine::{
    Project,
    schematic::{Point, SymbolPlacement, WireEndpoint, orthogonal_path},
    wire::Anchor,
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
    /// Resolve the frozen corners and current orthogonal preview leg.
    pub(crate) fn path(&self, from: Point, to: Point) -> Vec<Point> {
        let previous = self.corners.last().cloned().unwrap_or_else(|| vec![from]);
        orthogonal_path(previous.into_iter().chain(std::iter::once(to)), self.horizontal_first)
    }

    /// Freeze a clicked corner if the resulting route fits the shared command's bound.
    pub(crate) fn corner(&mut self, from: Point, to: Point) -> bool {
        let path = self.path(from, to);
        if path.len() < 2 || path.len() > pcbcraft_engine::wire::MAX_BENDS {
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
    /// A point strictly inside the leg, naming it for Break and Delete Segment. Legs join grid points, so
    /// their midpoint is a whole number of nanometers.
    pub(crate) fn inside(&self) -> Point {
        let (a, b) = (self.route[self.leg], self.route[self.leg + 1]);
        Point { x_nm: a.x_nm + (b.x_nm - a.x_nm) / 2, y_nm: a.y_nm + (b.y_nm - a.y_nm) / 2 }
    }
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
) -> Option<(Anchor, bool)> {
    let clicked = response.clicked_by(PointerButton::Primary);
    let pointer = if clicked { response.interact_pointer_pos() } else { response.hover_pos() };
    let target = preview(painter, rect, project, camera, start, draft, pointer);
    response.clone().on_hover_cursor(egui::CursorIcon::Crosshair);
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
) -> Option<Anchor> {
    let tokens = Tokens::get();
    let endpoints: Vec<_> = project
        .symbols()
        .iter()
        .flat_map(|symbol| symbol.terminals().into_iter().map(WireEndpoint::Terminal).zip(symbol.pins()))
        .chain(project.junctions().map(|(id, point)| (WireEndpoint::Junction(id), point)))
        .map(|(endpoint, point)| (endpoint, camera.screen(point, rect)))
        .filter(|(_, pos)| rect.contains(*pos))
        .collect();
    let target = pointer.filter(|pos| rect.contains(*pos)).and_then(|at| {
        let mut nearest = None;
        let mut distance = theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2);
        let mut ambiguous = false;
        for &(endpoint, pos) in &endpoints {
            let candidate = at.distance_sq(pos);
            if candidate < distance {
                nearest = Some(endpoint);
                distance = candidate;
                ambiguous = false;
            } else if candidate == distance && nearest.is_some() {
                ambiguous = true;
            }
        }
        if ambiguous {
            None
        } else if let Some(endpoint) = nearest {
            Some(Anchor::Existing(endpoint))
        } else {
            let point = camera.snap(at, rect);
            super::valid_position(point).then_some(Anchor::Free(point))
        }
    });
    if let Some(from) = start.and_then(|anchor| anchor.position(project))
        && let Some(to) = target.and_then(|anchor| anchor.position(project))
    {
        paint_route(painter, rect, camera, draft.path(from, to), tokens.accent);
    }
    for (endpoint, pos) in endpoints {
        let anchor = Anchor::Existing(endpoint);
        let active = start == Some(anchor) || target == Some(anchor);
        paint_pin(painter, pos, if active { tokens.accent } else { tokens.canvas_ink });
    }
    target
}

/// Return the closest wire stroke under the pointer, leaving symbol picking to the caller.
pub(super) fn at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<String> {
    project
        .wires()
        .iter()
        .filter_map(|wire| {
            let points: Vec<_> = project.wire_path(wire).into_iter().map(|point| camera.screen(point, rect)).collect();
            let distance = points.windows(2).map(|pair| distance_sq(pointer, pair[0], pair[1])).min_by(f32::total_cmp)?;
            (distance < theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2)).then(|| (distance, wire.id()))
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
