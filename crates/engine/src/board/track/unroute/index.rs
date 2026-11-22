//! Cooperative broad phase for unroute's source-ordered contact walks.
use crate::board::work::yield_unit;
use std::collections::{BTreeMap, BTreeSet};

type Bounds = [f64; 4];

struct Node {
    bounds: Bounds,
    item: Item,
}

#[derive(Clone, Copy)]
enum Item {
    Leaf(usize),
    Branch(usize, usize),
}

/// Bottom-up bounds hierarchy over spatially ordered contours. Queries prune disjoint
/// subtrees, but emit increasing source IDs to retain the planner's path tie breaks.
pub(super) struct Index {
    nodes: Vec<Node>,
}

/// Spread a 32-bit coordinate into the even bits of the Morton key.
fn spread(value: u32) -> u64 {
    let mut bits = u64::from(value);
    bits = (bits | (bits << 16)) & 0x0000_ffff_0000_ffff;
    bits = (bits | (bits << 8)) & 0x00ff_00ff_00ff_00ff;
    bits = (bits | (bits << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    bits = (bits | (bits << 2)) & 0x3333_3333_3333_3333;
    (bits | (bits << 1)) & 0x5555_5555_5555_5555
}

/// Morton order is only a partition heuristic; exact envelopes retain fractional bounds.
fn spatial_key([left, low, right, high]: Bounds) -> u64 {
    let axis = |a: f64, b: f64| (a.midpoint(b) as i32 as u32) ^ (1 << 31);
    spread(axis(left, right)) | (spread(axis(low, high)) << 1)
}

impl Index {
    /// Order one envelope and assemble one tree node per unit of work.
    pub(super) async fn new(bounds: &[Bounds]) -> Self {
        let mut ordered = BTreeMap::new();
        for (id, &bounds) in bounds.iter().enumerate() {
            yield_unit().await;
            ordered.insert((spatial_key(bounds), id), bounds);
        }
        let mut nodes = Vec::new();
        let mut level = Vec::new();
        for ((_, id), bounds) in ordered {
            yield_unit().await;
            level.push(nodes.len());
            nodes.push(Node { bounds, item: Item::Leaf(id) });
        }
        while level.len() > 1 {
            let mut next = Vec::new();
            for pair in level.chunks(2) {
                yield_unit().await;
                if let &[a, b] = pair {
                    let [al, at, ar, ab] = nodes[a].bounds;
                    let [bl, bt, br, bb] = nodes[b].bounds;
                    next.push(nodes.len());
                    nodes.push(Node { bounds: [al.min(bl), at.min(bt), ar.max(br), ab.max(bb)], item: Item::Branch(a, b) });
                } else {
                    next.push(pair[0]);
                }
            }
            level = next;
        }
        Self { nodes }
    }

    /// Visit at most one envelope per suspension; inclusive comparisons keep tangencies.
    pub(super) async fn query(&self, [left, low, right, high]: Bounds) -> BTreeSet<usize> {
        let mut stack: Vec<_> = self.nodes.len().checked_sub(1).into_iter().collect();
        let mut hits = BTreeSet::new();
        while let Some(node) = stack.pop() {
            yield_unit().await;
            let [l, t, r, b] = self.nodes[node].bounds;
            if left <= r && l <= right && low <= b && t <= high {
                match self.nodes[node].item {
                    Item::Leaf(id) => {
                        hits.insert(id);
                    }
                    Item::Branch(a, b) => stack.extend([b, a]),
                }
            }
        }
        hits
    }
}
