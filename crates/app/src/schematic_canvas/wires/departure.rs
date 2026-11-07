//! Gesture-local guards for pulling a disconnected straight piece away from old contacts.
use super::*;

#[derive(Debug)]
struct End {
    id: u64,
    origin: Point,
    contacts: Vec<(u64, Vec<Point>)>,
    pins: Vec<WireEndpoint>,
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
            let pins = project
                .symbols()
                .iter()
                .flat_map(|symbol| symbol.terminals().into_iter().zip(symbol.pins()))
                .filter(|(_, point)| *point == origin)
                .map(|(pin, _)| WireEndpoint::Terminal(pin))
                .chain(project.hierarchy_endpoints().filter(|(_, point)| *point == origin).map(|(end, _)| end))
                .collect::<Vec<_>>();
            ends.push(End { id, origin, cleared: contacts.is_empty() && pins.is_empty(), contacts, pins });
        }
        Self { ends }
    }

    /// Release an end's departure guard only after it leaves every original contact's magnetic
    /// region. The guard stays released if the same gesture returns to its starting position.
    pub(super) fn update(&mut self, camera: &Camera, rect: Rect, grabbed: Point, to: Point) {
        for end in self.ends.iter_mut().filter(|end| !end.cleared) {
            let moved = Point { x_nm: end.origin.x_nm + to.x_nm - grabbed.x_nm, y_nm: end.origin.y_nm + to.y_nm - grabbed.y_nm };
            let pos = camera.screen(moved, rect);
            let radius = theme::SCHEMATIC_WIRE_HIT_RADIUS.max(theme::SCHEMATIC_PIN_HIT_RADIUS).powi(2);
            end.cleared = (end.pins.is_empty() || pos.distance_sq(camera.screen(end.origin, rect)) > radius)
                && end
                    .contacts
                    .iter()
                    .flat_map(|(_, path)| path.windows(2))
                    .all(|pair| distance_sq(pos, camera.screen(pair[0], rect), camera.screen(pair[1], rect)) > radius);
        }
    }

    /// Hide only original neighboring routes while their detached end is departing.
    pub(super) fn suppressed_wires(&self) -> std::collections::BTreeSet<u64> {
        self.ends.iter().filter(|end| !end.cleared).flat_map(|end| end.contacts.iter().map(|(id, _)| *id)).collect()
    }

    /// Mask vacated pin targets until the detached end has been pulled clear of them.
    pub(super) fn suppressed_nodes(&self) -> std::collections::BTreeSet<WireEndpoint> {
        self.ends.iter().filter(|end| !end.cleared).flat_map(|end| end.pins.iter().copied()).collect()
    }

    /// Request contact only for ends that have cleared their initial detached neighbors.
    pub(super) fn connection_ends(&self) -> Vec<u64> {
        self.ends.iter().filter(|end| end.cleared).map(|end| end.id).collect()
    }
}

#[cfg(test)]
mod hierarchy_tests {
    use super::*;
    use pcbcraft_engine::{Session, hierarchy, schematic::GRID_NM};
    use serde_json::json;

    /// Break at a sheet pin keeps its original contact masked until departure, then allows
    /// the same gesture to return and deliberately reconnect through the normal target path.
    #[test]
    fn broken_sheet_pin_uses_the_same_departure_guard_as_component_pins() {
        let mut session = Session::default();
        session.apply("project.new", &json!({})).unwrap();
        session.apply(hierarchy::CREATE_CHILD, &json!({"name":"Child","x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
        let parent = session.project().unwrap().sheet_target();
        let child = session.project().unwrap().sheets().nth(1).unwrap().target;
        session
            .apply(
                hierarchy::ADD_LABEL,
                &json!({"sheet":child.id(),"name":"DATA","direction":"input","x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
            )
            .unwrap();
        let label = session.project().unwrap().for_sheet(child).unwrap().hierarchical_labels()[0].id;
        let endpoint = WireEndpoint::SheetPin(label);
        let origin = session.project().unwrap().endpoint_position(endpoint).unwrap();
        session.apply(pcbcraft_engine::DRAW_WIRE,&json!({"sheet":parent.id(),"from":endpoint.inspect(),"to":{"x_nm":origin.x_nm-10*GRID_NM,"y_nm":origin.y_nm},"bends":[],"expected_revision":session.revision()})).unwrap();
        let wire = session.project().unwrap().wires()[0].id();
        session.apply(pcbcraft_engine::BREAK_WIRE,&json!({"sheet":parent.id(),"wire":wire,"at":{"x_nm":origin.x_nm-5*GRID_NM,"y_nm":origin.y_nm},"expected_revision":session.revision()})).unwrap();
        let project = session.project().unwrap();
        let wire = &project.wires()[0];
        let source = wire.endpoints().into_iter().find(|end| project.endpoint_position(*end) == Some(origin)).unwrap();
        let WireEndpoint::Junction(id) = source else {
            panic!("Break must detach the fixed pin");
        };
        let mut departure = Departure::new(project, &wire.id(), &Reshape::Junction(id));
        assert!(departure.suppressed_nodes().contains(&endpoint));
        let camera = Camera::default();
        let rect = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1000., 800.));
        departure.update(&camera, rect, origin, origin);
        assert!(departure.connection_ends().is_empty());
        departure.update(&camera, rect, origin, Point { x_nm: origin.x_nm, y_nm: origin.y_nm + 10 * GRID_NM });
        departure.update(&camera, rect, origin, origin);
        assert!(departure.suppressed_nodes().is_empty());
        assert_eq!(departure.connection_ends(), vec![id]);
    }
}
