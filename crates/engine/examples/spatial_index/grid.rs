//! Bounded grid comparison; large envelopes use an overflow list instead of cell explosion.
use super::bvh::{Bounds, near};
use std::{collections::BTreeMap, mem::size_of};

/// Fixed one-millimeter cells with a bounded per-object cell fanout.
pub struct Grid {
    bounds: Vec<Bounds>,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
    overflow: Vec<usize>,
}

/// Expand a query before cell lookup; inclusive endpoints retain exact grid-boundary contacts.
fn cells(b: Bounds, clearance: f64) -> [i64; 4] {
    [
        (b[0] - clearance).div_euclid(1_000_000.) as i64,
        (b[1] - clearance).div_euclid(1_000_000.) as i64,
        (b[2] + clearance).div_euclid(1_000_000.) as i64,
        (b[3] + clearance).div_euclid(1_000_000.) as i64,
    ]
}

impl Grid {
    /// Keep at most 64 cell references per object; store larger bounds once in overflow.
    pub fn new(bounds: &[Bounds]) -> Self {
        let mut grid = Self { bounds: bounds.to_vec(), cells: BTreeMap::new(), overflow: Vec::new() };
        for (index, b) in bounds.iter().enumerate() {
            let c = cells(*b, 0.);
            if (c[2] - c[0] + 1).saturating_mul(c[3] - c[1] + 1) > 64 {
                grid.overflow.push(index);
            } else {
                for x in c[0]..=c[2] {
                    for y in c[1]..=c[3] {
                        grid.cells.entry((x, y)).or_default().push(index);
                    }
                }
            }
        }
        grid
    }

    /// Large queries fall back to a scan; small queries deduplicate cell and overflow indices.
    pub fn query(&self, bounds: Bounds, clearance: f64) -> Vec<usize> {
        let c = cells(bounds, clearance);
        if (c[2] - c[0] + 1).saturating_mul(c[3] - c[1] + 1) > 4096 {
            return self.bounds.iter().enumerate().filter_map(|(i, b)| near(bounds, *b, clearance).then_some(i)).collect();
        }
        let mut out = self.overflow.clone();
        for x in c[0]..=c[2] {
            for y in c[1]..=c[3] {
                if let Some(ids) = self.cells.get(&(x, y)) {
                    out.extend(ids);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out.retain(|i| near(bounds, self.bounds[*i], clearance));
        out
    }

    /// Payload capacity lower bound; BTree node allocation/metadata are intentionally excluded.
    pub fn retained_bytes(&self) -> usize {
        self.bounds.capacity() * size_of::<Bounds>()
            + self.overflow.capacity() * size_of::<usize>()
            + self.cells.values().map(|v| size_of::<(i64, i64)>() + size_of::<Vec<usize>>() + v.capacity() * size_of::<usize>()).sum::<usize>()
    }
}
