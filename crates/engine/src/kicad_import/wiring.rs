//! KiCad's wires as native wire routes, and KiCad's nets worked out from the file alone.
//!
//! KiCad draws wires as loose segments that connect where an end, a pin, a label or a junction
//! dot lies on them; crossings without a dot stay apart. Native wires are routes between
//! explicit endpoints. [`plan`] splits KiCad's segments at every point that connects, then joins
//! runs through plain bends back into one route each. [`kicad_nets`] works out which pins KiCad
//! connects, independently of the native model, so the import can check the result.

use pcbcraft_kicad::Point;
use std::collections::{BTreeMap, BTreeSet};

/// A straight piece of wire between two distinct points, horizontal or vertical.
pub(super) type Segment = (Point, Point);

/// One native wire to draw: from one point to another through orthogonal bends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Route {
    pub(super) from: Point,
    pub(super) to: Point,
    pub(super) bends: Vec<Point>,
}

/// The straight pieces of KiCad's wires, deduplicated, each ordered from its lesser end.
/// Fails at the first diagonal piece, which native wires can't draw.
pub(super) fn segments(wires: &[Vec<Point>]) -> Result<Vec<Segment>, Point> {
    let mut out = BTreeSet::new();
    for wire in wires {
        for pair in wire.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a == b {
                continue;
            }
            if a.x_nm != b.x_nm && a.y_nm != b.y_nm {
                return Err(a);
            }
            out.insert(if a < b { (a, b) } else { (b, a) });
        }
    }
    Ok(out.into_iter().collect())
}

/// Split every segment at each point in `cuts` that lies inside it, plus every other segment's
/// end that does: the pieces connect there in KiCad. Overlapping runs on one line are merged
/// first, since every point inside them is shared; points are indexed by line, so the work
/// follows the number of segments and points rather than their product.
pub(super) fn split(segments: &[Segment], cuts: &BTreeSet<Point>) -> Vec<Segment> {
    let mut on_x: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    let mut on_y: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    for p in segments.iter().flat_map(|(a, b)| [*a, *b]).chain(cuts.iter().copied()) {
        on_x.entry(p.x_nm).or_default().insert(p.y_nm);
        on_y.entry(p.y_nm).or_default().insert(p.x_nm);
    }
    // Each line's runs as (start, end) along it; segments are ordered from their lesser end.
    let mut vertical: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
    let mut horizontal: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
    for &(a, b) in segments {
        if a.x_nm == b.x_nm {
            vertical.entry(a.x_nm).or_default().push((a.y_nm, b.y_nm));
        } else {
            horizontal.entry(a.y_nm).or_default().push((a.x_nm, b.x_nm));
        }
    }
    let mut out = Vec::new();
    for (lines, points, vertical) in [(&mut vertical, &on_x, true), (&mut horizontal, &on_y, false)] {
        for (line, runs) in lines.iter_mut() {
            let at = |along: i64| if vertical { Point { x_nm: *line, y_nm: along } } else { Point { x_nm: along, y_nm: *line } };
            for (start, end) in merged(runs) {
                let inside = points.get(line).into_iter().flat_map(|stops| stops.range(start..=end));
                let stops: Vec<i64> = inside.copied().collect();
                for pair in stops.windows(2) {
                    out.push((at(pair[0]), at(pair[1])));
                }
            }
        }
    }
    out.sort();
    out
}

/// Overlapping or touching runs on one line as single runs, in order.
fn merged(runs: &mut [(i64, i64)]) -> Vec<(i64, i64)> {
    runs.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for &(start, end) in runs.iter() {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// Join pieces into routes. A route ends at every point in `stops` (pins, labels, junction dots
/// and no-connect marks) and wherever pieces meet other than two at a time; it bends through
/// the rest. A closed loop with no such point is broken at its least point.
pub(super) fn plan(pieces: &[Segment], stops: &BTreeSet<Point>) -> Vec<Route> {
    let mut neighbors: BTreeMap<Point, Vec<Point>> = BTreeMap::new();
    for &(a, b) in pieces {
        neighbors.entry(a).or_default().push(b);
        neighbors.entry(b).or_default().push(a);
    }
    let is_end = |p: &Point| stops.contains(p) || neighbors.get(p).is_none_or(|n| n.len() != 2);
    let mut used: BTreeSet<Segment> = BTreeSet::new();
    let key = |a: Point, b: Point| if a < b { (a, b) } else { (b, a) };
    let mut routes = Vec::new();
    // Walk from every end first, then from what is left: loops of plain bends.
    let starts: Vec<Point> = neighbors.keys().copied().filter(|p| is_end(p)).chain(neighbors.keys().copied()).collect();
    for start in starts {
        let firsts = neighbors.get(&start).cloned().unwrap_or_default();
        for first in firsts {
            if used.contains(&key(start, first)) {
                continue;
            }
            let mut path = vec![start, first];
            used.insert(key(start, first));
            let mut here = first;
            // Through plain bends, which have exactly two pieces: take the one not walked yet.
            while !is_end(&here) && here != start {
                let Some(next) = neighbors.get(&here).and_then(|n| n.iter().copied().find(|&n| !used.contains(&key(here, n)))) else {
                    break;
                };
                used.insert(key(here, next));
                here = next;
                path.push(here);
            }
            routes.extend(drawable(&path).iter().map(|piece| route(piece)));
        }
    }
    routes
}

/// A walked path as pieces native wires can draw: a closed loop is cut in two, since a wire
/// needs two distinct ends, and a run with more corners than a wire may bend through is cut at
/// a corner. The cut points become ends the next piece starts from.
fn drawable(path: &[Point]) -> Vec<Vec<Point>> {
    let mut pieces = if path.len() >= 3 && path.first() == path.last() {
        let middle = path.len() / 2;
        vec![path[..=middle].to_vec(), path[middle..].to_vec()]
    } else {
        vec![path.to_vec()]
    };
    let mut out = Vec::new();
    while let Some(piece) = pieces.pop() {
        let corners: Vec<usize> = (1..piece.len().saturating_sub(1)).filter(|&i| !straight(piece[i - 1], piece[i], piece[i + 1])).collect();
        match corners.get(crate::wire::MAX_BENDS) {
            Some(&cut) => {
                pieces.push(piece[cut..].to_vec());
                pieces.push(piece[..=cut].to_vec());
            }
            None => out.push(piece),
        }
    }
    out
}

/// Whether `b` lies on the straight line from `a` to `c`, so it is no corner.
fn straight(a: Point, b: Point, c: Point) -> bool {
    (a.x_nm == b.x_nm && b.x_nm == c.x_nm) || (a.y_nm == b.y_nm && b.y_nm == c.y_nm)
}

/// A route along `path`, keeping only the corners as bends.
fn route(path: &[Point]) -> Route {
    let mut bends = Vec::new();
    for window in path.windows(3) {
        if !straight(window[0], window[1], window[2]) {
            bends.push(window[1]);
        }
    }
    Route { from: path[0], to: path[path.len() - 1], bends }
}

/// Union-find over small integer keys.
struct Sets(Vec<usize>);

impl Sets {
    /// The representative of `i`'s set, halving paths on the way.
    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
    /// Merge the sets of `a` and `b`; nothing changes when they are one set already.
    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a] = b;
        }
    }
}

/// What can join a net in KiCad: a pin (by caller-chosen key) at a point, a named label at a
/// point, or a named global net with no point of its own (a power symbol's net).
pub(super) struct Contacts<K> {
    pub(super) pins: Vec<(K, Point)>,
    /// `(scope, name, point)`: labels of the same scope and name join.
    pub(super) labels: Vec<(&'static str, String, Point)>,
    /// `(pin key, net name)`: power symbols join every power symbol of the same name.
    pub(super) power: Vec<(K, String)>,
}

/// Which pins KiCad connects, as groups of pin keys; every key appears in exactly one group.
/// Pin entries that share a key are one pin and always join.
/// Pieces must already be split ([`split`]) so that connections lie at their ends.
pub(super) fn kicad_nets<K: Ord + Clone>(pieces: &[Segment], contacts: &Contacts<K>) -> BTreeSet<BTreeSet<K>> {
    let mut index: BTreeMap<Point, usize> = BTreeMap::new();
    let point = |p: Point, index: &mut BTreeMap<Point, usize>| {
        let next = index.len();
        *index.entry(p).or_insert(next)
    };
    let pins: Vec<usize> = contacts.pins.iter().map(|(_, p)| point(*p, &mut index)).collect();
    let pieces: Vec<(usize, usize)> = pieces.iter().map(|(a, b)| (point(*a, &mut index), point(*b, &mut index))).collect();
    let labels: Vec<usize> = contacts.labels.iter().map(|(_, _, p)| point(*p, &mut index)).collect();
    let mut names: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut count = index.len();
    for (scope, name, _) in &contacts.labels {
        names.entry((scope, name)).or_insert_with(|| {
            count += 1;
            count - 1
        });
    }
    for (_, name) in &contacts.power {
        names.entry(("power", name)).or_insert_with(|| {
            count += 1;
            count - 1
        });
    }
    let first_pin_key = count;
    let mut sets = Sets((0..count + contacts.pins.len()).collect());
    for (a, b) in pieces {
        sets.join(a, b);
    }
    for (label, (scope, name, _)) in labels.iter().zip(&contacts.labels) {
        if let Some(&n) = names.get(&(*scope, name.as_str())) {
            sets.join(*label, n);
        }
    }
    let by_key: BTreeMap<&K, usize> = contacts.pins.iter().enumerate().map(|(i, (k, _))| (k, i)).collect();
    for (i, at) in pins.iter().enumerate() {
        sets.join(first_pin_key + i, *at);
    }
    // Pins with one key are one pin, such as a pin every unit of a part shares.
    for (i, (key, _)) in contacts.pins.iter().enumerate() {
        if let Some(&first) = by_key.get(key) {
            sets.join(first_pin_key + i, first_pin_key + first);
        }
    }
    for (key, name) in &contacts.power {
        if let (Some(&i), Some(&n)) = (by_key.get(key), names.get(&("power", name.as_str()))) {
            sets.join(first_pin_key + i, n);
        }
    }
    let mut groups: BTreeMap<usize, BTreeSet<K>> = BTreeMap::new();
    for (i, (key, _)) in contacts.pins.iter().enumerate() {
        let root = sets.find(first_pin_key + i);
        groups.entry(root).or_default().insert(key.clone());
    }
    groups.into_values().collect()
}
