//! Per-view connection geometry, rebuilt only when its project revision or transform changes.

use super::{Anchor, Camera, Point, Project, WireEndpoint, distance_sq, endpoints, nearest_node};
use egui::{Pos2, Rect};
use pcbcraft_chrome::theme;
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    revision: u64,
    rect: Rect,
    origin: Pos2,
    step: f32,
}

#[derive(Debug)]
struct Segment {
    wire: u64,
    ends: [WireEndpoint; 2],
    from: Point,
    to: Point,
    a: Pos2,
    b: Pos2,
    hit: Rect,
}

/// Disposable model-derived data. It has no edit authority and never enters project storage.
#[derive(Debug, Default)]
pub(super) struct Targets {
    pub(super) endpoints: Vec<(WireEndpoint, Pos2)>,
    segments: Vec<Segment>,
    degree: BTreeMap<WireEndpoint, usize>,
}

/// Each view retains its own projected targets, including across ordinary pointer motion.
#[derive(Debug, Default)]
pub(super) struct Cache {
    key: Option<Key>,
    targets: Targets,
}

impl Cache {
    /// Rebuild after an edit, undo/reopen, zoom, pan or resize. Grid changes use fresh snapping
    /// at query time and do not change the cached electrical identities or projected geometry.
    pub(super) fn get(&mut self, project: &Project, revision: u64, camera: &Camera, rect: Rect) -> &Targets {
        let key = Key { revision, rect, origin: camera.screen(Point { x_nm: 0, y_nm: 0 }, rect), step: camera.unit_step() };
        if self.key != Some(key) {
            self.targets = Targets::new(project, camera, rect);
            self.key = Some(key);
        }
        &self.targets
    }
}

impl Targets {
    /// Resolve routes once, retaining screen-space bounds for fast rejection away from a
    /// wire. Connected pins/junctions and named markers contacting routes remain explicit targets,
    /// although automatic starts exclude occupied contacts.
    pub(super) fn new(project: &Project, camera: &Camera, rect: Rect) -> Self {
        let mut result = Self { endpoints: endpoints(project, camera, rect), ..Self::default() };
        for wire in project.wires() {
            for end in wire.endpoints() {
                *result.degree.entry(end).or_default() += 1;
            }
            for pair in project.wire_path(wire).windows(2) {
                let (a, b) = (camera.screen(pair[0], rect), camera.screen(pair[1], rect));
                let hit = Rect::from_two_pos(a, b).expand(theme::SCHEMATIC_WIRE_HIT_RADIUS);
                if rect.intersects(hit) {
                    result.segments.push(Segment { wire: wire.ordinal(), ends: wire.endpoints(), from: pair[0], to: pair[1], a, b, hit });
                }
            }
        }
        for label in project.net_labels() {
            if result.segments.iter().any(|segment| pcbcraft_engine::wire::path_contains_point(&[segment.from, segment.to], label.at)) {
                // A marker touching a route is occupied even when that route's endpoint is
                // a component pin or junction rather than the marker's own identity.
                result.degree.entry(WireEndpoint::NetLabel(label.id)).or_insert(1);
            }
        }
        result
    }

    /// An ordinary click may start only at an unconnected pin or a degree-one free end.
    /// The nearest occupied node keeps selection priority instead of redirecting to another pin.
    pub(super) fn automatic_node(&self, pointer: Pos2) -> Option<Anchor> {
        let end = nearest_node(&self.endpoints, pointer).ok()??;
        let degree = self.degree.get(&end).copied().unwrap_or(0);
        let allowed = match end {
            WireEndpoint::NetLabel(_) | WireEndpoint::Terminal(_) | WireEndpoint::Label(_) | WireEndpoint::SheetPin(_) => degree == 0,
            WireEndpoint::Junction(_) => degree == 1,
        };
        allowed.then_some(Anchor::Existing(end))
    }

    /// Resolve an unambiguous node, stroke or free point. Explicit connections retain their
    /// exact positions when grid snapping is bypassed; stroke splits project the current
    /// pointer coordinate onto the segment. Neither policy rebuilds the cached geometry.
    pub(super) fn anchor(&self, camera: &Camera, rect: Rect, at: Pos2, ignore: Option<WireEndpoint>, snap_to_grid: bool) -> Option<Anchor> {
        self.anchor_excluding(camera, rect, at, ignore, snap_to_grid, &std::collections::BTreeSet::new(), &std::collections::BTreeSet::new())
    }

    /// Endpoint departures can hide their source identity and original neighboring routes
    /// temporarily. Drawing passes no exclusions and retains ordinary magnetic targeting.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn anchor_excluding(
        &self,
        camera: &Camera,
        rect: Rect,
        at: Pos2,
        ignore: Option<WireEndpoint>,
        snap_to_grid: bool,
        suppressed: &std::collections::BTreeSet<u64>,
        suppressed_pins: &std::collections::BTreeSet<WireEndpoint>,
    ) -> Option<Anchor> {
        let nodes;
        let endpoints = if ignore.is_none() && suppressed.is_empty() && suppressed_pins.is_empty() {
            &self.endpoints
        } else {
            let suppressed_nodes: std::collections::BTreeSet<_> =
                self.segments.iter().filter(|segment| suppressed.contains(&segment.wire)).flat_map(|segment| segment.ends).collect();
            nodes = self
                .endpoints
                .iter()
                .copied()
                .filter(|(end, _)| Some(*end) != ignore && !suppressed_nodes.contains(end) && !suppressed_pins.contains(end))
                .collect();
            &nodes
        };
        if let Some(end) = nearest_node(endpoints, at).ok()? {
            return Some(Anchor::Existing(end));
        }
        let snapped = if snap_to_grid { camera.snap(at, rect) } else { camera.unsnapped(at, rect) };
        let mut nearest = theme::SCHEMATIC_WIRE_HIT_RADIUS.powi(2);
        let (mut hit, mut ambiguous) = (None, false);
        for segment in &self.segments {
            if !segment.hit.contains(at) || suppressed.contains(&segment.wire) || ignore.is_some_and(|end| segment.ends.contains(&end)) {
                continue;
            }
            let distance = distance_sq(at, segment.a, segment.b);
            let point = if segment.from.x_nm == segment.to.x_nm {
                Point {
                    x_nm: segment.from.x_nm,
                    y_nm: snapped.y_nm.clamp(segment.from.y_nm.min(segment.to.y_nm), segment.from.y_nm.max(segment.to.y_nm)),
                }
            } else {
                Point {
                    x_nm: snapped.x_nm.clamp(segment.from.x_nm.min(segment.to.x_nm), segment.from.x_nm.max(segment.to.x_nm)),
                    y_nm: segment.from.y_nm,
                }
            };
            let anchor = Anchor::Split { wire: segment.wire, at: point };
            if distance < nearest {
                hit = Some(anchor);
                nearest = distance;
                ambiguous = false;
            } else if distance == nearest && hit.is_some_and(|old| old != anchor) {
                ambiguous = true;
            }
        }
        if ambiguous { None } else { hit.or_else(|| super::super::valid_position(snapped).then_some(Anchor::Free(snapped))) }
    }
}
