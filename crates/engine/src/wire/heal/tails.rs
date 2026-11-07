//! Remove the dead-end side of a folded transit corner without deleting authored endpoints.

use super::Edit;
use crate::{CommandError, schematic::WireEndpoint};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

impl Edit {
    /// Prune touched leaf runs only when their endpoint was a degree-two transit junction
    /// or a temporary split point. Duplicate removal can leave these as artificial stubs.
    /// Real free ends, pins and original branches are protected, including through merges.
    /// Bound both the scan work and the number of passes; errors discard the candidate.
    pub(super) fn trim_retrace_tails(&mut self) -> Result<(), CommandError> {
        for _ in 0..128 {
            self.budget = self
                .budget
                .checked_sub(self.project.wires.len())
                .ok_or_else(|| CommandError::Unavailable("wire healing exceeds this edit's comparison budget".into()))?;
            let mut degree = BTreeMap::new();
            for end in self.project.wires.iter().flat_map(|wire| wire.endpoints) {
                *degree.entry(end).or_insert(0) += 1;
            }
            let mut remove = BTreeSet::new();
            let mut neighbors = BTreeSet::new();
            for wire in self.project.wires.iter() {
                if self.touched.contains(&wire.ordinal)
                    && wire
                        .endpoints
                        .iter()
                        .any(|end| matches!(end, WireEndpoint::Junction(_)) && !self.protected.contains(end) && degree.get(end) == Some(&1))
                {
                    remove.insert(wire.ordinal);
                    neighbors.extend(wire.endpoints);
                }
            }
            if remove.is_empty() {
                return Ok(());
            }
            for wire in self.project.wires.iter().filter(|wire| wire.endpoints.iter().any(|end| neighbors.contains(end))) {
                self.touched.insert(wire.ordinal);
            }
            Arc::make_mut(&mut self.project.wires).retain(|wire| !remove.contains(&wire.ordinal));
            self.project.prune_junctions();
        }
        Err(CommandError::Unavailable("too many folded transit corners to heal in one edit".into()))
    }
}
