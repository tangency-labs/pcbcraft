//! Local geometry invalidation for two clearance queries of the same board.
//!
//! This is not a complete DRC invalidation set. It does not track board-rule or net-class edits,
//! intended net changes, or the propagation of a physical island merge or split. Callers must
//! invalidate those separately or use a full check, even when this set is empty.

use crate::{
    CommandError,
    board::{
        clearance::{ClearanceQuery, Object, ObjectKind},
        geometry::Contour,
        setup::LayerPurpose,
        view::BoardTarget,
    },
};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// One copper or mechanical geometry footprint on explicit copper layers.
#[derive(Clone, Debug)]
pub struct RegionFootprint {
    pub contour: Contour,
    pub layers: Vec<Uuid>,
}

impl RegionFootprint {
    fn from_object(object: &Object) -> Self {
        Self { contour: object.contour.clone(), layers: object.layers.clone() }
    }

    fn same_geometry(&self, other: &Self) -> bool {
        same_contour(&self.contour, &other.contour) && self.layers == other.layers
    }
}

/// How one snapshot object changed between two states of the same board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

/// One changed snapshot object, retaining both its previous and new footprints so incremental
/// checks cover vacated and newly occupied neighborhoods alike.
#[derive(Clone, Debug)]
pub struct ObjectChange {
    pub name: String,
    /// Index in the target snapshot, absent when the object was removed.
    pub after_index: Option<usize>,
    pub change: ChangeKind,
    /// Footprint before the edit, present for removed and modified objects.
    pub before: Option<RegionFootprint>,
    /// Footprint after the edit, present for added and modified objects.
    pub after: Option<RegionFootprint>,
}

/// Tracked object geometry differences between two immutable snapshots of the same board.
#[derive(Clone, Debug)]
pub struct EditDelta {
    target: BoardTarget,
    from_revision: u64,
    to_revision: u64,
    changes: Vec<ObjectChange>,
}

impl EditDelta {
    /// Compare added and removed objects, contours, layer coverage, mechanical status and via drill sizes.
    ///
    /// Only call for geometry edits under unchanged board and net-class policy and net intent.
    /// Different resolved board rules or maximum clearances are rejected; equality of those values
    /// does not establish unchanged class policy or net intent. This delta does not propagate
    /// changes to unchanged geometry on an island that merged or split.
    pub fn between_geometry_edits(before: &ClearanceQuery, after: &ClearanceQuery) -> Result<Self, CommandError> {
        let target = before.authority().target();
        if target != after.authority().target() {
            return Err(CommandError::InvalidParams("before and after queries must address the same board".into()));
        }
        if before.authority().rules() != after.authority().rules() || before.authority().clearance_nm() != after.authority().clearance_nm() {
            return Err(CommandError::InvalidParams("geometry deltas require unchanged board rules and maximum clearance".into()));
        }

        let before_objects = before.objects();
        let after_objects = after.objects();
        let before_index = index_objects(before_objects);
        let after_index = index_objects(after_objects);
        let before_vias = via_drills(before);
        let after_vias = via_drills(after);

        let mut all_keys: BTreeSet<(u8, String, usize)> = before_index.keys().cloned().collect();
        all_keys.extend(after_index.keys().cloned());

        let mut changes = Vec::new();
        for key in all_keys {
            match (before_index.get(&key).copied(), after_index.get(&key).copied()) {
                (None, Some(after_idx)) => {
                    if let Some(after_obj) = after_objects.get(after_idx) {
                        changes.push(ObjectChange {
                            name: after_obj.name.clone(),
                            after_index: Some(after_idx),
                            change: ChangeKind::Added,
                            before: None,
                            after: Some(RegionFootprint::from_object(after_obj)),
                        });
                    }
                }
                (Some(before_idx), None) => {
                    if let Some(before_obj) = before_objects.get(before_idx) {
                        changes.push(ObjectChange {
                            name: before_obj.name.clone(),
                            after_index: None,
                            change: ChangeKind::Removed,
                            before: Some(RegionFootprint::from_object(before_obj)),
                            after: None,
                        });
                    }
                }
                (Some(before_idx), Some(after_idx)) => {
                    if let (Some(before_obj), Some(after_obj)) = (before_objects.get(before_idx), after_objects.get(after_idx))
                        && object_changed(before_obj, after_obj, &before_vias, &after_vias)
                    {
                        changes.push(ObjectChange {
                            name: after_obj.name.clone(),
                            after_index: Some(after_idx),
                            change: ChangeKind::Modified,
                            before: Some(RegionFootprint::from_object(before_obj)),
                            after: Some(RegionFootprint::from_object(after_obj)),
                        });
                    }
                }
                (None, None) => {}
            }
        }

        Ok(Self { target, from_revision: before.authority().revision(), to_revision: after.authority().revision(), changes })
    }

    /// Board addressed by both snapshots in this delta.
    pub fn target(&self) -> BoardTarget {
        self.target
    }

    /// Revision of the snapshot before the edit.
    pub fn from_revision(&self) -> u64 {
        self.from_revision
    }

    /// Revision of the snapshot after the edit.
    pub fn to_revision(&self) -> u64 {
        self.to_revision
    }

    /// Changed objects ordered deterministically by kind, name, and occurrence.
    pub fn changes(&self) -> &[ObjectChange] {
        &self.changes
    }

    /// Whether no tracked object geometry changed; this does not certify unchanged DRC results.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Local geometry neighborhood under the target snapshot's largest clearance rule, not a complete DRC recheck set.
#[derive(Clone, Debug)]
pub struct DirtyRegion {
    target: BoardTarget,
    from_revision: u64,
    to_revision: u64,
    max_clearance_nm: i64,
    changed_indices: Vec<usize>,
    dirty_indices: Vec<usize>,
    dirty_names: Vec<String>,
    removed_names: Vec<String>,
}

impl DirtyRegion {
    /// Compute the local geometry neighborhood; see [`EditDelta::between_geometry_edits`] for caller obligations.
    pub fn between_geometry_edits(before: &ClearanceQuery, after: &ClearanceQuery) -> Result<Self, CommandError> {
        let delta = EditDelta::between_geometry_edits(before, after)?;
        Self::from_delta(after, &delta)
    }

    /// Expand a geometry delta using the same target query that supplied its after indices.
    /// This never expands a policy or connectivity change outside the geometric neighborhood.
    pub fn from_delta(after: &ClearanceQuery, delta: &EditDelta) -> Result<Self, CommandError> {
        let target = after.authority().target();
        if target != delta.target {
            return Err(CommandError::InvalidParams("edit delta does not match target board".into()));
        }
        if after.authority().revision() != delta.to_revision {
            return Err(CommandError::InvalidParams("edit delta revision does not match target snapshot".into()));
        }

        let objects = after.objects();
        let max_clearance_nm = after.authority().clearance_nm();
        let max_clearance = max_clearance_nm as f64;
        let live_copper: BTreeSet<Uuid> = after
            .authority()
            .view()
            .and_then(|view| view.setup())
            .map(|setup| setup.layers().iter().filter(|layer| layer.purpose == LayerPurpose::Copper).map(|layer| layer.id).collect())
            .unwrap_or_default();

        let mut changed_set = BTreeSet::new();
        let mut dirty_set = BTreeSet::new();
        let mut removed_set = BTreeSet::new();

        for change in &delta.changes {
            if let Some(after_idx) = change.after_index {
                if after_idx >= objects.len() {
                    return Err(CommandError::InvalidParams("edit delta references an object outside the target snapshot".into()));
                }
                changed_set.insert(after_idx);
                dirty_set.insert(after_idx);
            } else {
                removed_set.insert(change.name.clone());
            }

            let skip_duplicate_after = matches!((&change.before, &change.after), (Some(b), Some(a)) if b.same_geometry(a));
            let footprints = change.before.iter().chain(if skip_duplicate_after { None } else { change.after.as_ref() });
            for footprint in footprints {
                let query_layers: Vec<Uuid> = footprint.layers.iter().copied().filter(|id| live_copper.contains(id)).collect();
                if query_layers.is_empty() {
                    continue;
                }
                for idx in after.candidates(&footprint.contour, &query_layers)? {
                    if let Some(candidate) = objects.get(idx)
                        && candidate.contour.gap(&footprint.contour) <= max_clearance
                    {
                        dirty_set.insert(idx);
                    }
                }
            }
        }

        let changed_indices: Vec<usize> = changed_set.into_iter().collect();
        let dirty_indices: Vec<usize> = dirty_set.into_iter().collect();
        let dirty_names: Vec<String> = dirty_indices.iter().filter_map(|&idx| objects.get(idx).map(|obj| obj.name.clone())).collect();
        let removed_names: Vec<String> = removed_set.into_iter().collect();

        Ok(Self {
            target,
            from_revision: delta.from_revision,
            to_revision: delta.to_revision,
            max_clearance_nm,
            changed_indices,
            dirty_indices,
            dirty_names,
            removed_names,
        })
    }

    /// Board addressed by the dirty region.
    pub fn target(&self) -> BoardTarget {
        self.target
    }

    /// Revision of the snapshot before the edit.
    pub fn from_revision(&self) -> u64 {
        self.from_revision
    }

    /// Revision of the snapshot against which dirty indices are valid.
    pub fn to_revision(&self) -> u64 {
        self.to_revision
    }

    /// Largest effective clearance rule in nanometers used to expand the edit delta.
    pub fn max_clearance_nm(&self) -> i64 {
        self.max_clearance_nm
    }

    /// Sorted target-snapshot indices of added or modified objects.
    pub fn changed_indices(&self) -> &[usize] {
        &self.changed_indices
    }

    /// Sorted target-snapshot indices of every changed object and neighbor within clearance.
    pub fn dirty_indices(&self) -> &[usize] {
        &self.dirty_indices
    }

    /// Object labels corresponding to [`Self::dirty_indices`], in the same order.
    pub fn dirty_names(&self) -> &[String] {
        &self.dirty_names
    }

    /// Sorted labels of objects removed between the two snapshots.
    pub fn removed_names(&self) -> &[String] {
        &self.removed_names
    }

    /// Whether the local geometry set and removed labels are empty, not whether DRC is unchanged.
    pub fn is_empty(&self) -> bool {
        self.dirty_indices.is_empty() && self.removed_names.is_empty()
    }

    /// Whether the given target-snapshot object index belongs to the dirty set.
    pub fn contains_index(&self, index: usize) -> bool {
        self.dirty_indices.binary_search(&index).is_ok()
    }
}

/// Deterministic ordering key for snapshot object kinds.
fn kind_rank(kind: ObjectKind) -> u8 {
    match kind {
        ObjectKind::Pad => 0,
        ObjectKind::Track => 1,
        ObjectKind::Via => 2,
        ObjectKind::Other => 3,
    }
}

/// Map each snapshot object to its index using `(kind, name, ordinal)` so repeated pad names stay distinct.
fn index_objects(objects: &[Object]) -> BTreeMap<(u8, String, usize), usize> {
    let mut counts: BTreeMap<(u8, &str), usize> = BTreeMap::new();
    let mut map = BTreeMap::new();
    for (idx, object) in objects.iter().enumerate() {
        let rank = kind_rank(object.kind);
        let entry = counts.entry((rank, object.name.as_str())).or_insert(0);
        let ordinal = *entry;
        *entry += 1;
        map.insert((rank, object.name.clone(), ordinal), idx);
    }
    map
}

/// Collect per-via drill sizes keyed by via UUID string; net assignments are not geometry.
fn via_drills(query: &ClearanceQuery) -> BTreeMap<String, i64> {
    let Some(view) = query.authority().view() else {
        return BTreeMap::new();
    };
    view.vias().iter().map(|via| (via.id.to_string(), via.drill_nm)).collect()
}

fn same_contour(left: &Contour, right: &Contour) -> bool {
    left.core == right.core && left.radius == right.radius
}

/// Compare two matched objects by footprint, layers, mechanical flag, and via drill size.
fn object_changed(before: &Object, after: &Object, before_vias: &BTreeMap<String, i64>, after_vias: &BTreeMap<String, i64>) -> bool {
    if !same_contour(&before.contour, &after.contour) || before.layers != after.layers || before.mechanical != after.mechanical {
        return true;
    }
    before.kind == ObjectKind::Via && before_vias.get(&before.name) != after_vias.get(&after.name)
}

#[cfg(test)]
mod tests;
