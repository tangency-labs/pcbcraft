//! Gesture-local guards for pulling a disconnected straight piece away from old contacts.
use super::*;

#[derive(Debug)]
struct End {
    id: u64,
    origin: Point,
    contacts: Vec<(u64, Vec<Point>)>,
    cleared: bool,
}

/// Clones of one gesture share this transient state; other views and history never do.
#[derive(Debug, Default)]
pub(super) struct Departure {
    ends: Vec<End>,
}

impl Departure {
    /// Capture a free handle or the degree-one ends of a straight piece, with original
    /// coincident neighbors. Pins, shared junctions and bent-segment slides retain their semantics.
    pub(super) fn new(project: &Project, wire: &str, edit: &Reshape) -> Self {
        let Some(wire) = project.wires().iter().find(|w| w.id() == wire) else {
            return Self::default();
        };
        if !matches!(edit, Reshape::Junction(_)) && (!matches!(edit, Reshape::Segment { .. }) || project.wire_path(wire).len() != 2) {
            return Self::default();
        }
        let mut ends = Vec::new();
        for endpoint in wire.endpoints() {
            let WireEndpoint::Junction(id) = endpoint else {
                continue;
            };
            if matches!(edit,Reshape::Junction(source) if *source != id)
                || project.wires().iter().filter(|w| w.endpoints().contains(&endpoint)).count() != 1
            {
                continue;
            }
            let Some(origin) = project.endpoint_position(endpoint) else {
                continue;
            };
            let contacts = project
                .wires()
                .iter()
                .filter(|w| w.id() != wire.id())
                .map(|w| (w.ordinal(), project.wire_path(w)))
                .filter(|(_, path)| path.first() == Some(&origin) || path.last() == Some(&origin))
                .collect::<Vec<_>>();
            ends.push(End { id, origin, cleared: contacts.is_empty(), contacts });
        }
        Self { ends }
    }

    /// Release an end's departure guard only after it leaves every original contact's magnetic
    /// region. The guard stays released if the same gesture returns to its starting position.
    pub(super) fn update(&mut self, camera: &Camera, rect: Rect, grabbed: Point, to: Point) {
        for end in self.ends.iter_mut().filter(|end| !end.cleared) {
            let moved = Point { x_nm: end.origin.x_nm + to.x_nm - grabbed.x_nm, y_nm: end.origin.y_nm + to.y_nm - grabbed.y_nm };
            let pos = camera.screen(moved, rect);
            end.cleared = end.contacts.iter().flat_map(|(_, path)| path.windows(2)).all(|pair| {
                distance_sq(pos, camera.screen(pair[0], rect), camera.screen(pair[1], rect))
                    > theme::SCHEMATIC_WIRE_HIT_RADIUS.max(theme::SCHEMATIC_PIN_HIT_RADIUS).powi(2)
            });
        }
    }

    /// Hide only original neighboring routes while their detached end is departing.
    pub(super) fn suppressed_wires(&self) -> std::collections::BTreeSet<u64> {
        self.ends.iter().filter(|end| !end.cleared).flat_map(|end| end.contacts.iter().map(|(id, _)| *id)).collect()
    }

    /// Request contact only for ends that have cleared their initial detached neighbors.
    pub(super) fn connection_ends(&self) -> Vec<u64> {
        self.ends.iter().filter(|end| end.cleared).map(|end| end.id).collect()
    }
}
