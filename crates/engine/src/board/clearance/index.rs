//! Immutable local-query acceleration. Shape predicates and layer membership stay outside it.
use geo_index::rtree::{RTree, RTreeBuilder, RTreeIndex, sort::HilbertSort};

/// Small boards retain a contiguous scan; larger boards use packed Hilbert ordering.
enum Tree {
    Linear(Vec<[f64; 4]>),
    Packed(RTree<f64>),
}

impl Tree {
    /// Build from already validated finite native envelopes. The 128-object crossover is a
    /// conservative tuning choice between the measured 72- and 578-object workloads.
    fn new(bounds: Vec<[f64; 4]>) -> Self {
        let Ok(count) = u32::try_from(bounds.len()) else { return Self::Linear(bounds) };
        if count < 128 {
            return Self::Linear(bounds);
        }
        let mut builder = RTreeBuilder::<f64>::new(count);
        for b in bounds {
            builder.add(b[0], b[1], b[2], b[3]);
        }
        Self::Packed(builder.finish::<HilbertSort>())
    }

    /// Inclusive clearance-expanded lookup with stable source order, including tangency.
    /// Native coordinates/radii and the allowed clearance remain exactly representable here.
    fn candidates(&self, bounds: [f64; 4], clearance: f64) -> Vec<usize> {
        match self {
            Self::Linear(items) => items.iter().enumerate().filter_map(|(i, b)| super::near_bounds(bounds, *b, clearance).then_some(i)).collect(),
            Self::Packed(tree) => {
                let mut ids: Vec<_> = tree
                    .search(bounds[0] - clearance, bounds[1] - clearance, bounds[2] + clearance, bounds[3] + clearance)
                    .into_iter()
                    .map(|i| i as usize)
                    .collect();
                ids.sort_unstable();
                ids
            }
        }
    }
}

/// Single-layer copper has one layer-local tree. Spanning copper is stored once, avoiding a
/// 64-fold duplication of through vias; callers still filter its exact physical membership.
pub(super) struct Index {
    single: std::collections::BTreeMap<uuid::Uuid, Group>,
    spanning: Group,
}

struct Group {
    source: Vec<usize>,
    tree: Tree,
}

impl Group {
    /// Preserve source identity while grouping physical envelopes by query layer.
    fn new(entries: Vec<(usize, [f64; 4])>) -> Self {
        let (source, bounds) = entries.into_iter().unzip();
        Self { source, tree: Tree::new(bounds) }
    }

    /// Map packed positions back to the source's stable object order.
    fn append(&self, bounds: [f64; 4], clearance: f64, out: &mut Vec<usize>) {
        out.extend(self.tree.candidates(bounds, clearance).into_iter().map(|i| self.source[i]));
    }
}

impl Index {
    /// Group source data once; no object is duplicated across physical layers.
    pub(super) fn new(items: &[super::Object]) -> Self {
        let mut groups = std::collections::BTreeMap::<uuid::Uuid, Vec<(usize, [f64; 4])>>::new();
        let mut spanning = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let record = (i, item.contour.bounds());
            if let [layer] = item.layers.as_slice() {
                groups.entry(*layer).or_default().push(record);
            } else {
                spanning.push(record);
            }
        }
        Self { single: groups.into_iter().map(|(layer, entries)| (layer, Group::new(entries))).collect(), spanning: Group::new(spanning) }
    }

    /// Query only requested single-layer trees and the shared spanning tree, preserving order.
    pub(super) fn candidates(&self, bounds: [f64; 4], clearance: f64, layers: &[uuid::Uuid]) -> Vec<usize> {
        let mut ids = Vec::new();
        for layer in layers {
            if let Some(group) = self.single.get(layer) {
                group.append(bounds, clearance, &mut ids);
            }
        }
        self.spanning.append(bounds, clearance, &mut ids);
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        board::{clearance::Object, geometry::Contour},
        schematic::Point,
    };

    /// Layer-local packed branches and a shared through-via group retain every inclusive
    /// bound neighbor at 64 layers, fractional widths, long diagonals and coordinate limits.
    #[test]
    fn sixty_four_layers_match_exhaustive_bounds_without_spanning_duplication() {
        let layers: Vec<_> = (1..=64).map(uuid::Uuid::from_u128).collect();
        let items: Vec<_> = (0..14_336)
            .map(|i| {
                let x = (i % 128) as i64 * 1_270_000;
                let y = (i / 128) as i64 * 1_270_000;
                let start = Point { x_nm: x, y_nm: y };
                let end = if i % 7 == 0 { Point { x_nm: x + 80_000_000, y_nm: y - 80_000_000 } } else { start };
                Object {
                    name: format!("test-{i}"),
                    contour: Contour::segment(start, end, 60_001).unwrap(),
                    layers: if i >= 12_288 { layers.clone() } else { vec![layers[i % 64]] },
                    intended_nets: vec![i % 4000],
                    at: start,
                    island: i,
                    net: None,
                }
            })
            .collect();
        let index = Index::new(&items);
        assert_eq!(index.single.values().map(|g| g.source.len()).sum::<usize>() + index.spanning.source.len(), items.len());
        assert!(index.single.values().all(|g| matches!(g.tree, Tree::Packed(_))));
        for (n, layer) in layers.iter().enumerate() {
            let b = items[n * 61].contour.bounds();
            for clearance in [0., 1., 200_000.] {
                let mut ids = index.candidates(b, clearance, &[*layer, *layer]);
                ids.retain(|i| items[*i].layers.contains(layer));
                let expected: Vec<_> = items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, item)| {
                        (item.layers.contains(layer) && super::super::near_bounds(b, item.contour.bounds(), clearance)).then_some(i)
                    })
                    .collect();
                assert_eq!(ids, expected);
            }
        }
        for b in [[-1e12, -1e12, -1e12, -1e12], [1e12, 1e12, 1e12, 1e12]] {
            assert!(index.candidates(b, 0., &layers).is_empty());
        }
    }
}
