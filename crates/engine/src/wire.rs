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

mod heal;
mod reshape;
pub use reshape::Reshape;

/// Automatic normalization defaults on for existing command clients. An explicit false
/// preserves authored routes while still resolving deliberate connection targets.
pub(crate) fn auto_heal(params: &Value) -> bool {
    params["auto_heal"].as_bool().unwrap_or(true)
}

/// Maximum intermediate points accepted in a single authored wire.
pub const MAX_BENDS: usize = 64;

/// A wire gesture targets an existing electrical identity or creates a free endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Existing(WireEndpoint),
    Free(Point),
    /// Explicitly branch into this wire at an unambiguous point on its current route.
    Split {
        wire: u64,
        at: Point,
    },
}

impl Anchor {
    /// Parse a strict terminal/junction reference, explicit wire/at target or bounded free point.
    pub fn parse(value: &Value) -> Option<Self> {
        if let Some(endpoint) = WireEndpoint::parse(value) {
            return Some(Self::Existing(endpoint));
        }
        if let Some(object) = value.as_object()
            && object.len() == 2
            && object.contains_key("wire")
            && object.contains_key("at")
        {
            let wire = object["wire"].as_str().and_then(|id| parse_id(id, "wire-"))?;
            let at = serde_json::from_value::<Point>(object["at"].clone()).ok().filter(valid_point)?;
            return Some(Self::Split { wire, at });
        }
        serde_json::from_value::<Point>(value.clone()).ok().filter(valid_point).map(Self::Free)
    }

    /// Describe one target in the shared draw-wire command representation.
    pub fn inspect(self) -> Value {
        match self {
            Self::Existing(endpoint) => endpoint.inspect(),
            Self::Free(point) => json!(point),
            Self::Split { wire, at } => json!({"wire":format!("wire-{wire}"),"at":at}),
        }
    }

    /// Resolve a gesture target in its current project; missing identities are rejected.
    pub fn position(self, project: &Project) -> Option<Point> {
        match self {
            Self::Existing(endpoint) => project.endpoint_position(endpoint),
            Self::Free(point) => Some(point),
            Self::Split { wire, at } => {
                project.wires.iter().find(|w| w.ordinal == wire).filter(|w| locate(&project.wire_path(w), at).is_ok()).map(|_| at)
            }
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

/// Where a point lies on a drawn route: on a corner or end (by point index), or strictly inside
/// one leg (by the index of the leg's first point).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnPath {
    Corner(usize),
    Leg(usize),
}

/// Find `at` on an orthogonal route. A route that passes through the same point twice is
/// ambiguous and rejected, as is a point off the route.
pub(crate) fn locate(path: &[Point], at: Point) -> Result<OnPath, CommandError> {
    let strictly_between = |a: i64, b: i64, c: i64| a.min(c) < b && b < a.max(c);
    let corners = path.iter().enumerate().filter(|(_, point)| **point == at).map(|(k, _)| OnPath::Corner(k));
    let legs = path.windows(2).enumerate().filter_map(|(i, pair)| {
        let [a, b] = [pair[0], pair[1]];
        let inside = (a.x_nm == b.x_nm && at.x_nm == a.x_nm && strictly_between(a.y_nm, at.y_nm, b.y_nm))
            || (a.y_nm == b.y_nm && at.y_nm == a.y_nm && strictly_between(a.x_nm, at.x_nm, b.x_nm));
        inside.then_some(OnPath::Leg(i))
    });
    let mut hits = corners.chain(legs);
    match (hits.next(), hits.next()) {
        (Some(hit), None) => Ok(hit),
        (None, _) => Err(CommandError::InvalidParams("that point is not on the wire".to_owned())),
        (Some(_), Some(_)) => Err(CommandError::InvalidParams("the wire passes through that point more than once".to_owned())),
    }
}

/// How a piece of a replaced wire ends: on the original wire's endpoint at that end, on a new
/// free endpoint of its own, or on the one new junction the pieces share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    Keep,
    Fresh,
    Shared,
}

/// One wire made from part of a replaced wire's route, with how each end attaches.
struct Piece {
    start: End,
    route: Vec<Point>,
    end: End,
}

/// The leg strictly containing `at`. A corner touches two legs, so it is rejected as ambiguous.
fn leg_at(path: &[Point], at: Point) -> Result<usize, CommandError> {
    match locate(path, at)? {
        OnPath::Leg(leg) => Ok(leg),
        OnPath::Corner(_) => Err(CommandError::InvalidParams("that point is a corner of two legs; choose a point inside one leg".to_owned())),
    }
}

/// The authored bends of a piece of route: every point but its two ends, within [`MAX_BENDS`].
fn interior(piece: &[Point]) -> Result<Vec<Point>, CommandError> {
    let bends = piece.get(1..piece.len().saturating_sub(1)).unwrap_or_default().to_vec();
    if bends.len() > MAX_BENDS {
        return Err(CommandError::Unavailable(format!("a wire piece would have more than {MAX_BENDS} bends")));
    }
    Ok(bends)
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
    /// Interior wire targets split only on commit; overlaps heal in the same candidate.
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
        let mut edit = heal::Edit::new(project, self.revision());
        let endpoints = match anchors {
            [Anchor::Split { wire: a, at: p }, Anchor::Split { wire: b, at: q }] if a == b => {
                let (from, to) = edit.cut_pair(a, p, q)?;
                [from, to]
            }
            _ => [edit.resolve(anchors[0])?, edit.resolve(anchors[1])?],
        };
        edit.draw(endpoints, &path)?;
        let candidate = edit.finish(auto_heal(params))?;
        self.change(candidate);
        Ok(())
    }

    /// Slice one wire at a point on its drawn route into two wires joined by a new junction there.
    /// Connectivity is unchanged: the halves keep the original endpoints, so a following edit can
    /// branch from or delete either one. The first half keeps the wire's identity. `at` must be on
    /// the route, away from both ends, and on exactly one leg or corner.
    pub(crate) fn slice_wire(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let (index, path, at) = self.wire_target(params)?;
        let (first, second) = match locate(&path, at)? {
            OnPath::Corner(k) if k == 0 || k + 1 == path.len() => {
                return Err(CommandError::InvalidParams("a wire is sliced between its ends, not at one".to_owned()));
            }
            OnPath::Corner(k) => (path[..=k].to_vec(), path[k..].to_vec()),
            OnPath::Leg(i) => (path[..=i].iter().copied().chain([at]).collect(), [at].into_iter().chain(path[i + 1..].iter().copied()).collect()),
        };
        self.replace_wire(
            index,
            vec![Piece { start: End::Keep, route: first, end: End::Shared }, Piece { start: End::Shared, route: second, end: End::Keep }],
        )
    }

    /// Break the straight leg of a wire that contains `at` away from the legs beside it. The leg
    /// stays where it was as its own wire; every place it met a neighbouring leg becomes two free
    /// endpoints, one on each side, so the break is electrical while the drawing is unchanged.
    /// The wire's original endpoints stay attached. A one-leg wire has nothing to break from.
    pub(crate) fn break_wire(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let (index, path, at) = self.wire_target(params)?;
        let leg = leg_at(&path, at)?;
        let (before, after) = (leg > 0, leg + 2 < path.len());
        if !before && !after {
            return Err(CommandError::InvalidParams("a one-leg wire has no neighbouring leg to break from".to_owned()));
        }
        let mut pieces = Vec::new();
        if before {
            pieces.push(Piece { start: End::Keep, route: path[..=leg].to_vec(), end: End::Fresh });
        }
        let ends = (if before { End::Fresh } else { End::Keep }, if after { End::Fresh } else { End::Keep });
        pieces.push(Piece { start: ends.0, route: path[leg..=leg + 1].to_vec(), end: ends.1 });
        if after {
            pieces.push(Piece { start: End::Fresh, route: path[leg + 1..].to_vec(), end: End::Keep });
        }
        self.replace_wire(index, pieces)
    }

    /// Delete the straight leg of a wire that contains `at`, keeping the rest of its route.
    /// Each remaining piece ends at a new free endpoint where the leg was, so the break is
    /// electrical as well as visible; the first remaining piece keeps the wire's identity. A
    /// one-leg wire is deleted whole. A corner touches two legs, so it is rejected as ambiguous.
    pub(crate) fn delete_wire_segment(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let (index, path, at) = self.wire_target(params)?;
        let leg = leg_at(&path, at)?;
        let mut pieces = Vec::new();
        if leg > 0 {
            pieces.push(Piece { start: End::Keep, route: path[..=leg].to_vec(), end: End::Fresh });
        }
        if leg + 2 < path.len() {
            pieces.push(Piece { start: End::Fresh, route: path[leg + 1..].to_vec(), end: End::Keep });
        }
        self.replace_wire(index, pieces)
    }

    /// Replace the wire at `index` with `pieces` of its route as one revision and undo step.
    /// `End::Keep` on a piece's start or end means the original wire's `from` or `to`
    /// endpoint; `End::Fresh` makes a new free endpoint for that piece alone; `End::Shared` uses
    /// one new junction shared by every piece that names it. The first piece keeps the wire's
    /// identity and the others get new IDs; no pieces deletes the wire. Unused free endpoints are
    /// pruned. Limits on wires, bends and IDs are checked before anything changes.
    fn replace_wire(&mut self, index: usize, pieces: Vec<Piece>) -> Result<(), CommandError> {
        let project = self.open_project()?;
        if project.wires.len() - 1 + pieces.len() > MAX_WIRES {
            return Err(CommandError::Unavailable("the schematic wire limit has been reached".to_owned()));
        }
        let mut next = self.next_object_id()?;
        // At most two new endpoints per piece, then the new wire IDs, which must be the highest.
        if next > MAX_OBJECT_ID - 3 * pieces.len() as u64 {
            return Err(CommandError::Unavailable("project has exhausted its object IDs".to_owned()));
        }
        let original = project.wires[index].clone();
        let [from, to] = original.endpoints;
        let mut candidate = project.clone();
        let mut shared = None;
        let mut endpoint = |end: End, kept: WireEndpoint, at: Point, junctions: &mut std::collections::BTreeMap<u64, Point>| match end {
            End::Keep => kept,
            End::Shared => *shared.get_or_insert_with(|| {
                let id = next;
                next += 1;
                junctions.insert(id, at);
                WireEndpoint::Junction(id)
            }),
            End::Fresh => {
                let id = next;
                next += 1;
                junctions.insert(id, at);
                WireEndpoint::Junction(id)
            }
        };
        let mut wires = Vec::new();
        let junctions = Arc::make_mut(&mut candidate.junctions);
        for piece in &pieces {
            let (Some(&first), Some(&last)) = (piece.route.first(), piece.route.last()) else {
                return Err(CommandError::InvalidParams("a wire piece needs a route".to_owned()));
            };
            let endpoints = [endpoint(piece.start, from, first, junctions), endpoint(piece.end, to, last, junctions)];
            wires.push((endpoints, interior(&piece.route)?));
        }
        let list = Arc::make_mut(&mut candidate.wires);
        let mut wires = wires.into_iter();
        match wires.next() {
            Some((endpoints, bends)) => list[index] = Wire { ordinal: original.ordinal, endpoints, bends: Some(bends) },
            None => {
                list.remove(index);
            }
        }
        for (endpoints, bends) in wires {
            list.push(Wire { ordinal: next, endpoints, bends: Some(bends) });
            next += 1;
        }
        candidate.prune_junctions();
        candidate.connectivity = Arc::default();
        self.change(candidate);
        Ok(())
    }

    /// Resolve the `wire` and `at` parameters shared by the segment edits: the wire's index, its
    /// drawn route and the point. Fails when the wire is missing or its route can't be resolved.
    fn wire_target(&self, params: &Value) -> Result<(usize, Vec<Point>, Point), CommandError> {
        let id = params["wire"].as_str().and_then(|value| parse_id(value, "wire-"));
        let project = self.open_project()?;
        let index = project
            .wires
            .iter()
            .position(|wire| Some(wire.ordinal) == id)
            .ok_or_else(|| CommandError::InvalidParams("wire does not exist".to_owned()))?;
        let at = serde_json::from_value::<Point>(params["at"].clone())
            .ok()
            .filter(valid_point)
            .ok_or_else(|| CommandError::InvalidParams("invalid point on the wire".to_owned()))?;
        let path = project.wire_path(&project.wires[index]);
        if path.len() < 2 {
            return Err(CommandError::Unavailable("the wire's route can't be resolved".to_owned()));
        }
        Ok((index, path, at))
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
        BREAK_WIRE, DELETE_WIRE, DELETE_WIRE_SEGMENT, DRAW_WIRE, PLACE_SYMBOL, ROTATE_SYMBOL, SLICE_WIRE,
        persistence::{self, Encoding},
        schematic::{GRID_NM, WireEndpoint},
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

    /// Run a segment edit on a wire at a grid point, at the current revision.
    fn edit(session: &mut Session, command: &str, wire: &str, at: Value) -> Result<u64, CommandError> {
        session.apply(command, &json!({"wire":wire,"at":at,"expected_revision":session.revision()}))
    }

    /// Every wire's drawn route, in wire order.
    fn routes(session: &Session) -> Vec<Vec<Point>> {
        let project = session.project().unwrap();
        project.wires().iter().map(|wire| project.wire_path(wire)).collect()
    }

    /// A grid point as a model point.
    fn grid(x: i64, y: i64) -> Point {
        Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM }
    }

    /// Slicing keeps the route and its connections, splits identity at a new shared junction,
    /// saves with no diff on a second save and undoes as one step.
    #[test]
    fn slicing_a_wire_keeps_its_route_and_connections() {
        let mut session = empty();
        draw(&mut session, point(0, 0), point(8, 6), json!([point(8, 0)]));
        let before = session.snapshot().unwrap();
        let route = routes(&session)[0].clone();
        assert_eq!(route, vec![grid(0, 0), grid(8, 0), grid(8, 6)]);

        edit(&mut session, SLICE_WIRE, "wire-4", point(4, 0)).unwrap();
        let project = session.snapshot().unwrap();
        assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(4, 0)], vec![grid(4, 0), grid(8, 0), grid(8, 6)]]);
        assert_eq!(project.wires()[0].id(), "wire-4", "the first half keeps the wire's identity");
        let [_, middle] = project.wires()[0].endpoints();
        assert_eq!(project.wires()[1].endpoints()[0], middle, "the halves share the new junction");
        assert_eq!(project.connectivity().nets.len(), before.connectivity().nets.len());
        let bytes = persistence::encode(project.clone()).unwrap();
        assert_eq!(persistence::encode(persistence::decode(&bytes).unwrap()).unwrap(), bytes);

        // Slicing at a corner splits there without adding a point to either half.
        session.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        edit(&mut session, SLICE_WIRE, "wire-4", point(8, 0)).unwrap();
        assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(8, 0)], vec![grid(8, 0), grid(8, 6)]]);
        session.apply("edit.undo", &Value::Null).unwrap();
        session.apply("edit.redo", &Value::Null).unwrap();
        assert_eq!(routes(&session).len(), 2);
    }

    /// Ends, points off the route, corners for segment deletion, missing wires and stale revisions
    /// are all rejected without changing the project.
    #[test]
    fn invalid_segment_targets_change_nothing() {
        let mut session = empty();
        draw(&mut session, point(0, 0), point(8, 6), json!([point(8, 0)]));
        let before = session.inspect();
        for (command, wire, at) in [
            (SLICE_WIRE, "wire-4", point(0, 0)),
            (SLICE_WIRE, "wire-4", point(8, 6)),
            (SLICE_WIRE, "wire-4", point(4, 1)),
            (SLICE_WIRE, "wire-99", point(4, 0)),
            (DELETE_WIRE_SEGMENT, "wire-4", point(8, 0)),
            (DELETE_WIRE_SEGMENT, "wire-4", point(0, 0)),
            (DELETE_WIRE_SEGMENT, "wire-4", point(9, 3)),
            (DELETE_WIRE_SEGMENT, "wire-99", point(4, 0)),
        ] {
            assert!(edit(&mut session, command, wire, at.clone()).is_err(), "{command} {wire} {at}");
            assert_eq!(session.inspect(), before);
        }
        let stale = json!({"wire":"wire-4","at":point(4, 0),"expected_revision":session.revision() - 1});
        assert!(session.apply(SLICE_WIRE, &stale).is_err());
        assert!(session.apply(DELETE_WIRE_SEGMENT, &stale).is_err());
        assert!(session.apply(SLICE_WIRE, &json!({"wire":"wire-4","at":{"x_nm":0.5,"y_nm":0},"expected_revision":session.revision()})).is_err());
        assert_eq!(session.inspect(), before);
    }

    /// Deleting a middle leg between two resistors disconnects them and leaves free ends exactly
    /// where the leg was; an unrelated crossing wire and a branch keep their identity and route.
    #[test]
    fn deleting_a_middle_segment_breaks_only_that_leg() {
        let mut session = empty();
        session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
        session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":20 * GRID_NM,"expected_revision":session.revision()})).unwrap();
        let terminal = |symbol: &str, number: &str| crate::schematic::Terminal::parse(&json!({"symbol":symbol,"terminal":number})).unwrap();
        let (one, two) = (terminal("symbol-2", "2"), terminal("symbol-3", "1"));
        let project = session.snapshot().unwrap();
        let (p, q) = (project.terminal_position(one).unwrap(), project.terminal_position(two).unwrap());
        let (east, west, mid) = (p.x_nm + 4 * GRID_NM, q.x_nm - 4 * GRID_NM, 10 * GRID_NM);
        let bend = |x_nm, y_nm| json!({"x_nm":x_nm,"y_nm":y_nm});
        let bends = json!([bend(east, p.y_nm), bend(east, mid), bend(west, mid), bend(west, q.y_nm)]);
        let routed = draw(&mut session, json!({"symbol":"symbol-2","terminal":"2"}), json!({"symbol":"symbol-3","terminal":"1"}), bends);
        let wire = routed["project"]["schematic"]["wires"][0]["id"].as_str().unwrap().to_owned();
        draw(&mut session, bend((east + west) / 2 + GRID_NM, mid - 4 * GRID_NM), bend((east + west) / 2 + GRID_NM, mid + 4 * GRID_NM), json!([]));
        draw(&mut session, bend(east, p.y_nm - 4 * GRID_NM), bend(east + 4 * GRID_NM, p.y_nm - 4 * GRID_NM), json!([]));
        let route = routes(&session)[0].clone();
        assert_eq!(route.len(), 6);
        assert_eq!(session.project().unwrap().connectivity().net_for(one).unwrap(), &[one, two]);
        let others: Vec<_> = session.project().unwrap().wires()[1..].to_vec();
        let other_routes = routes(&session)[1..].to_vec();

        edit(&mut session, DELETE_WIRE_SEGMENT, &wire, bend((east + west) / 2, mid)).unwrap();
        let project = session.snapshot().unwrap();
        assert_eq!(project.wires()[0].id(), wire);
        assert_eq!(routes(&session)[0], route[..=2]);
        assert_eq!(routes(&session).last().unwrap(), &route[3..]);
        assert_eq!(&project.wires()[1..3], others.as_slice(), "the crossing and the other wire are untouched");
        assert_eq!(&routes(&session)[1..3], other_routes.as_slice());
        assert_eq!(project.connectivity().net_for(one).unwrap(), &[one]);
        assert_eq!(project.connectivity().net_for(two).unwrap(), &[two]);
        let bytes = persistence::encode(project.clone()).unwrap();
        assert_eq!(persistence::encode(persistence::decode(&bytes).unwrap()).unwrap(), bytes);
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(session.project().unwrap().connectivity().net_for(one).unwrap(), &[one, two]);
        assert_eq!(routes(&session)[0], route);
    }

    /// Breaking a middle leg keeps all three pieces where they were but disconnects them: each
    /// gap has a free endpoint on either side. An end leg keeps the wire's original endpoint, and
    /// a one-leg wire or a corner is refused.
    #[test]
    fn breaking_a_leg_separates_it_from_its_neighbours() {
        let mut session = empty();
        session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
        let pin = json!({"symbol":"symbol-2","terminal":"2"});
        let terminal = crate::schematic::Terminal::parse(&pin).unwrap();
        let start = session.project().unwrap().terminal_position(terminal).unwrap();
        let (x, y) = (start.x_nm, start.y_nm);
        let p = |dx: i64, dy: i64| json!({"x_nm":x + dx * GRID_NM,"y_nm":y + dy * GRID_NM});
        let drawn = draw(&mut session, pin.clone(), p(8, 8), json!([p(4, 0), p(4, 8)]));
        let id = drawn["project"]["schematic"]["wires"][0]["id"].as_str().unwrap().to_owned();
        let route = routes(&session)[0].clone();
        assert_eq!(route.len(), 4);
        let before = session.inspect();
        for (command, at) in [(BREAK_WIRE, p(4, 0)), (BREAK_WIRE, p(9, 8))] {
            assert!(edit(&mut session, command, &id, at).is_err());
            assert_eq!(session.inspect(), before);
        }

        edit(&mut session, BREAK_WIRE, &id, p(4, 3)).unwrap();
        assert_eq!(routes(&session), vec![route[..=1].to_vec(), route[1..=2].to_vec(), route[2..].to_vec()]);
        let project = session.snapshot().unwrap();
        let wires = project.wires();
        assert_eq!(wires[0].endpoints()[0], WireEndpoint::Terminal(terminal), "the pin stays attached");
        assert_ne!(wires[0].endpoints()[1], wires[1].endpoints()[0], "the gap is a real break");
        assert_ne!(wires[1].endpoints()[1], wires[2].endpoints()[0]);
        assert_eq!(project.junctions().count(), 5);
        assert_eq!(project.connectivity().net_for(terminal).unwrap(), &[terminal]);
        let bytes = persistence::encode(project.clone()).unwrap();
        assert_eq!(persistence::encode(persistence::decode(&bytes).unwrap()).unwrap(), bytes);
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(session.inspect()["project"], before["project"]);

        // The first leg keeps the pin and gets a free end only where it met the next leg.
        edit(&mut session, BREAK_WIRE, &id, p(2, 0)).unwrap();
        assert_eq!(routes(&session), vec![route[..=1].to_vec(), route[1..].to_vec()]);
        assert_eq!(session.project().unwrap().wires()[0].endpoints()[0], WireEndpoint::Terminal(terminal));

        let mut single = empty();
        draw(&mut single, point(0, 0), point(8, 0), json!([]));
        let before = single.inspect();
        assert!(edit(&mut single, BREAK_WIRE, "wire-4", point(4, 0)).is_err());
        assert_eq!(single.inspect(), before);
    }

    /// Deleting an end leg shortens the wire in place; deleting a one-leg wire's only leg removes
    /// it with its unused free endpoints.
    #[test]
    fn deleting_an_end_or_only_segment_shortens_or_removes_the_wire() {
        let mut session = empty();
        draw(&mut session, point(0, 0), point(8, 6), json!([point(8, 0)]));
        edit(&mut session, DELETE_WIRE_SEGMENT, "wire-4", point(8, 3)).unwrap();
        assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(8, 0)]]);
        assert_eq!(session.project().unwrap().wires()[0].id(), "wire-4");
        assert_eq!(session.project().unwrap().junctions().count(), 2, "the far end was pruned and the cut end added");
        edit(&mut session, DELETE_WIRE_SEGMENT, "wire-4", point(1, 0)).unwrap();
        assert!(session.project().unwrap().wires().is_empty());
        assert_eq!(session.project().unwrap().junctions().count(), 0);
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
        assert_eq!(stored["format"], "pcbcraft-project/4");
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
