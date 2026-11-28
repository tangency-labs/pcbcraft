//! Deliberately joining two wires where they cross. A crossing alone never connects; this edit
//! splits both selected wires at one ordinary crossing and gives the four pieces one shared
//! junction, so the routes keep their drawn geometry and other wires stay independent.
//!
//! [`Project::wire_join`] checks the request and describes the join without editing, for the
//! editor's menu preview and the command's dry run. The command builds the edit in a disposable
//! sheet transaction and checks it with the native encoder before previewing or publishing it.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{OnPath, heal, locate, path_contains_point, valid_point};
use crate::{
    CommandError, JOIN_WIRES, Project, Session,
    schematic::{Point, Wire},
};

/// The refusal for a point where the wires touch without an ordinary crossing.
const NOT_ORDINARY: &str =
    "the wires meet at an end, a corner or an overlap there, not at an ordinary crossing; draw a wire to that point to connect it";

/// A checked join of two wires at one ordinary crossing, as previewed and as applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireJoin {
    /// The two wires' stable numeric IDs, in request order.
    pub wires: [u64; 2],
    /// The exact crossing point that becomes a junction.
    pub at: Point,
}

impl Project {
    /// Check that wires `wire` and `other` cross at one ordinary point of this sheet, where each
    /// passes straight through the inside of one leg at right angles to the other. Without `at`
    /// the wires must cross exactly once. Refused when no other wire, pin or sheet pin may share
    /// that point (the junction dot would look connected to it), or when both wires carry authored
    /// net names and none is shared, since the joined net would then have two names. Nothing is
    /// edited.
    pub fn wire_join(&self, wire: &str, other: &str, at: Option<Point>) -> Result<WireJoin, CommandError> {
        let find =
            |id: &str| self.wires.iter().find(|candidate| candidate.id() == id).ok_or_else(|| CommandError::NotFound("wire does not exist".into()));
        let (a, b) = (find(wire)?, find(other)?);
        if a.ordinal == b.ordinal {
            return Err(CommandError::InvalidParams("choose two different wires".into()));
        }
        let (route_a, route_b) = (self.wire_path(a), self.wire_path(b));
        let meetings = meetings(&route_a, &route_b);
        let crossings: Vec<_> = meetings.iter().copied().filter(|&point| crosses(&route_a, &route_b, point)).collect();
        let at = match (at, crossings.as_slice()) {
            (Some(at), _) if crossings.contains(&at) => at,
            (Some(at), _) if meetings.contains(&at) => return Err(CommandError::InvalidParams(NOT_ORDINARY.into())),
            (Some(_), _) => return Err(CommandError::InvalidParams("the wires do not cross at that point".into())),
            (None, [at]) => *at,
            (None, []) if meetings.is_empty() => return Err(CommandError::InvalidParams("the wires do not cross".into())),
            (None, []) => return Err(CommandError::InvalidParams(NOT_ORDINARY.into())),
            (None, many) => {
                return Err(CommandError::InvalidParams(format!("the wires cross at {} points; choose one with `at`", many.len())));
            }
        };
        let busy =
            self.wires.iter().any(|wire| wire.ordinal != a.ordinal && wire.ordinal != b.ordinal && path_contains_point(&self.wire_path(wire), at))
                || self.symbols.iter().any(|symbol| symbol.pins().contains(&at))
                || self.hierarchy_endpoints().any(|(_, point)| point == at);
        if busy {
            return Err(CommandError::InvalidParams("another wire or pin is at this crossing; move it away to join only the chosen wires".into()));
        }
        self.check_join_names(a, b)?;
        Ok(WireJoin { wires: [a.ordinal, b.ordinal], at })
    }

    /// The ordinary crossings of two wires in route order, so an editor can offer the one
    /// nearest the pointer. Missing wires have none.
    pub fn wire_crossings(&self, wire: &str, other: &str) -> Vec<Point> {
        let route = |id: &str| self.wires.iter().find(|candidate| candidate.id() == id).map(|found| self.wire_path(found));
        let (Some(a), Some(b)) = (route(wire), route(other)) else {
            return Vec::new();
        };
        meetings(&a, &b).into_iter().filter(|&point| crosses(&a, &b, point)).collect()
    }

    /// Refuse a join of two differently named nets. A net with authored names shows one of
    /// them and keeps the others as aliases; that presentation does not resolve an electrical
    /// conflict, so a deliberate join leaves such a choice to the designer.
    fn check_join_names(&self, a: &Wire, b: &Wire) -> Result<(), CommandError> {
        let graph = self.connectivity();
        let [Some(first), Some(second)] = [a, b].map(|wire| graph.net_index_for_endpoint(wire.endpoints[0])) else {
            return Ok(());
        };
        let (names, others) = (graph.net_names(first), graph.net_names(second));
        if first != second && !names.is_empty() && !others.is_empty() && names.iter().all(|name| !others.contains(name)) {
            return Err(CommandError::InvalidParams(format!(
                "the wires are on nets named {} and {}; joining them would give one net two names. Rename or remove a label first",
                graph.net_name(first),
                graph.net_name(second)
            )));
        }
        Ok(())
    }

    /// Describe a checked join in the command's reply shape: the wires, the point, whether they
    /// are already connected elsewhere, and each wire's net with its authored names and terminal
    /// count. Counts keep the reply small for nets with many pins.
    pub fn inspect_wire_join(&self, join: &WireJoin) -> Value {
        let graph = self.connectivity();
        let indexes =
            join.wires.map(|id| self.wires.iter().find(|wire| wire.ordinal == id).and_then(|wire| graph.net_index_for_endpoint(wire.endpoints[0])));
        let nets: Vec<_> = join
            .wires
            .iter()
            .zip(indexes)
            .map(|(id, index)| {
                json!({
                    "wire": format!("wire-{id}"),
                    "net": index.map_or_else(|| "Unconnected".to_owned(), |index| graph.net_name(index)),
                    "names": index.map(|index| graph.net_names(index)).unwrap_or_default(),
                    "terminals": index.and_then(|index| graph.nets.get(index)).map_or(0, Vec::len),
                })
            })
            .collect();
        let connected = indexes[0].is_some() && indexes[0] == indexes[1];
        json!({"wires": join.wires.map(|id| format!("wire-{id}")), "at": join.at, "already_connected": connected, "nets": nets})
    }
}

/// Every point where two orthogonal routes meet: perpendicular legs that touch, and both ends
/// of a collinear run they share. Each point is listed once, in route order.
fn meetings(a: &[Point], b: &[Point]) -> Vec<Point> {
    let mut found = Vec::new();
    for first in a.windows(2) {
        for second in b.windows(2) {
            let points = match heal::overlap(first, second) {
                Some((low, high)) => vec![low, high],
                None => touch(first, second).into_iter().collect(),
            };
            for point in points {
                if !found.contains(&point) {
                    found.push(point);
                }
            }
        }
    }
    found
}

/// The one point two orthogonal legs share without a collinear overlap, if any.
fn touch(a: &[Point], b: &[Point]) -> Option<Point> {
    let vertical = |leg: &[Point]| leg[0].x_nm == leg[1].x_nm;
    let candidate = match (vertical(a), vertical(b)) {
        (true, false) => Point { x_nm: a[0].x_nm, y_nm: b[0].y_nm },
        (false, true) => Point { x_nm: b[0].x_nm, y_nm: a[0].y_nm },
        // Collinear legs that meet end to end share an end point; a longer overlap is handled by the caller.
        _ => return [a[0], a[1]].into_iter().find(|end| path_contains_point(b, *end)),
    };
    (path_contains_point(a, candidate) && path_contains_point(b, candidate)).then_some(candidate)
}

/// Whether both routes pass straight through `point` inside one leg each, at right angles.
fn crosses(a: &[Point], b: &[Point], point: Point) -> bool {
    match (locate(a, point), locate(b, point)) {
        (Ok(OnPath::Leg(i)), Ok(OnPath::Leg(j))) => (a[i].x_nm == a[i + 1].x_nm) != (b[j].x_nm == b[j + 1].x_nm),
        _ => false,
    }
}

/// Read the optional crossing point of a join request.
fn at_param(params: &Value) -> Result<Option<Point>, CommandError> {
    match params.get("at").filter(|value| !value.is_null()) {
        None => Ok(None),
        Some(value) => serde_json::from_value::<Point>(value.clone())
            .ok()
            .filter(valid_point)
            .map(Some)
            .ok_or_else(|| CommandError::InvalidParams("invalid crossing point".into())),
    }
}

/// The two wire IDs of a join request.
fn wire_params(params: &Value) -> [&str; 2] {
    ["wire", "other"].map(|key| params[key].as_str().unwrap_or_default())
}

impl Session {
    /// Join two crossing wires as one revision-checked undo step, or preview the join with
    /// `dry_run`. The request is checked on its sheet first, then the edit is built in a
    /// disposable sheet transaction and the whole candidate is checked with the native encoder,
    /// so a refused join or a dry run leaves the project, revision and history unchanged. The
    /// reply is [`Project::inspect_wire_join`] with the revision and, once applied, the junction.
    pub(crate) fn join_wires(&mut self, params: &Value) -> Result<Value, CommandError> {
        self.authorize_command(JOIN_WIRES)?;
        self.refuse_stale(JOIN_WIRES, params)?;
        self.validate_project_command(JOIN_WIRES, params)?;
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = project.resolve_sheet(params.get("sheet").and_then(Value::as_str))?;
        let sheet = project.for_sheet(target).ok_or_else(|| CommandError::NotFound("unknown sheet ID".into()))?;
        let [wire, other] = wire_params(params);
        let join = sheet.wire_join(wire, other, at_param(params)?)?;
        let mut reply = sheet.inspect_wire_join(&join);
        let mut transaction = Self {
            project: self.project.clone(),
            libraries: self.libraries.clone(),
            revision: self.revision,
            history_root_revision: self.history_root_revision,
            ..Self::default()
        };
        transaction.apply_to_sheet(JOIN_WIRES, params)?;
        let candidate = self.retain_allocation_watermark(Arc::new(transaction.open_project()?.clone()));
        crate::persistence::encode_documents(candidate.clone()).map_err(CommandError::InvalidParams)?;
        if params["dry_run"].as_bool() != Some(true) {
            let junction = candidate
                .for_sheet(target)
                .and_then(|sheet| sheet.wires.iter().find(|wire| wire.ordinal == join.wires[0]).map(|wire| wire.endpoints[1].inspect()));
            self.publish_change(candidate, transaction.revision);
            self.report_committed(JOIN_WIRES, params);
            reply["junction"] = json!(junction);
        }
        reply["revision"] = json!(self.revision);
        Ok(reply)
    }

    /// The edit itself, run on a sheet projection inside [`Self::join_wires`]'s transaction:
    /// split both wires at the crossing and merge the two new junctions into one. Healing is
    /// not run, so both routes keep their drawn geometry.
    pub(crate) fn join_crossing(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let [wire, other] = wire_params(params);
        let join = project.wire_join(wire, other, at_param(params)?)?;
        let mut edit = heal::Edit::new(project, self.revision());
        let first = edit.split(join.wires[0], join.at)?;
        let second = edit.split(join.wires[1], join.at)?;
        edit.merge(first, second)?;
        let candidate = edit.finish(false)?;
        self.change(candidate)
    }
}

#[cfg(test)]
mod tests;
