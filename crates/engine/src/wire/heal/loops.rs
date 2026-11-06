//! Loop removal only follows existing electrical identities. Crossings between separate
//! routes never acquire a connection, and no pin or intermediate branch may be discarded.

use super::{Edit, on_segment, simplify};
use crate::{
    CommandError, Project,
    schematic::{Point, Wire, WireEndpoint},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Find the first intersection along the later orthogonal leg, including a collinear return.
fn intersection(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let candidates = [c, d, a, b, Point { x_nm: a.x_nm, y_nm: c.y_nm }, Point { x_nm: c.x_nm, y_nm: a.y_nm }];
    candidates
        .into_iter()
        .filter(|&p| on_segment(p, a, b) && on_segment(p, c, d))
        .min_by_key(|p| u128::from(p.x_nm.abs_diff(c.x_nm)) + u128::from(p.y_nm.abs_diff(c.y_nm)))
}

/// Compare complete routed lengths without overflow at the schematic coordinate limits.
fn length(project: &Project, wire: &Wire) -> u128 {
    project.wire_path(wire).windows(2).map(|p| u128::from(p[0].x_nm.abs_diff(p[1].x_nm)) + u128::from(p[0].y_nm.abs_diff(p[1].y_nm))).sum()
}

impl Edit {
    /// Erase returns to an earlier point on this same wire. A wire has no internal electrical
    /// anchors, so its endpoints remain fixed and no attached branch or pin can be skipped.
    /// Each splice removes points; the shared comparison budget bounds dense self-intersections.
    pub(super) fn erase_path_loops(&mut self, path: &mut Vec<Point>) -> Result<(), CommandError> {
        loop {
            let mut found = None;
            'scan: for later in 2..path.len().saturating_sub(1) {
                for earlier in 0..later - 1 {
                    self.compare()?;
                    if let Some(at) = intersection(path[earlier], path[earlier + 1], path[later], path[later + 1]) {
                        found = Some((earlier, later, at));
                        break 'scan;
                    }
                }
            }
            let Some((earlier, later, at)) = found else { return Ok(()) };
            path.splice(earlier + 1..later + 1, [at]);
            simplify(path);
        }
    }

    /// Remove one redundant alternative route to a touched edge. Only degree-two junctions
    /// may lie inside the alternative; pins and other branches stop the walk. Keep the shorter
    /// path, breaking equal-length ties by the older wire ID. Geometry alone never joins nets.
    pub(super) fn remove_redundant_loop(&mut self) -> Result<bool, CommandError> {
        let wires = self.project.wires.clone();
        let mut adjacent = BTreeMap::<WireEndpoint, Vec<usize>>::new();
        for (index, wire) in wires.iter().enumerate() {
            for end in wire.endpoints {
                adjacent.entry(end).or_default().push(index);
            }
        }
        for (index, wire) in wires.iter().enumerate() {
            if !self.touched.contains(&wire.ordinal) {
                continue;
            }
            let [from, to] = wire.endpoints;
            let Some(starts) = adjacent.get(&from) else { continue };
            for &start in starts.iter().filter(|&&i| i != index) {
                let mut visited = BTreeSet::from([from]);
                let mut at = from;
                let mut next = start;
                let mut alternative = Vec::new();
                loop {
                    self.compare()?;
                    if next == index {
                        break;
                    }
                    alternative.push(next);
                    let ends = wires[next].endpoints;
                    at = if ends[0] == at { ends[1] } else { ends[0] };
                    if at == to {
                        let distance: u128 = alternative.iter().map(|&i| length(&self.project, &wires[i])).sum();
                        let oldest = alternative.iter().map(|&i| wires[i].ordinal).min().unwrap_or(wire.ordinal);
                        let remove: BTreeSet<_> = if (length(&self.project, wire), wire.ordinal) > (distance, oldest) {
                            BTreeSet::from([wire.ordinal])
                        } else {
                            alternative.iter().map(|&i| wires[i].ordinal).collect()
                        };
                        self.touched.extend(alternative.iter().map(|&i| wires[i].ordinal));
                        Arc::make_mut(&mut self.project.wires).retain(|w| !remove.contains(&w.ordinal));
                        self.project.prune_junctions();
                        return Ok(true);
                    }
                    if !matches!(at, WireEndpoint::Junction(_)) || !visited.insert(at) {
                        break;
                    }
                    let Some(edges) = adjacent.get(&at).filter(|edges| edges.len() == 2) else { break };
                    next = if edges[0] == next { edges[1] } else { edges[0] };
                }
            }
        }
        Ok(false)
    }
}
