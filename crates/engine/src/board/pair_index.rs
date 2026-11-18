//! Resumable broad phase for board-wide contact and clearance checks.
use super::geometry::Contour;
use std::ops::Range;

/// Closed projection intervals on x, y, x+y and x−y. Diagonal axes stop long parallel tracks
/// from becoming candidates merely because they cross the same axis-aligned rectangle.
#[derive(Clone, Copy)]
struct Bounds {
    min: [f64; 4],
    max: [f64; 4],
}

/// Expand the unnormalized diagonal projection conservatively. Integer native cores project
/// exactly, and the extra nanometer rounds outward (including at floating-point tangencies).
fn diagonal_margin(distance: f64) -> f64 {
    distance * std::f64::consts::SQRT_2 + 1.
}

impl Bounds {
    /// Enclose the integer core and its round expansion in all four directions.
    fn new(contour: &Contour) -> Self {
        let xy = contour.bounds();
        let plus = contour.core.map(|p| p.x_nm + p.y_nm);
        let minus = contour.core.map(|p| p.x_nm - p.y_nm);
        let r = diagonal_margin(contour.radius);
        Self {
            min: [xy[0], xy[1], *plus.iter().min().unwrap() as f64 - r, *minus.iter().min().unwrap() as f64 - r],
            max: [xy[2], xy[3], *plus.iter().max().unwrap() as f64 + r, *minus.iter().max().unwrap() as f64 + r],
        }
    }

    /// Enclose both child bounds.
    fn union(self, other: Self) -> Self {
        Self { min: std::array::from_fn(|a| self.min[a].min(other.min[a])), max: std::array::from_fn(|a| self.max[a].max(other.max[a])) }
    }

    /// Reject only disjoint projections. Shape distance still decides strict clearance and
    /// contact, including equality at zero; these intervals may retain false positives.
    fn near(self, other: Self, margin: [f64; 4]) -> bool {
        (0..4).all(|a| self.min[a] <= other.max[a] + margin[a] && other.min[a] <= self.max[a] + margin[a])
    }

    /// Twice the center; selection depends only on relative projected center positions.
    fn center(self, axis: usize) -> f64 {
        self.min[axis] + self.max[axis]
    }
}

struct Node {
    bounds: Bounds,
    min_item: usize,
    contents: Contents,
}

enum Contents {
    Children(usize, usize),
    Leaf(Range<usize>),
}

/// Immutable hierarchy over source-ordered contours. Node envelopes include every descendant.
/// Building uses median spatial partitions, O(n log n) work and O(n) storage; no pairs are stored.
/// Queries can still encounter quadratic true overlap across the whole board.
pub(super) struct PairIndex {
    bounds: Vec<Bounds>,
    order: Vec<usize>,
    nodes: Vec<Node>,
    root: Option<usize>,
}

/// One resumable DFS. The stack stores at most one sibling per tree level.
#[derive(Default)]
pub(super) struct Search {
    stack: Vec<usize>,
    leaf: Range<usize>,
    margin: [f64; 4],
}

pub(super) enum Hit {
    /// One branch or leaf record was visited without producing a pair.
    Pending,
    Candidate(usize),
    Done,
}

impl PairIndex {
    /// Build one local snapshot index. The current checks do not mutate indexed shapes.
    pub(super) fn new<'a>(contours: impl Iterator<Item = &'a Contour>) -> Self {
        let bounds: Vec<_> = contours.map(Bounds::new).collect();
        let mut order: Vec<_> = (0..bounds.len()).collect();
        let mut nodes = Vec::new();
        let root = (!order.is_empty()).then(|| Self::build(&bounds, &mut order, 0, &mut nodes));
        Self { bounds, order, nodes, root }
    }

    /// Split on the widest physical center spread. Normalize the diagonal spread when comparing
    /// axes; the source ID is a deterministic tiebreaker for coincident centers.
    fn build(bounds: &[Bounds], order: &mut [usize], offset: usize, nodes: &mut Vec<Node>) -> usize {
        let mut envelope = bounds[order[0]];
        let mut min_item = order[0];
        let mut lo = std::array::from_fn::<_, 4, _>(|axis| envelope.center(axis));
        let mut hi = lo;
        for &item in &order[1..] {
            let b = bounds[item];
            envelope = envelope.union(b);
            min_item = min_item.min(item);
            for axis in 0..4 {
                let center = b.center(axis);
                lo[axis] = lo[axis].min(center);
                hi[axis] = hi[axis].max(center);
            }
        }
        let contents = if order.len() <= 8 {
            Contents::Leaf(offset..offset + order.len())
        } else {
            let spread = |axis: usize| (hi[axis] - lo[axis]) * if axis < 2 { 1. } else { std::f64::consts::FRAC_1_SQRT_2 };
            let axis = (0..4).max_by(|&a, &b| spread(a).total_cmp(&spread(b))).unwrap();
            let pivot = order.len() / 2;
            order.select_nth_unstable_by(pivot, |&a, &b| bounds[a].center(axis).total_cmp(&bounds[b].center(axis)).then(a.cmp(&b)));
            let (left, right) = order.split_at_mut(pivot);
            Contents::Children(Self::build(bounds, left, offset, nodes), Self::build(bounds, right, offset + pivot, nodes))
        };
        let id = nodes.len();
        nodes.push(Node { bounds: envelope, min_item, contents });
        id
    }

    /// Reuse the cursor's bounded stack between source items.
    pub(super) fn begin(&self, search: &mut Search, clearance: f64) {
        search.stack.clear();
        search.stack.extend(self.root);
        search.leaf = 0..0;
        let diagonal = diagonal_margin(clearance);
        search.margin = [clearance, clearance, diagonal, diagonal];
    }

    /// Visit one branch or one leaf entry. Only lower-numbered neighbors are emitted, so even
    /// though spatial order differs from source order every unordered pair is considered once.
    pub(super) fn next(&self, source: usize, search: &mut Search) -> Hit {
        let target = self.bounds[source];
        if let Some(position) = search.leaf.next() {
            let candidate = self.order[position];
            return if candidate < source && target.near(self.bounds[candidate], search.margin) { Hit::Candidate(candidate) } else { Hit::Pending };
        }
        let Some(node) = search.stack.pop().map(|id| &self.nodes[id]) else { return Hit::Done };
        if node.min_item < source && target.near(node.bounds, search.margin) {
            match &node.contents {
                Contents::Children(left, right) => search.stack.extend([*right, *left]),
                Contents::Leaf(range) => search.leaf = range.clone(),
            }
        }
        Hit::Pending
    }

    /// Number of source items; empty indexes do not start a lookup.
    fn len(&self) -> usize {
        self.bounds.len()
    }
}

/// Shared board-wide clearance cursor, advancing one index operation per call.
pub(super) struct PairWalk {
    index: PairIndex,
    search: Search,
    source: usize,
    clearance: f64,
}

pub(super) enum PairStep {
    Pending,
    Pair(usize, usize),
    Done,
}

impl PairWalk {
    /// Prepare the index once per clearance pass, including the maximum applicable rule.
    pub(super) fn new<'a>(contours: impl Iterator<Item = &'a Contour>, clearance: i64) -> Self {
        let index = PairIndex::new(contours);
        let mut search = Search::default();
        let clearance = clearance as f64;
        index.begin(&mut search, clearance);
        Self { index, search, source: 0, clearance }
    }

    /// Yield after every search operation, whether or not it produced a candidate.
    pub(super) fn step(&mut self) -> PairStep {
        if self.source >= self.index.len() {
            return PairStep::Done;
        }
        match self.index.next(self.source, &mut self.search) {
            Hit::Pending => PairStep::Pending,
            Hit::Candidate(lower) => PairStep::Pair(lower, self.source),
            Hit::Done => {
                self.source += 1;
                self.index.begin(&mut self.search, self.clearance);
                PairStep::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::{MAX_CENTER_NM, Point};
    use std::collections::BTreeSet;

    /// Compare with exact shape distances across tree branches, both diagonal orientations,
    /// half-nanometer radii and contacts/equality near the valid coordinate extremes.
    #[test]
    fn indexed_pairs_keep_every_close_shape_once() {
        let mut contours = Vec::new();
        for sign in [-1, 1] {
            let x = sign * (MAX_CENTER_NM - 3_000_000);
            for i in 0..64_i64 {
                let y = sign * (MAX_CENTER_NM - 3_000_000) + i * 20_000;
                let start = Point { x_nm: x, y_nm: y };
                let end = if i % 5 == 0 { start } else { Point { x_nm: x + 1_000_000, y_nm: y - sign * 1_000_000 } };
                contours.push(Contour::capsule(start, end, if i % 4 == 0 { 20_000. } else { 5000.5 }));
            }
        }
        for clearance in [0, 1, 9999, 200_000] {
            let mut cursor = PairWalk::new(contours.iter(), clearance);
            let mut seen = BTreeSet::new();
            loop {
                match cursor.step() {
                    PairStep::Pending => (),
                    PairStep::Done => break,
                    PairStep::Pair(a, b) => assert!(a < b && seen.insert((a, b))),
                }
            }
            let mut close = 0;
            for (b, right) in contours.iter().enumerate() {
                for (a, left) in contours[..b].iter().enumerate() {
                    if left.gap(right) <= clearance as f64 {
                        assert!(seen.contains(&(a, b)), "missing {a}/{b} at {clearance} nm");
                        close += 1;
                    }
                }
            }
            assert!(close > 0);
        }
    }
}
