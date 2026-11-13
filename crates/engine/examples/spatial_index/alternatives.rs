//! Qualified, benchmark-only adapters; no external implementation is copied here.
use super::{
    bvh::{Bounds, Bvh, near},
    grid::Grid,
    hash_grid::HashGrid,
};
use geo_index::rtree::{RTree, RTreeBuilder, RTreeIndex, sort::HilbertSort};
use rstar::{AABB, RTreeObject};

/// Original envelope adapter; the library owns tree layout and query execution.
#[derive(Clone, PartialEq)]
pub struct RstarEntry {
    bounds: Bounds,
    id: usize,
}

impl RTreeObject for RstarEntry {
    type Envelope = AABB<[f64; 2]>;
    /// Return the precomputed source envelope without changing geometry precision.
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners([self.bounds[0], self.bounds[1]], [self.bounds[2], self.bounds[3]])
    }
}

/// Compare contemporary packed/bulk-loaded libraries with original prototypes and a scan.
pub enum Candidate {
    Linear,
    Layered(Vec<(u64, Vec<usize>, Candidate)>),
    Bvh(Bvh),
    Grid(Grid),
    Hash(HashGrid),
    Packed(RTree<f64>),
    Rstar(rstar::RTree<RstarEntry>),
}

impl Candidate {
    /// Construct each strategy from the same validated native envelopes.
    pub fn new(name: &str, bounds: &[Bounds]) -> Result<Self, Box<dyn std::error::Error>> {
        if bounds.is_empty() {
            return Ok(Self::Linear);
        }
        Ok(match name {
            "bvh" => Self::Bvh(Bvh::new(bounds)),
            "grid" => Self::Grid(Grid::new(bounds)),
            "hash_grid" => Self::Hash(HashGrid::new(bounds)),
            "packed_hilbert" => {
                let mut builder = RTreeBuilder::<f64>::new(u32::try_from(bounds.len())?);
                for b in bounds {
                    builder.add(b[0], b[1], b[2], b[3]);
                }
                Self::Packed(builder.finish::<HilbertSort>())
            }
            "rstar_bulk" => Self::Rstar(rstar::RTree::bulk_load(bounds.iter().enumerate().map(|(i, b)| RstarEntry { bounds: *b, id: i }).collect())),
            "linear" => Self::Linear,
            _ => return Err("unknown candidate".into()),
        })
    }

    /// Partition by exact copper-layer mask without duplicating through-via entries.
    pub fn layered(bounds: &[Bounds], masks: &[u64], strategy: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut groups = std::collections::BTreeMap::<u64, Vec<usize>>::new();
        for (i, mask) in masks.iter().enumerate() {
            groups.entry(*mask).or_default().push(i);
        }
        let mut trees = Vec::new();
        for (mask, ids) in groups {
            let local: Vec<_> = ids.iter().map(|i| bounds[*i]).collect();
            trees.push((mask, ids, Self::new(strategy, &local)?));
        }
        Ok(Self::Layered(trees))
    }

    /// Filter shared copper layers within the timed boundary and restore source ordering.
    pub fn query_layers(&self, bounds: &[Bounds], masks: &[u64], probe: Bounds, clearance: f64, layers: u64) -> Vec<usize> {
        if let Self::Layered(groups) = self {
            let mut ids = Vec::new();
            for (_, source, tree) in groups.iter().filter(|(mask, _, _)| *mask & layers != 0) {
                if matches!(tree, Self::Linear) {
                    ids.extend(source.iter().copied().filter(|i| near(probe, bounds[*i], clearance)));
                } else {
                    ids.extend(tree.query(&[], probe, clearance).into_iter().map(|i| source[i]));
                }
            }
            ids.sort_unstable();
            ids
        } else {
            let mut ids = self.query(bounds, probe, clearance);
            ids.retain(|i| masks[*i] & layers != 0);
            ids
        }
    }

    /// Include deterministic result sorting for every index; this cost cannot be omitted.
    pub fn query(&self, bounds: &[Bounds], probe: Bounds, clearance: f64) -> Vec<usize> {
        match self {
            Self::Layered(_) => self.query_layers(bounds, &[], probe, clearance, u64::MAX),
            Self::Linear => bounds.iter().enumerate().filter_map(|(i, b)| near(probe, *b, clearance).then_some(i)).collect(),
            Self::Bvh(t) => t.query(probe, clearance),
            Self::Grid(g) => g.query(probe, clearance),
            Self::Hash(g) => g.query(probe, clearance),
            Self::Packed(t) => {
                let mut ids: Vec<_> = t
                    .search(probe[0] - clearance, probe[1] - clearance, probe[2] + clearance, probe[3] + clearance)
                    .into_iter()
                    .map(|i| i as usize)
                    .collect();
                ids.sort_unstable();
                ids
            }
            Self::Rstar(t) => {
                let envelope = AABB::from_corners([probe[0] - clearance, probe[1] - clearance], [probe[2] + clearance, probe[3] + clearance]);
                let mut ids: Vec<_> = t.locate_in_envelope_intersecting(envelope).map(|v| v.id).collect();
                ids.sort_unstable();
                ids
            }
        }
    }

    /// Payload lower bounds are separate from external process allocation measurements.
    pub fn retained_bytes(&self, bounds: &[Bounds]) -> Option<usize> {
        match self {
            Self::Layered(groups) => groups.iter().try_fold(0usize, |sum, (_, ids, t)| {
                let coordinates = if matches!(t, Self::Linear) { ids.len() * std::mem::size_of::<Bounds>() } else { t.retained_bytes(&[])? };
                Some(sum + ids.capacity() * std::mem::size_of::<usize>() + coordinates)
            }),
            Self::Linear => Some(std::mem::size_of_val(bounds)),
            Self::Bvh(t) => Some(t.retained_bytes()),
            Self::Grid(g) => Some(g.retained_bytes()),
            Self::Hash(g) => Some(g.retained_bytes()),
            Self::Packed(t) => Some(t.as_ref().len()),
            Self::Rstar(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every candidate must retain exact envelope equality, degenerate bounds, fractional
    /// radii, long diagonals, coordinate extremes and deterministic source ordering.
    #[test]
    fn candidates_match_exhaustive_bounds_at_boundaries_and_after_reordering() {
        let mut bounds = Vec::new();
        for i in 0..256 {
            let x = ((i * 104729) % 1999) as f64 * 25_400. - 25_000_000.;
            let y = ((i * 13007) % 1999) as f64 * 25_400. - 25_000_000.;
            bounds.push([x - 30_000.5, y - 30_000.5, x + 30_000.5, y + 30_000.5]);
        }
        bounds.extend([[0., 0., 0., 0.], [-1e12, -1e12, 1e12, 1e12], [60_001., 0., 60_001., 0.], [1e12, 1e12, 1e12, 1e12]]);
        for _ in 0..2 {
            for name in ["linear", "bvh", "grid", "hash_grid", "packed_hilbert", "rstar_bulk"] {
                let tree = Candidate::new(name, &bounds).unwrap();
                for clearance in [0., 0.5, 1., 200_000.] {
                    for probe in bounds.iter().step_by(7) {
                        let expected: Vec<_> = bounds.iter().enumerate().filter_map(|(i, b)| near(*probe, *b, clearance).then_some(i)).collect();
                        assert_eq!(tree.query(&bounds, *probe, clearance), expected, "{name} at {clearance}");
                    }
                }
            }
            bounds.reverse();
        }
    }

    /// Empty boards need no allocation or library-specific nonempty builder precondition.
    #[test]
    fn empty_board_candidates_are_empty() {
        for name in ["linear", "bvh", "grid", "hash_grid", "packed_hilbert", "rstar_bulk"] {
            assert!(Candidate::new(name, &[]).unwrap().query(&[], [0.; 4], 0.).is_empty());
        }
    }

    /// Partitioning is applied equally to every candidate; shared all-layer objects occur once.
    #[test]
    fn layer_groups_match_global_reference_and_preserve_spanning_objects() {
        let bounds: Vec<_> = (0..256).map(|i| [i as f64, 0., i as f64 + 2., 2.]).collect();
        let masks: Vec<_> = (0..256).map(|i| if i % 7 == 0 { u64::MAX } else { 1 << (i % 64) }).collect();
        for name in ["linear", "bvh", "hash_grid", "packed_hilbert", "rstar_bulk"] {
            let grouped = Candidate::layered(&bounds, &masks, name).unwrap();
            assert_eq!(grouped.query(&bounds, [0., 0., 300., 3.], 0.), (0..256).collect::<Vec<_>>());
            for layer in [1, 2, 1 << 63, u64::MAX] {
                let expected: Vec<_> = (0..256).filter(|i| masks[*i] & layer != 0 && near([0., 0., 128., 3.], bounds[*i], 0.)).collect();
                assert_eq!(grouped.query_layers(&bounds, &masks, [0., 0., 128., 3.], 0., layer), expected, "{name}");
            }
        }
    }
}
