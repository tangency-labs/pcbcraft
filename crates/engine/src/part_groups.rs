//! Project-owned part groups: named sets of part instances addressed by UUID, with explicit
//! membership stored by part-instance identity in the project's instance home.

mod commands;
#[cfg(test)]
mod tests;

use crate::Project;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

/// Create a part group with a fresh stable identity.
pub const CREATE: &str = "part_groups.create";
/// Rename a part group, retaining its identity and membership.
pub const UPDATE: &str = "part_groups.update";
/// Remove a part group that no part instance belongs to.
pub const REMOVE: &str = "part_groups.remove";
/// Inspect the part-group catalog without reading membership or connectivity.
pub const INSPECT: &str = "part_groups.inspect";
/// Replace the complete group set of one part instance.
pub const SET_GROUPS: &str = "part_instances.set_groups";
/// Inspect the group memberships of one part instance, or of every grouped part instance.
pub const INSPECT_GROUPS: &str = "part_instances.inspect_groups";
/// Saved catalog version; later property vocabularies arrive under a new version.
pub const FORMAT: &str = "pcbcraft-part-groups/1";
/// A project holds at most this many part groups, the same bound as net classes.
pub const MAX_GROUPS: usize = crate::net_classes::MAX_CLASSES;
/// Every part instance may belong to every group, and no more.
pub const MAX_MEMBERSHIPS: usize = crate::schematic::MAX_SYMBOLS * MAX_GROUPS;
/// The published address prefix of a part instance. The legacy word stays because saved
/// projects, commands and reports already use these addresses (see docs/libraries/part-schema.md).
const PART_INSTANCE_ADDRESS_PREFIX: &str = "component-";

/// One independently revisioned part group, addressed by UUID rather than display name.
/// Duplicate names are allowed because names never select membership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartGroup {
    pub id: Uuid,
    pub revision: u32,
    pub name: String,
}

impl crate::named_catalog::Entry for PartGroup {
    fn id(&self) -> Uuid {
        self.id
    }
    fn revision(&self) -> u32 {
        self.revision
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Versioned project catalog; a project with no part groups omits this record entirely.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub format: String,
    pub groups: Vec<PartGroup>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self { format: FORMAT.into(), groups: Vec::new() }
    }
}

impl Catalog {
    /// Validate the complete bounded catalog before adoption. Duplicate IDs never overwrite records.
    pub fn validate(&self) -> Result<(), String> {
        crate::named_catalog::validate(&self.format, FORMAT, "part-group", "group", &self.groups)
    }

    /// Resolve an exact group identity. Names and vector positions are never lookup authority.
    pub fn get(&self, id: Uuid) -> Option<&PartGroup> {
        crate::named_catalog::get(&self.groups, id)
    }
}

/// The saved identity of a part instance in membership records: today its numeric ID, the
/// same number its project record and placed symbols carry.
pub type PartInstanceKey = u64;

/// Read a part-instance address (as reported by project inspection) as a key. This is the only
/// conversion between addressed part instances and stored membership keys.
pub fn part_instance_key(address: &str) -> Option<PartInstanceKey> {
    crate::schematic::wiring::parse_id(address, PART_INSTANCE_ADDRESS_PREFIX)
}

/// Write the address of a stored membership key, matching [`crate::schematic::PartInstance::id`].
pub fn part_instance_address(key: PartInstanceKey) -> String {
    format!("{PART_INSTANCE_ADDRESS_PREFIX}{key}")
}

/// One explicit membership of one part instance in one group. Records sort by part instance,
/// then group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Membership {
    pub part_instance: PartInstanceKey,
    pub group: Uuid,
}

/// Explicitly empty stored lists are not a second encoding of absent membership.
pub(crate) fn stored_memberships<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<Vec<Membership>>, D::Error> {
    let memberships = Vec::<Membership>::deserialize(de)?;
    if memberships.is_empty() || memberships.len() > MAX_MEMBERSHIPS {
        return Err(serde::de::Error::custom("Part-group memberships must be omitted or nonempty and bounded."));
    }
    Ok(Some(memberships))
}

/// Check sorted order, the bound and that every membership names a known part instance and
/// group. Each distinct part instance and each distinct group is looked up once, however many
/// memberships share it; callers rely on that to keep validation linear in the membership count.
fn check_memberships(
    memberships: impl IntoIterator<Item = Membership>,
    part_instance_exists: impl Fn(PartInstanceKey) -> bool,
    group_exists: impl Fn(Uuid) -> bool,
) -> Result<(), String> {
    let (mut count, mut previous) = (0, None);
    let (mut part_instances, mut groups) = (BTreeSet::new(), BTreeSet::new());
    for membership in memberships {
        count += 1;
        if count > MAX_MEMBERSHIPS || previous.is_some_and(|previous| previous >= membership) {
            return Err("Part-group memberships must be bounded, unique and sorted by part instance and group.".into());
        }
        previous = Some(membership);
        part_instances.insert(membership.part_instance);
        groups.insert(membership.group);
    }
    if !part_instances.into_iter().all(part_instance_exists) || !groups.into_iter().all(group_exists) {
        return Err("Part-group memberships must name existing part instances and groups.".into());
    }
    Ok(())
}

/// Explicit memberships held per part instance, sorted by key. Each part instance's group set
/// (sorted, unique, never empty) is its own shared allocation, so an edit to one part instance
/// copies this index (one key and one pointer per grouped part instance) and builds only that
/// instance's set; every other set stays shared with the previous snapshot. Each Undo step
/// therefore keeps one copy of this index (at most 4,096 entries of 24 bytes) plus the edited
/// set, not a copy of every membership.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Memberships {
    by_part_instance: Vec<(PartInstanceKey, Arc<[Uuid]>)>,
}

impl Memberships {
    /// Group saved records, which the caller has checked to be sorted and unique.
    pub(crate) fn from_records(records: &[Membership]) -> Self {
        let by_part_instance = records
            .chunk_by(|left, right| left.part_instance == right.part_instance)
            .map(|chunk| (chunk[0].part_instance, chunk.iter().map(|membership| membership.group).collect()))
            .collect();
        Self { by_part_instance }
    }

    /// The saved records, sorted by part instance and then group.
    pub(crate) fn to_records(&self) -> Vec<Membership> {
        self.iter().collect()
    }

    /// Every membership, sorted by part instance and then group.
    pub fn iter(&self) -> impl Iterator<Item = Membership> + '_ {
        self.by_part_instance
            .iter()
            .flat_map(|(part_instance, groups)| groups.iter().map(|group| Membership { part_instance: *part_instance, group: *group }))
    }

    /// The number of memberships.
    pub fn len(&self) -> usize {
        self.by_part_instance.iter().map(|(_, groups)| groups.len()).sum()
    }

    /// Whether no part instance belongs to any group.
    pub fn is_empty(&self) -> bool {
        self.by_part_instance.is_empty()
    }

    /// The grouped row at a stable index for the incremental folder writer.
    pub(crate) fn grouped_row(&self, index: usize) -> Option<(PartInstanceKey, &[Uuid])> {
        self.by_part_instance.get(index).map(|(part_instance, groups)| (*part_instance, groups.as_ref()))
    }

    /// Every grouped part instance, in key order.
    pub fn part_instances(&self) -> impl Iterator<Item = PartInstanceKey> + '_ {
        self.by_part_instance.iter().map(|(part_instance, _)| *part_instance)
    }

    /// One part instance's groups in UUID order; none when it is not grouped.
    pub fn groups_of(&self, part_instance: PartInstanceKey) -> &[Uuid] {
        match self.position(part_instance) {
            Ok(index) => &self.by_part_instance[index].1,
            Err(_) => &[],
        }
    }

    /// The shared set of one part instance, if it is grouped.
    #[cfg(test)]
    fn shared_set(&self, part_instance: PartInstanceKey) -> Option<&Arc<[Uuid]>> {
        self.position(part_instance).ok().map(|index| &self.by_part_instance[index].1)
    }

    /// Find a part instance's index entry, or where it would be inserted.
    fn position(&self, part_instance: PartInstanceKey) -> Result<usize, usize> {
        self.by_part_instance.binary_search_by_key(&part_instance, |(key, _)| *key)
    }

    /// Replace one part instance's set with sorted, unique groups; an empty set removes it.
    fn replace(&mut self, part_instance: PartInstanceKey, groups: &BTreeSet<Uuid>) {
        match (self.position(part_instance), groups.is_empty()) {
            (Ok(index), true) => {
                self.by_part_instance.remove(index);
            }
            (Ok(index), false) => self.by_part_instance[index].1 = groups.iter().copied().collect(),
            (Err(_), true) => {}
            (Err(index), false) => self.by_part_instance.insert(index, (part_instance, groups.iter().copied().collect())),
        }
    }
}

impl Project {
    /// Inspect immutable project-level part groups without reading membership.
    pub fn part_groups(&self) -> &Catalog {
        &self.part_groups
    }

    /// Every explicit part-group membership, held per part instance.
    pub fn part_group_memberships(&self) -> &Memberships {
        &self.part_group_memberships
    }

    /// The groups a part instance belongs to, in UUID order. Unknown part instances belong to none.
    pub fn groups_of_part_instance(&self, part_instance: PartInstanceKey) -> impl Iterator<Item = Uuid> + '_ {
        self.part_group_memberships.groups_of(part_instance).iter().copied()
    }

    /// The project's part instances by key. The project map keeps its existing field name,
    /// which a later rename moves to the part vocabulary; this is the one place that reads it.
    fn part_instance_map(&self) -> &std::sync::Arc<std::collections::BTreeMap<PartInstanceKey, crate::schematic::PartInstance>> {
        &self.components
    }

    /// Whether this snapshot holds the part instance.
    pub(crate) fn has_part_instance(&self, key: PartInstanceKey) -> bool {
        self.part_instance_map().contains_key(&key)
    }

    /// Every part instance's address, in key order, as the part-group commands take it.
    pub fn part_instance_addresses(&self) -> impl Iterator<Item = String> + '_ {
        self.part_instance_map().keys().map(|key| part_instance_address(*key))
    }

    /// Whether both snapshots hold the same unchanged part-instance map, so no membership can
    /// have lost its part instance between them.
    pub(crate) fn shares_part_instances(&self, other: &Project) -> bool {
        std::sync::Arc::ptr_eq(self.part_instance_map(), other.part_instance_map())
    }

    /// Refuse unsorted, repeated, foreign or dangling memberships before adoption. Every
    /// membership names a live part instance and a group in this project's catalog. The
    /// catalog's identities are collected into a set once per call, so the cost grows with the
    /// number of memberships and groups, not with their product.
    pub(crate) fn validate_part_group_memberships(&self, memberships: impl IntoIterator<Item = Membership>) -> Result<(), String> {
        let groups: BTreeSet<Uuid> = self.part_groups.groups.iter().map(|group| group.id).collect();
        check_memberships(memberships, |key| self.has_part_instance(key), |group| groups.contains(&group))
    }

    /// Drop memberships of part instances this snapshot no longer holds, so a deleted part
    /// instance takes its memberships with it in the same history step. Undo restores both.
    pub(crate) fn prune_part_group_memberships(&mut self) {
        if self.part_group_memberships.part_instances().any(|key| !self.has_part_instance(key)) {
            let part_instances = Arc::clone(self.part_instance_map());
            Arc::make_mut(&mut self.part_group_memberships).by_part_instance.retain(|(key, _)| part_instances.contains_key(key));
        }
    }
}
