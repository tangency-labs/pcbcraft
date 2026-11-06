use egui::{Color32, Painter, PointerButton, Pos2, Rect, Response, Shape, Stroke};
use pcbcraft_chrome::{Tokens, theme};
use pcbcraft_engine::{
    Project,
    schematic::{Point, SymbolPlacement, WireEndpoint, orthogonal_path},
    wire::Anchor,
};

use super::Camera;

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

/// Paint resolved paths, including a disposable symbol move, and explicit junction markers.
pub(super) fn paint(
    painter: &Painter,
    rect: Rect,
    project: &Project,
    camera: &Camera,
    moved: Option<(SymbolPlacement, Point)>,
    selected: Option<&str>,
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
        let color = if selected == Some(wire.id().as_str()) { tokens.accent } else { tokens.schematic_wire };
        paint_route(painter, rect, camera, pcbcraft_engine::wire::wire_path(wire, from, to), color);
    }
    for (_, point) in project.junctions() {
        let pos = camera.screen(point, rect);
        if rect.contains(pos) {
            paint_pin(painter, pos, tokens.schematic_wire);
        }
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
            let distance = points
                .windows(2)
                .map(|pair| {
                    let delta = pair[1] - pair[0];
                    let t = ((pointer - pair[0]).dot(delta) / delta.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
                    pointer.distance_sq(pair[0] + delta * t)
                })
                .min_by(f32::total_cmp)?;
            (distance < theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2)).then(|| (distance, wire.id()))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// Mark an explicit connectable endpoint with the shared schematic pin styling.
fn paint_pin(painter: &Painter, pos: Pos2, color: Color32) {
    painter.circle_stroke(pos, theme::SCHEMATIC_PIN_RADIUS, Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color));
}

/// Paint a model-space polyline through the canvas clip using shared wire styling.
fn paint_route(painter: &Painter, rect: Rect, camera: &Camera, points: Vec<Point>, color: Color32) {
    if points.len() >= 2 {
        painter.add(Shape::line(
            points.into_iter().map(|point| camera.screen(point, rect)).collect(),
            Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color),
        ));
    }
}
