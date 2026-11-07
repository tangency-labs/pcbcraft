//! Bounded normalization of deliberately edited wire geometry, adapted from the independent
//! PCBcraft Next overlap/split/merge workflow. Crossings alone never create graph edges.

use super::{Anchor, MAX_BENDS, OnPath, locate, valid_point};
use crate::{
    CommandError, MAX_OBJECT_ID, Project,
    schematic::{Point, Wire, WireEndpoint, wiring::MAX_WIRES},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

mod loops;

/// Normalize one orthogonal draft using the same retrace and self-loop cleanup as committed
/// edits. Endpoints stay fixed; this never inspects other wires or creates electrical links.
/// Accept at most twice the authored route bound to allow a temporary elbow at each anchor.
/// Invalid coordinates, diagonal legs and excessive comparison work fail without mutation.
pub fn normalized_path(path: &[Point]) -> Result<Vec<Point>, CommandError> {
    if path.is_empty()
        || path.len() > 2 * (MAX_BENDS + 2)
        || !path.iter().all(valid_point)
        || path.windows(2).any(|p| p[0].x_nm != p[1].x_nm && p[0].y_nm != p[1].y_nm)
    {
        return Err(CommandError::InvalidParams("invalid orthogonal wire preview".into()));
    }
    let mut path = path.to_vec();
    simplify(&mut path);
    loops::erase_path_loops(&mut path, &mut 200_000)?;
    Ok(path)
}

/// One disposable edit owns allocation and comparison budgets; nothing reaches history on error.
pub(super) struct Edit {
    pub project: Project,
    next: u64,
    budget: usize,
    pub touched: BTreeSet<u64>,
}

/// Remove repeated, redundant collinear and doubled-back points without changing the endpoints.
fn simplify(path: &mut Vec<Point>) {
    path.dedup();
    let mut i = 1;
    while i + 1 < path.len() {
        let (a, b, c) = (path[i - 1], path[i], path[i + 1]);
        if (a.x_nm == b.x_nm && b.x_nm == c.x_nm) || (a.y_nm == b.y_nm && b.y_nm == c.y_nm) {
            path.remove(i);
            path.dedup();
            i = i.saturating_sub(1).max(1);
        } else {
            i += 1;
        }
    }
}

/// A nonzero orthogonal interval includes its boundary, for intentional endpoint contact.
fn on_segment(p: Point, a: Point, b: Point) -> bool {
    (a.x_nm == b.x_nm && p.x_nm == a.x_nm && (a.y_nm.min(b.y_nm)..=a.y_nm.max(b.y_nm)).contains(&p.y_nm))
        || (a.y_nm == b.y_nm && p.y_nm == a.y_nm && (a.x_nm.min(b.x_nm)..=a.x_nm.max(b.x_nm)).contains(&p.x_nm))
}

/// Locate the nearer end of a three-leg U-shaped return. The shortcut must land inside the
/// first leg; a plain elbow, a straight continuation or a return past that leg is unchanged.
fn return_corner(path: &[Point]) -> Option<Point> {
    let [a, b, c, d] = *path.last_chunk::<4>()?;
    let at = if a.x_nm == b.x_nm && b.y_nm == c.y_nm && c.x_nm == d.x_nm {
        Point { x_nm: b.x_nm, y_nm: d.y_nm }
    } else if a.y_nm == b.y_nm && b.x_nm == c.x_nm && c.y_nm == d.y_nm {
        Point { x_nm: d.x_nm, y_nm: b.y_nm }
    } else {
        return None;
    };
    (at != a && at != b && at != d && on_segment(at, a, b)).then_some(at)
}

/// A strictly positive collinear overlap; perpendicular crossings and point contact are excluded.
fn overlap(a: &[Point], b: &[Point]) -> Option<(Point, Point)> {
    if a[0].y_nm == a[1].y_nm && b[0].y_nm == b[1].y_nm && a[0].y_nm == b[0].y_nm {
        let lo = a[0].x_nm.min(a[1].x_nm).max(b[0].x_nm.min(b[1].x_nm));
        let hi = a[0].x_nm.max(a[1].x_nm).min(b[0].x_nm.max(b[1].x_nm));
        (lo < hi).then_some((Point { x_nm: lo, y_nm: a[0].y_nm }, Point { x_nm: hi, y_nm: a[0].y_nm }))
    } else if a[0].x_nm == a[1].x_nm && b[0].x_nm == b[1].x_nm && a[0].x_nm == b[0].x_nm {
        let lo = a[0].y_nm.min(a[1].y_nm).max(b[0].y_nm.min(b[1].y_nm));
        let hi = a[0].y_nm.max(a[1].y_nm).min(b[0].y_nm.max(b[1].y_nm));
        (lo < hi).then_some((Point { x_nm: a[0].x_nm, y_nm: lo }, Point { x_nm: a[0].x_nm, y_nm: hi }))
    } else {
        None
    }
}

impl Edit {
    /// Finish a candidate with optional normalization. With cleanup disabled, retain authored
    /// paths but reject graph shapes the native format cannot represent; never save invalid
    /// parallel edges or a wire whose endpoints have become the same identity.
    pub fn finish(mut self, auto_heal: bool) -> Result<Project, CommandError> {
        if auto_heal {
            return self.heal();
        }
        let mut pairs = BTreeSet::new();
        for wire in self.project.wires.iter() {
            let [a, b] = wire.endpoints;
            if a == b || !pairs.insert((a.min(b), a.max(b))) {
                return Err(CommandError::InvalidParams("this connection needs wire healing; enable Auto-heal or separate the routes".into()));
            }
        }
        self.project.prune_junctions();
        self.project.connectivity = Arc::default();
        Ok(self.project)
    }
    /// Find one unambiguous electrical identity or interior route at a contact point, excluding
    /// routes already attached to the moving symbol. A crossing has no preferred branch.
    fn contact(&self, point: Point, symbol: u64) -> Result<Option<Anchor>, CommandError> {
        let mut found = None;
        for wire in self.project.wires.iter().filter(|w| !w.terminals().iter().any(|p| p.symbol == symbol)) {
            let path = self.project.wire_path(wire);
            if !path.windows(2).any(|s| on_segment(point, s[0], s[1])) {
                continue;
            }
            let anchor = if path.first() == Some(&point) {
                Anchor::Existing(wire.endpoints[0])
            } else if path.last() == Some(&point) {
                Anchor::Existing(wire.endpoints[1])
            } else {
                Anchor::Split { wire: wire.ordinal, at: point }
            };
            if found.is_some_and(|old| old != anchor) {
                return Err(CommandError::InvalidParams("pin drop meets ambiguous wire connections".into()));
            }
            found = Some(anchor);
        }
        Ok(found)
    }

    /// Connect moved/placed pins at exact contacts. A two-pin part dropped across one straight
    /// run replaces the run between its pins, so insertion does not short out the component.
    /// Moved free wire ends also attach to stationary pins, never to a nearby body or crossing.
    fn contacts(&mut self, before: &Project) -> Result<(), CommandError> {
        let moved: Vec<_> = self
            .project
            .symbols
            .iter()
            .filter(|s| s.terminals().iter().any(|&p| before.terminal_position(p) != self.project.terminal_position(p)))
            .cloned()
            .collect();
        for symbol in moved {
            let pins: Vec<_> = symbol.terminals().into_iter().map(WireEndpoint::Terminal).zip(symbol.pins()).collect();
            let contacts = pins.iter().map(|(_, point)| self.contact(*point, symbol.ordinal)).collect::<Result<Vec<_>, _>>()?;
            if pins.len() == 2 && pins[0].1 != pins[1].1 {
                let spans: Vec<_> = self
                    .project
                    .wires
                    .iter()
                    .filter(|w| {
                        !w.terminals().iter().any(|p| p.symbol == symbol.ordinal)
                            && self.project.wire_path(w).windows(2).any(|s| on_segment(pins[0].1, s[0], s[1]) && on_segment(pins[1].1, s[0], s[1]))
                    })
                    .map(|w| w.ordinal)
                    .collect();
                if spans.is_empty()
                    && contacts.iter().all(Option::is_some)
                    && self
                        .project
                        .wires
                        .iter()
                        .filter(|w| !w.terminals().iter().any(|p| p.symbol == symbol.ordinal))
                        .any(|w| self.project.wire_path(w).windows(2).any(|pair| overlap(pair, &[pins[0].1, pins[1].1]).is_some()))
                {
                    return Err(CommandError::InvalidParams("part insertion crosses a wire branch; separate that run first".into()));
                }
                if spans.len() > 1 {
                    return Err(CommandError::InvalidParams("part insertion meets overlapping wires; heal them first".into()));
                }
                if let Some(&wire) = spans.first() {
                    let (a, b) = self.cut_pair(wire, pins[0].1, pins[1].1)?;
                    Arc::make_mut(&mut self.project.wires).retain(|w| w.endpoints != [a, b] && w.endpoints != [b, a]);
                    self.merge(a, pins[0].0)?;
                    self.merge(b, pins[1].0)?;
                }
            }
            for (pin, point) in pins {
                if let Some(anchor) = self.contact(point, symbol.ordinal)? {
                    let other = self.resolve(anchor)?;
                    self.merge(other, pin)?;
                }
            }
        }
        let moved_ends: Vec<_> = self.project.junctions().filter(|(id, point)| before.junctions.get(id) != Some(point)).collect();
        self.connect_pins(moved_ends)?;
        Ok(())
    }

    /// Attach exactly coincident free ends to pins, including deliberate manual healing of
    /// pre-existing contacts. More than one pin at a contact is an error, never an arbitrary pick.
    fn connect_pins(&mut self, ends: Vec<(u64, Point)>) -> Result<(), CommandError> {
        for (id, point) in ends {
            let pins: Vec<_> = self
                .project
                .symbols
                .iter()
                .flat_map(|s| s.terminals().into_iter().zip(s.pins()))
                .filter(|(_, p)| *p == point)
                .map(|(pin, _)| WireEndpoint::Terminal(pin))
                .collect();
            if pins.len() > 1 {
                return Err(CommandError::InvalidParams("wire drop meets coincident component pins".into()));
            }
            if let Some(&pin) = pins.first() {
                self.merge(WireEndpoint::Junction(id), pin)?;
            }
        }
        Ok(())
    }

    /// Start above all persistent and session IDs, including IDs removed by earlier undo.
    pub fn new(project: &Project, revision: u64) -> Self {
        let highest = project
            .wires
            .last()
            .map_or(0, |w| w.ordinal)
            .max(project.junctions.last_key_value().map_or(0, |(&id, _)| id))
            .max(project.symbols.last().map_or(0, |s| s.ordinal))
            .max(revision);
        Self { project: project.clone(), next: highest.saturating_add(1), budget: 200_000, touched: BTreeSet::new() }
    }

    /// Bound pair scans independently of the number of successful splits and merges.
    fn compare(&mut self) -> Result<(), CommandError> {
        self.budget =
            self.budget.checked_sub(1).ok_or_else(|| CommandError::Unavailable("wire healing exceeds this edit's comparison budget".into()))?;
        Ok(())
    }

    /// Allocate monotonically within the native format's exact integer range.
    pub fn id(&mut self) -> Result<u64, CommandError> {
        if self.next > MAX_OBJECT_ID {
            return Err(CommandError::Unavailable("project has exhausted its object IDs".into()));
        }
        let id = self.next;
        self.next += 1;
        Ok(id)
    }

    /// Allocate an explicit free end after validating its model coordinate.
    fn free(&mut self, point: Point) -> Result<WireEndpoint, CommandError> {
        if !valid_point(&point) {
            return Err(CommandError::InvalidParams("wire point is outside schematic bounds".into()));
        }
        let id = self.id()?;
        Arc::make_mut(&mut self.project.junctions).insert(id, point);
        Ok(WireEndpoint::Junction(id))
    }

    /// Resolve explicit endpoints or split a targeted wire. Resolving a free point never uses
    /// coordinate coincidence to pick an electrical identity.
    pub fn resolve(&mut self, anchor: Anchor) -> Result<WireEndpoint, CommandError> {
        match anchor {
            Anchor::Existing(end) => {
                self.project.endpoint_position(end).map(|_| end).ok_or_else(|| CommandError::InvalidParams("wire endpoint does not exist".into()))
            }
            Anchor::Free(point) => self.free(point),
            Anchor::Split { wire, at } => self.split(wire, at),
        }
    }

    /// Add a distinct path between resolved identities. A native wire pair must be unique, so
    /// a second route is represented through an explicit corner junction before optional loop
    /// cleanup. This also lets cleanup-off preserve intentional parallel paths on save/reopen.
    pub(super) fn draw(&mut self, endpoints: [WireEndpoint; 2], path: &[Point]) -> Result<(), CommandError> {
        if let Some(existing) = self.project.wires.iter().find(|w| w.endpoints == endpoints || w.endpoints == [endpoints[1], endpoints[0]]).cloned() {
            let old = self.project.wire_path(&existing);
            if old == path || old.iter().rev().eq(path.iter()) {
                return Err(CommandError::InvalidParams("these endpoints are already joined by this route".into()));
            }
            if old.len() > 2 {
                self.split(existing.ordinal, old[1])?;
            } else if path.len() > 2 {
                let node = self.free(path[1])?;
                self.push_wire([endpoints[0], node], &path[..2])?;
                return self.push_wire([node, endpoints[1]], &path[1..]);
            } else {
                return Err(CommandError::InvalidParams("these endpoints are already joined by a wire".into()));
            }
        }
        self.push_wire(endpoints, path)
    }

    /// Allocate one authored route within the wire limit and track it for local normalization.
    fn push_wire(&mut self, endpoints: [WireEndpoint; 2], path: &[Point]) -> Result<(), CommandError> {
        if self.project.wires.len() >= MAX_WIRES {
            return Err(CommandError::Unavailable("the schematic wire limit has been reached".into()));
        }
        let ordinal = self.id()?;
        let bends = super::interior(path)?;
        Arc::make_mut(&mut self.project.wires).push(Wire { ordinal, endpoints, bends: Some(bends) });
        self.touched.insert(ordinal);
        Ok(())
    }

    /// Split at an unambiguous route point, preserving the first piece's ID. Endpoints reuse
    /// their identities; a new interior junction and second wire are allocated atomically.
    pub fn split(&mut self, id: u64, at: Point) -> Result<WireEndpoint, CommandError> {
        let index =
            self.project.wires.iter().position(|w| w.ordinal == id).ok_or_else(|| CommandError::InvalidParams("wire does not exist".into()))?;
        let wire = self.project.wires[index].clone();
        let path = self.project.wire_path(&wire);
        let (left, right) = match locate(&path, at)? {
            OnPath::Corner(0) => return Ok(wire.endpoints[0]),
            OnPath::Corner(k) if k + 1 == path.len() => return Ok(wire.endpoints[1]),
            OnPath::Corner(k) => (path[..=k].to_vec(), path[k..].to_vec()),
            OnPath::Leg(k) => (path[..=k].iter().copied().chain([at]).collect(), [at].into_iter().chain(path[k + 1..].iter().copied()).collect()),
        };
        if self.project.wires.len() >= MAX_WIRES {
            return Err(CommandError::Unavailable("the schematic wire limit has been reached".into()));
        }
        let node = self.free(at)?;
        let ordinal = self.id()?;
        let wires = Arc::make_mut(&mut self.project.wires);
        wires[index].endpoints[1] = node;
        wires[index].bends = Some(super::interior(&left)?);
        wires.push(Wire { ordinal, endpoints: [node, wire.endpoints[1]], bends: Some(super::interior(&right)?) });
        self.touched.extend([id, ordinal]);
        Ok(node)
    }

    /// Merge coincident identities, retaining a pin over a junction and otherwise the older
    /// junction. Distinct coincident component pins are refused rather than losing an identity.
    pub fn merge(&mut self, a: WireEndpoint, b: WireEndpoint) -> Result<WireEndpoint, CommandError> {
        if a == b {
            return Ok(a);
        }
        if self.project.endpoint_position(a) != self.project.endpoint_position(b) {
            return Err(CommandError::InvalidParams("wire attachment positions disagree".into()));
        }
        let (keep, remove) = match (a, b) {
            (WireEndpoint::Terminal(_), WireEndpoint::Terminal(_)) => {
                return Err(CommandError::InvalidParams("separate coincident component pins before joining wires".into()));
            }
            (WireEndpoint::Terminal(_), _) => (a, b),
            (_, WireEndpoint::Terminal(_)) => (b, a),
            _ => (a.min(b), a.max(b)),
        };
        for wire in Arc::make_mut(&mut self.project.wires) {
            if wire.endpoints.contains(&remove) {
                for end in &mut wire.endpoints {
                    if *end == remove {
                        *end = keep;
                    }
                }
                self.touched.insert(wire.ordinal);
            }
        }
        Ok(keep)
    }

    /// Resolve both boundaries of one overlap, following the newly split piece for the second.
    pub(super) fn cut_pair(&mut self, wire: u64, p: Point, q: Point) -> Result<(WireEndpoint, WireEndpoint), CommandError> {
        let existing: BTreeSet<_> = self.project.wires.iter().map(|w| w.ordinal).collect();
        let a = self.split(wire, p)?;
        let second = self
            .project
            .wires
            .iter()
            .find(|w| {
                (w.ordinal == wire || !existing.contains(&w.ordinal)) && self.project.wire_path(w).windows(2).any(|s| on_segment(q, s[0], s[1]))
            })
            .map(|w| w.ordinal)
            .ok_or_else(|| CommandError::InvalidParams("overlap boundary cannot be resolved".into()))?;
        let b = self.split(second, q)?;
        Ok((a, b))
    }

    /// Find the first positive overlap involving edited geometry in stable wire/leg order.
    fn next_overlap(&mut self) -> Result<Option<(u64, u64, Point, Point)>, CommandError> {
        let paths: Vec<_> = self.project.wires.iter().map(|w| (w.ordinal, self.project.wire_path(w))).collect();
        for (i, (a, pa)) in paths.iter().enumerate() {
            for (b, pb) in paths.iter().skip(i + 1) {
                if !self.touched.contains(a) && !self.touched.contains(b) {
                    continue;
                }
                for x in pa.windows(2) {
                    for y in pb.windows(2) {
                        self.compare()?;
                        if let Some((p, q)) = overlap(x, y) {
                            return Ok(Some((*a, *b, p, q)));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Remove identical runs, preserving the earlier ID. Other parallel edges cannot be
    /// represented by the current file format and reject the candidate without dropping paths.
    fn deduplicate(&mut self) -> Result<(), CommandError> {
        let mut seen = BTreeMap::new();
        let mut remove = BTreeSet::new();
        for wire in self.project.wires.iter() {
            let [a, b] = wire.endpoints;
            let mut path = self.project.wire_path(wire);
            simplify(&mut path);
            if path.len() < 2 && a == b {
                remove.insert(wire.ordinal);
                continue;
            }
            if a == b {
                return Err(CommandError::InvalidParams("healing would create a closed wire loop".into()));
            }
            if a > b {
                path.reverse();
            }
            if let Some(old) = seen.insert((a.min(b), a.max(b)), path.clone()) {
                if old != path {
                    return Err(CommandError::InvalidParams("healing would create different parallel routes between the same endpoints".into()));
                }
                remove.insert(wire.ordinal);
            }
        }
        Arc::make_mut(&mut self.project.wires).retain(|w| !remove.contains(&w.ordinal));
        Ok(())
    }

    /// Remove redundant straight-through degree-two junctions touched by the edit. Intentional
    /// corners and branch points remain; untouched routes and their authored/automatic state stay.
    fn collapse(&mut self) -> Result<(), CommandError> {
        for _ in 0..128 {
            let mut joined = None;
            let mut adjacent = BTreeMap::<u64, Vec<&Wire>>::new();
            for wire in self.project.wires.iter() {
                for end in wire.endpoints {
                    if let WireEndpoint::Junction(id) = end {
                        adjacent.entry(id).or_default().push(wire);
                    }
                }
            }
            for (id, edges) in adjacent {
                let node = WireEndpoint::Junction(id);
                if edges.len() != 2 || !edges.iter().any(|w| self.touched.contains(&w.ordinal)) {
                    continue;
                }
                self.budget = self
                    .budget
                    .checked_sub(1)
                    .ok_or_else(|| CommandError::Unavailable("wire healing exceeds this edit's comparison budget".into()))?;
                let (a, b) = (edges[0], edges[1]);
                let mut pa = self.project.wire_path(a);
                if a.endpoints[0] == node {
                    pa.reverse();
                }
                let mut pb = self.project.wire_path(b);
                if b.endpoints[1] == node {
                    pb.reverse();
                }
                if pa.len() < 2 || pb.len() < 2 {
                    continue;
                }
                let (x, y, z) = (pa[pa.len() - 2], pa[pa.len() - 1], pb[1]);
                if !((x.x_nm == y.x_nm && y.x_nm == z.x_nm) || (x.y_nm == y.y_nm && y.y_nm == z.y_nm)) {
                    continue;
                }
                let from = if a.endpoints[1] == node { a.endpoints[0] } else { a.endpoints[1] };
                let to = if b.endpoints[0] == node { b.endpoints[1] } else { b.endpoints[0] };
                if from == to {
                    continue;
                }
                pa.extend(pb.into_iter().skip(1));
                simplify(&mut pa);
                if !(2..=MAX_BENDS + 2).contains(&pa.len()) {
                    continue;
                }
                joined = Some((a.ordinal, b.ordinal, [from, to], pa));
                break;
            }
            let Some((a, b, ends, path)) = joined else {
                return Ok(());
            };
            for wire in Arc::make_mut(&mut self.project.wires) {
                if wire.ordinal == a {
                    wire.endpoints = ends;
                    wire.bends = Some(super::interior(&path)?);
                }
            }
            Arc::make_mut(&mut self.project.wires).retain(|w| w.ordinal != b);
            self.project.prune_junctions();
            self.touched.insert(a);
        }
        Err(CommandError::Unavailable("too many redundant junctions to heal in one edit".into()))
    }

    /// Shorten one U-shaped return touched directly or through an incident wire by sliding
    /// its existing junction along an attached
    /// straight run. Every other incident leg must remain collinear, and at least one must cover
    /// the shortcut. Never pass another corner/endpoint, move a pin, or join a separate crossing.
    /// IDs and endpoint identities stay fixed; only the participating paths and junction move.
    fn straighten_return(&mut self) -> Result<bool, CommandError> {
        let mut adjacent = BTreeMap::<u64, Vec<usize>>::new();
        for (index, wire) in self.project.wires.iter().enumerate() {
            for end in wire.endpoints {
                if let WireEndpoint::Junction(id) = end {
                    adjacent.entry(id).or_default().push(index);
                }
            }
        }
        for index in 0..self.project.wires.len() {
            let wire = self.project.wires[index].clone();
            for end in wire.endpoints {
                let WireEndpoint::Junction(id) = end else { continue };
                let Some(neighbors) = adjacent.get(&id) else { continue };
                if !neighbors.iter().any(|&i| self.touched.contains(&self.project.wires[i].ordinal)) {
                    continue;
                }
                let mut path = self.project.wire_path(&wire);
                let reversed = wire.endpoints[0] == end;
                if reversed {
                    path.reverse();
                }
                let Some(at) = return_corner(&path) else { continue };
                let old = path[path.len() - 1];
                let mut covered = false;
                let mut allowed = true;
                let mut paths = Vec::new();
                for &other in neighbors.iter().filter(|&&other| other != index) {
                    self.compare()?;
                    let neighbor = &self.project.wires[other];
                    let mut route = self.project.wire_path(neighbor);
                    let reverse = neighbor.endpoints[0] == end;
                    if reverse {
                        route.reverse();
                    }
                    let Some(&q) = route.iter().rev().nth(1) else {
                        allowed = false;
                        break;
                    };
                    if !((old.x_nm == at.x_nm && q.x_nm == at.x_nm) || (old.y_nm == at.y_nm && q.y_nm == at.y_nm)) || on_segment(q, old, at) {
                        allowed = false;
                        break;
                    }
                    covered |= on_segment(at, old, q);
                    route.pop();
                    route.push(at);
                    simplify(&mut route);
                    if reverse {
                        route.reverse();
                    }
                    paths.push((other, route));
                }
                if !allowed || !covered {
                    continue;
                }
                path.truncate(path.len() - 3);
                path.push(at);
                simplify(&mut path);
                if reversed {
                    path.reverse();
                }
                paths.push((index, path));
                for (index, path) in paths {
                    let bends = super::interior(&path)?;
                    let wire = &mut Arc::make_mut(&mut self.project.wires)[index];
                    wire.bends = Some(bends);
                    self.touched.insert(wire.ordinal);
                }
                Arc::make_mut(&mut self.project.junctions).insert(id, at);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Normalize touched routes and their positive overlaps in a bounded, deterministic candidate.
    pub fn heal(mut self) -> Result<Project, CommandError> {
        let ids: Vec<_> = self.touched.iter().copied().collect();
        for id in ids {
            let Some(index) = self.project.wires.iter().position(|w| w.ordinal == id) else {
                continue;
            };
            let original = self.project.wire_path(&self.project.wires[index]);
            let mut path = original.clone();
            simplify(&mut path);
            loops::erase_path_loops(&mut path, &mut self.budget)?;
            if path.len() < 2 {
                let [a, b] = self.project.wires[index].endpoints;
                self.merge(a, b)?;
                Arc::make_mut(&mut self.project.wires).retain(|w| w.ordinal != id);
            } else if path != original {
                Arc::make_mut(&mut self.project.wires)[index].bends = Some(super::interior(&path)?);
            }
        }
        for _ in 0..64 {
            if self.remove_redundant_loop()? {
                continue;
            }
            if self.straighten_return()? {
                continue;
            }
            let Some((a, b, p, q)) = self.next_overlap()? else {
                self.deduplicate()?;
                self.collapse()?;
                if self.straighten_return()? {
                    continue;
                }
                self.deduplicate()?;
                self.project.prune_junctions();
                self.project.connectivity = Arc::default();
                return Ok(self.project);
            };
            let (ap, aq) = self.cut_pair(a, p, q)?;
            let (bp, bq) = self.cut_pair(b, p, q)?;
            self.merge(ap, bp)?;
            self.merge(aq, bq)?;
            self.deduplicate()?;
            self.touched.extend([a, b]);
        }
        Err(CommandError::Unavailable("too many overlapping runs or return bends in one edit".into()))
    }
}

impl Project {
    /// Heal the explicitly selected wires and exact pin contacts without moving symbols or
    /// autorouting around obstacles. Missing identities and symbol selections are refused.
    pub fn healed_wires(&self, items: &crate::selection::Selection, revision: u64) -> Result<Project, CommandError> {
        if !items.symbols.is_empty() || items.wires.is_empty() {
            return Err(CommandError::InvalidParams("select one or more wires to heal".into()));
        }
        let mut edit = Edit::new(self, revision);
        for id in &items.wires {
            let wire = self.wires.iter().find(|w| w.id() == *id).ok_or_else(|| CommandError::InvalidParams("selected wire does not exist".into()))?;
            edit.touched.insert(wire.ordinal);
        }
        let ends: BTreeSet<_> = self
            .wires
            .iter()
            .filter(|w| edit.touched.contains(&w.ordinal))
            .flat_map(|w| w.endpoints)
            .filter_map(|e| if let WireEndpoint::Junction(id) = e { Some(id) } else { None })
            .collect();
        let ends = self.junctions().filter(|(id, _)| ends.contains(id)).collect();
        edit.connect_pins(ends)?;
        edit.heal()
    }

    /// Normalize only routes changed by an edit, including routes whose component pins moved.
    /// Preview and commit use the same revision for deterministic new IDs. Loading never calls it.
    /// `auto_heal = false` still connects exact contacts, but preserves route geometry.
    pub fn healed(&self, before: &Project, revision: u64, auto_heal: bool) -> Result<Project, CommandError> {
        let mut edit = Edit::new(self, revision);
        for wire in self.wires.iter() {
            if before.wires.iter().find(|w| w.ordinal == wire.ordinal).is_none_or(|old| old != wire || before.wire_path(old) != self.wire_path(wire))
            {
                edit.touched.insert(wire.ordinal);
            }
        }
        edit.contacts(before)?;
        if edit.touched.is_empty() {
            return Ok(self.clone());
        }
        let mut next = edit.finish(auto_heal)?;
        if before.symbols.iter().flat_map(|s| s.terminals()).eq(next.symbols.iter().flat_map(|s| s.terminals()))
            && before.wires.iter().map(|w| (w.ordinal, w.endpoints)).eq(next.wires.iter().map(|w| (w.ordinal, w.endpoints)))
        {
            next.connectivity = before.connectivity.clone();
        }
        Ok(next)
    }

    /// Attach a dragged junction to an explicit pin, endpoint or wire interior, then heal the
    /// touched routes as one candidate. Allocation uses the gesture's captured revision.
    /// The optional cleanup mode is shared with commands and gesture previews.
    pub fn attached(&self, junction: u64, target: Anchor, revision: u64, auto_heal: bool) -> Result<Project, CommandError> {
        if target == Anchor::Existing(WireEndpoint::Junction(junction)) {
            return Err(CommandError::InvalidParams("choose a different endpoint to attach to".into()));
        }
        if matches!(target, Anchor::Free(_)) {
            return Err(CommandError::InvalidParams("attachment requires a pin, endpoint or wire segment".into()));
        }
        let point = target.position(self).ok_or_else(|| CommandError::InvalidParams("attachment target does not exist".into()))?;
        let moved = self.reshaped(&super::Reshape::Junction(junction), point)?;
        let mut edit = Edit::new(&moved, revision);
        let end = edit.resolve(target)?;
        for wire in Arc::make_mut(&mut edit.project.wires) {
            for endpoint in &mut wire.endpoints {
                if *endpoint == WireEndpoint::Junction(junction) {
                    *endpoint = end;
                }
            }
        }
        for wire in moved.wires.iter().filter(|w| w.endpoints.contains(&WireEndpoint::Junction(junction))) {
            edit.touched.insert(wire.ordinal);
        }
        edit.finish(auto_heal)
    }

    /// Preview a symbol move together with pin contact and local wire healing.
    /// With automatic cleanup disabled, exact pin contact still connects without normalization.
    pub fn moved_symbol(&self, symbol: &str, to: Point, revision: u64, auto_heal: bool) -> Result<Project, CommandError> {
        let index = super::parse_id(symbol, "symbol-")
            .and_then(|id| self.symbol_index(id))
            .ok_or_else(|| CommandError::InvalidParams("symbol does not exist".into()))?;
        if !valid_point(&to) {
            return Err(CommandError::InvalidParams("symbol exceeds schematic bounds".into()));
        }
        if self.symbols[index].position == to {
            return Ok(self.clone());
        }
        let mut next = self.clone();
        Arc::make_mut(&mut next.symbols)[index].position = to;
        next.healed(self, revision, auto_heal)
    }
}

impl crate::Session {
    /// Apply explicit wire healing at the caller's revision as one undo step; no-op healing
    /// leaves the session revision and history unchanged.
    pub(crate) fn heal_wires(&mut self, params: &serde_json::Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let items =
            crate::selection::Selection::parse(&params["items"]).ok_or_else(|| CommandError::InvalidParams("invalid wire selection".into()))?;
        let project = self.open_project()?;
        let next = project.healed_wires(&items, self.revision())?;
        if next.wires != project.wires || next.junctions != project.junctions {
            self.change(next);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
