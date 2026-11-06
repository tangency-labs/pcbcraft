//! Authored wire geometry and atomic edits shared by the editor and MCP.

use std::{collections::BTreeSet, sync::Arc};

use serde_json::{Value, json};

use crate::{
    CommandError, MAX_OBJECT_ID, Project, Session,
    schematic::{
        MAX_CENTER_NM, Point, Wire, WireEndpoint,
        wiring::{MAX_WIRES, parse_id},
    },
};

/// Maximum intermediate points accepted in a single authored wire.
pub const MAX_BENDS: usize = 64;

/// A wire gesture targets an existing electrical identity or creates a free endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Existing(WireEndpoint),
    Free(Point),
}

impl Anchor {
    /// Parse a strict terminal/junction reference or a bounded free point.
    pub fn parse(value: &Value) -> Option<Self> {
        if let Some(endpoint) = WireEndpoint::parse(value) {
            return Some(Self::Existing(endpoint));
        }
        serde_json::from_value::<Point>(value.clone()).ok().filter(valid_point).map(Self::Free)
    }

    /// Describe one target in the shared draw-wire command representation.
    pub fn inspect(self) -> Value {
        match self {
            Self::Existing(endpoint) => endpoint.inspect(),
            Self::Free(point) => json!(point),
        }
    }

    /// Resolve a gesture target in its current project; missing identities are rejected.
    pub fn position(self, project: &Project) -> Option<Point> {
        match self {
            Self::Existing(endpoint) => project.endpoint_position(endpoint),
            Self::Free(point) => Some(point),
        }
    }
}

/// Test point bounds without arithmetic that can overflow on integer extremes.
pub(crate) fn valid_point(point: &Point) -> bool {
    (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.x_nm) && (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.y_nm)
}

/// Parse bounded authored bends without accepting fractions, extra fields or unknown coordinates.
pub(crate) fn parse_bends(value: &Value) -> Option<Vec<Point>> {
    let points = value.as_array()?;
    if points.len() > MAX_BENDS {
        return None;
    }
    points.iter().map(|value| serde_json::from_value::<Point>(value.clone()).ok().filter(valid_point)).collect()
}

impl Project {
    /// Resolve either kind of explicit electrical endpoint in this snapshot.
    pub fn endpoint_position(&self, endpoint: WireEndpoint) -> Option<Point> {
        match endpoint {
            WireEndpoint::Terminal(pin) => self.terminal_position(pin),
            WireEndpoint::Junction(id) => self.junctions.get(&id).copied(),
        }
    }

    /// Inspect free endpoint and junction positions in stable identity order.
    pub fn junctions(&self) -> impl Iterator<Item = (u64, Point)> + '_ {
        self.junctions.iter().map(|(&id, &point)| (id, point))
    }

    /// Resolve the drawn polyline, adapting the end legs when a connected symbol moves or rotates.
    pub fn wire_path(&self, wire: &Wire) -> Vec<Point> {
        let [Some(from), Some(to)] = wire.endpoints.map(|end| self.endpoint_position(end)) else {
            return Vec::new();
        };
        wire_path(wire, from, to)
    }

    /// Remove unused free points after deleting an edge or a connected symbol.
    pub(crate) fn prune_junctions(&mut self) {
        let used: BTreeSet<_> = self
            .wires
            .iter()
            .flat_map(|wire| wire.endpoints)
            .filter_map(|endpoint| match endpoint {
                WireEndpoint::Junction(id) => Some(id),
                WireEndpoint::Terminal(_) => None,
            })
            .collect();
        Arc::make_mut(&mut self.junctions).retain(|id, _| used.contains(id));
    }
}

/// Produce a wire's display path at supplied endpoints, including disposable symbol-move previews.
pub fn wire_path(wire: &Wire, from: Point, to: Point) -> Vec<Point> {
    if let Some(bends) = wire.bends() {
        crate::schematic::orthogonal_path(std::iter::once(from).chain(bends.iter().copied()).chain(std::iter::once(to)), true)
    } else if from.x_nm == to.x_nm || from.y_nm == to.y_nm {
        vec![from, to]
    } else {
        let middle = (from.x_nm + to.x_nm) / 2;
        vec![from, Point { x_nm: middle, y_nm: from.y_nm }, Point { x_nm: middle, y_nm: to.y_nm }, to]
    }
}

impl Session {
    /// Commit one complete route, including its free points, as one revision and undo step.
    /// All geometry and endpoint checks precede adoption, so a rejected draft changes nothing.
    pub(crate) fn draw_wire(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        if project.wires.len() >= MAX_WIRES {
            return Err(CommandError::Unavailable("the schematic wire limit has been reached".to_owned()));
        }
        let parse = |key: &str| Anchor::parse(&params[key]).ok_or_else(|| CommandError::InvalidParams(format!("invalid {key} wire endpoint")));
        let anchors = [parse("from")?, parse("to")?];
        let positions = anchors.map(|anchor| anchor.position(project));
        let [Some(from), Some(to)] = positions else {
            return Err(CommandError::InvalidParams("wire endpoint does not exist".to_owned()));
        };
        if from == to {
            return Err(CommandError::InvalidParams("a wire needs distinct endpoint positions".to_owned()));
        }
        let bends = parse_bends(&params["bends"]).ok_or_else(|| CommandError::InvalidParams("invalid wire bends".to_owned()))?;
        let path: Vec<_> = std::iter::once(from).chain(bends.iter().copied()).chain(std::iter::once(to)).collect();
        if path.windows(2).any(|pair| pair[0] == pair[1] || (pair[0].x_nm != pair[1].x_nm && pair[0].y_nm != pair[1].y_nm)) {
            return Err(CommandError::InvalidParams("wire legs must be nonzero and horizontal or vertical".to_owned()));
        }
        let mut next = self.next_object_id()?;
        if next > MAX_OBJECT_ID - 3 {
            return Err(CommandError::Unavailable("project has exhausted its object IDs".to_owned()));
        }
        let mut candidate = project.clone();
        let endpoints = anchors.map(|anchor| match anchor {
            Anchor::Existing(endpoint) => endpoint,
            Anchor::Free(point) => {
                let id = next;
                next += 1;
                Arc::make_mut(&mut candidate.junctions).insert(id, point);
                WireEndpoint::Junction(id)
            }
        });
        if candidate.wires.iter().any(|wire| wire.endpoints == endpoints || wire.endpoints == [endpoints[1], endpoints[0]]) {
            return Err(CommandError::InvalidParams("these endpoints are already joined by a wire".to_owned()));
        }
        Arc::make_mut(&mut candidate.wires).push(Wire { ordinal: next, endpoints, bends: Some(bends) });
        candidate.connectivity = Arc::default();
        self.change(candidate);
        Ok(())
    }

    /// Delete only the selected electrical edge, preserving other branches and removing orphan points.
    pub(crate) fn delete_wire(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let id = params["wire"].as_str().and_then(|value| parse_id(value, "wire-"));
        let project = self.open_project()?;
        let index = project
            .wires
            .iter()
            .position(|wire| Some(wire.ordinal) == id)
            .ok_or_else(|| CommandError::InvalidParams("wire does not exist".to_owned()))?;
        let mut candidate = project.clone();
        Arc::make_mut(&mut candidate.wires).remove(index);
        candidate.prune_junctions();
        candidate.connectivity = Arc::default();
        self.change(candidate);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DELETE_WIRE, DRAW_WIRE, PLACE_SYMBOL, ROTATE_SYMBOL,
        persistence::{self, Encoding},
        schematic::GRID_NM,
    };

    /// Start an empty session for free-route and topology tests.
    fn empty() -> Session {
        let mut session = Session::default();
        session.apply("project.new", &json!({"name":"Routes"})).unwrap();
        session
    }

    /// Make a bounded point on the schematic grid.
    fn point(x: i64, y: i64) -> Value {
        json!({"x_nm":x * GRID_NM,"y_nm":y * GRID_NM})
    }

    /// Draw a complete route at the current revision and return its inspection.
    fn draw(session: &mut Session, from: Value, to: Value, bends: Value) -> Value {
        session.execute(DRAW_WIRE, &json!({"from":from,"to":to,"bends":bends,"expected_revision":session.revision()})).unwrap()
    }

    /// Free routes allocate unique identities atomically and round-trip in both native encodings.
    #[test]
    fn free_routes_round_trip_and_undo_as_one_edit() {
        let mut session = empty();
        let before = session.snapshot().unwrap();
        let state = draw(&mut session, point(0, 0), point(8, 6), json!([point(8, 0)]));
        let project = session.snapshot().unwrap();
        assert_eq!(project.junctions().count(), 2);
        assert_eq!(session.revision(), 4);
        assert_eq!(project.connectivity().nets, vec![Vec::<crate::schematic::Terminal>::new()]);
        assert_eq!(project.wire_path(&project.wires()[0]).len(), 3);
        for encoding in [Encoding::Json, Encoding::Zip] {
            let bytes = persistence::encode_file(project.clone(), encoding).unwrap();
            let (loaded, detected) = persistence::decode_file(&bytes).unwrap();
            assert_eq!(detected, encoding);
            assert_eq!(persistence::encode_file(loaded.clone(), encoding).unwrap(), bytes);
            assert_eq!(loaded.inspect_schematic(), state["project"]["schematic"]);
            let mut reopened = Session::default();
            reopened.open_snapshot(loaded);
            assert!(reopened.revision() > 4);
        }
        session.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        session.apply("edit.redo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&project, &session.snapshot().unwrap()));
        session.apply("edit.undo", &Value::Null).unwrap();
        let replacement = draw(&mut session, point(0, 0), point(4, 0), json!([]));
        assert_ne!(replacement["project"]["schematic"]["wires"][0]["id"], "wire-4");
    }

    /// Only explicit endpoint reuse joins nets; crossings and coincident new points stay separate.
    #[test]
    fn endpoints_connect_branches_but_crossings_do_not() {
        let mut session = empty();
        let first = draw(&mut session, point(-8, 0), point(8, 0), json!([]));
        let endpoint = first["project"]["schematic"]["wires"][0]["to"].clone();
        draw(&mut session, point(0, -8), point(0, 8), json!([]));
        draw(&mut session, point(8, 0), point(12, 0), json!([]));
        assert_eq!(session.project().unwrap().connectivity().nets.len(), 3);
        draw(&mut session, endpoint.clone(), point(8, 8), json!([]));
        assert_eq!(session.project().unwrap().connectivity().nets.len(), 3);
        let snapshot = session.snapshot().unwrap();
        let id = snapshot.wires().last().unwrap().id();
        session.apply(DELETE_WIRE, &json!({"wire":id,"expected_revision":session.revision()})).unwrap();
        assert_eq!(session.project().unwrap().junctions().count(), 6);
        assert!(Anchor::parse(&endpoint).unwrap().position(session.project().unwrap()).is_some());
        session.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    }

    /// Pinned wires follow rotation without losing identity or changing stored bend records.
    #[test]
    fn authored_pin_route_follows_rotation_and_legacy_duplicates_are_rejected() {
        let mut session = empty();
        session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
        let pin = json!({"symbol":"symbol-2","terminal":"2"});
        draw(&mut session, pin.clone(), point(8, 4), json!([point(8, 0)]));
        let before = session.snapshot().unwrap();
        session.apply(ROTATE_SYMBOL, &json!({"symbol":"symbol-2","expected_revision":session.revision()})).unwrap();
        let after = session.snapshot().unwrap();
        assert!(before.same_connectivity(&after));
        assert_eq!(before.wires(), after.wires());
        let path = after.wire_path(&after.wires()[0]);
        assert_ne!(path[0], before.wire_path(&before.wires()[0])[0]);
        assert!(path.windows(2).all(|pair| pair[0].x_nm == pair[1].x_nm || pair[0].y_nm == pair[1].y_nm));
        let bytes = persistence::encode(after).unwrap();
        assert_eq!(persistence::encode(persistence::decode(&bytes).unwrap()).unwrap(), bytes);
        session.apply(crate::DELETE_SYMBOL, &json!({"symbol":"symbol-2","expected_revision":session.revision()})).unwrap();
        assert!(session.project().unwrap().wires().is_empty());
        assert_eq!(session.project().unwrap().junctions().count(), 0);

        let mut session = empty();
        session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
        let other = json!({"symbol":"symbol-2","terminal":"1"});
        draw(&mut session, pin.clone(), other.clone(), json!([]));
        let before = session.inspect();
        assert!(session.apply(crate::CONNECT_PINS, &json!({"from":other,"to":pin,"expected_revision":session.revision()})).is_err());
        assert_eq!(session.inspect(), before);
    }

    /// Malformed geometry, stale revisions and invalid identities fail before any allocation is adopted.
    #[test]
    fn invalid_routes_are_atomic_and_old_formats_reject_new_geometry() {
        let mut session = empty();
        let before = session.inspect();
        let valid = json!({"from":point(0,0),"to":point(8,0),"bends":[],"expected_revision":1});
        for (field, value) in [
            ("from", json!({"junction":"junction-999"})),
            ("from", json!({"symbol":"symbol-999","terminal":"1"})),
            ("from", json!({"x_nm":i64::MIN,"y_nm":0})),
            ("from", json!({"x_nm":0,"y_nm":0,"extra":true})),
            ("to", point(0, 0)),
            ("to", point(8, 8)),
            ("bends", json!([point(0, 0)])),
            ("bends", json!([point(2, 3)])),
            ("bends", json!(vec![point(1, 0); MAX_BENDS + 1])),
            ("expected_revision", json!(0)),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(session.apply(DRAW_WIRE, &bad).is_err(), "{bad}");
            assert_eq!(session.inspect(), before);
        }
        session.apply(DRAW_WIRE, &valid).unwrap();
        let snapshot = session.inspect();
        assert!(session.apply(DRAW_WIRE, &valid).is_err());
        assert_eq!(session.inspect(), snapshot);
        let bytes = persistence::encode(session.snapshot().unwrap()).unwrap();
        let stored: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored["format"], "pcbcraft-project/3");
        for (path, value) in [
            ("/format", json!("pcbcraft-project/2")),
            ("/sheet/wires/0/from/junction", json!(99)),
            ("/sheet/junctions/0/id", json!(4)),
            ("/sheet/junctions/0/x_nm", json!(i64::MAX)),
            ("/sheet/wires/0/bends", json!(vec![point(0, 0); MAX_BENDS + 1])),
        ] {
            let mut bad = stored.clone();
            *bad.pointer_mut(path).unwrap() = value;
            assert!(persistence::decode(&serde_json::to_vec(&bad).unwrap()).is_err(), "{bad}");
        }
    }
}
