//! Original median-split bounds hierarchy for the #244 candidate comparison.
use std::mem::size_of;

pub type Bounds = [f64; 4];

/// Inclusive envelopes keep touching and clearance-equality candidates.
pub fn near(a: Bounds, b: Bounds, clearance: f64) -> bool {
    a[0] <= b[2] + clearance && b[0] <= a[2] + clearance && a[1] <= b[3] + clearance && b[1] <= a[3] + clearance
}

struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}

/// Immutable balanced bounds tree; source indices survive deterministic partitioning.
pub struct Bvh {
    bounds: Vec<Bounds>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}

impl Bvh {
    /// Build a deterministic median tree, retaining each source envelope exactly once.
    pub fn new(bounds: &[Bounds]) -> Self {
        let mut tree = Self { bounds: bounds.to_vec(), order: (0..bounds.len()).collect(), nodes: Vec::new() };
        if !bounds.is_empty() {
            tree.build(0, bounds.len());
        }
        tree
    }

    /// Split on the widest envelope axis until each leaf has at most eight objects.
    fn build(&mut self, start: usize, end: usize) -> usize {
        let bounds = self.order[start..end].iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |a, i| {
            let b = self.bounds[*i];
            [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
        });
        let node = self.nodes.len();
        self.nodes.push(Node { bounds, start, end, children: None });
        if end - start > 8 {
            let axis = usize::from(bounds[3] - bounds[1] > bounds[2] - bounds[0]);
            let middle = start + (end - start) / 2;
            self.order[start..end].select_nth_unstable_by(middle - start, |a, b| {
                let center = |i: usize| self.bounds[i][axis] + self.bounds[i][axis + 2];
                center(*a).total_cmp(&center(*b)).then_with(|| a.cmp(b))
            });
            let left = self.build(start, middle);
            let right = self.build(middle, end);
            self.nodes[node].children = Some((left, right));
        }
        node
    }

    /// Return source-ordered exact envelope matches; shapes and layers stay with the caller.
    pub fn query(&self, bounds: Bounds, clearance: f64) -> Vec<usize> {
        let mut out = Vec::new();
        if !self.nodes.is_empty() {
            self.visit(0, bounds, clearance, &mut out);
        }
        out.sort_unstable();
        out
    }

    /// Prune whole subtrees, then retain inclusive source bounds at the leaves.
    fn visit(&self, index: usize, bounds: Bounds, clearance: f64, out: &mut Vec<usize>) {
        let node = &self.nodes[index];
        if !near(bounds, node.bounds, clearance) {
            return;
        }
        if let Some((left, right)) = node.children {
            self.visit(left, bounds, clearance, out);
            self.visit(right, bounds, clearance, out);
        } else {
            out.extend(self.order[node.start..node.end].iter().copied().filter(|i| near(bounds, self.bounds[*i], clearance)));
        }
    }

    /// Retained vector capacity only, excluding allocator overhead and temporary query buffers.
    pub fn retained_bytes(&self) -> usize {
        self.bounds.capacity() * size_of::<Bounds>() + self.order.capacity() * size_of::<usize>() + self.nodes.capacity() * size_of::<Node>()
    }
}
