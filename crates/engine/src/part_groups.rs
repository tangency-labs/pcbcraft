//! Project-owned part groups: named sets of part instances addressed by UUID, with explicit
//! membership stored by part-instance identity in the project's instance home.

mod commands;
#[cfg(test)]
mod tests;

use crate::Project;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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
/// projects, commands and reports already use these addresses (see docs/part-schema.md).
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
    memberships: &[Membership],
    part_instance_exists: impl Fn(PartInstanceKey) -> bool,
    group_exists: impl Fn(Uuid) -> bool,
) -> Result<(), String> {
    if memberships.len() > MAX_MEMBERSHIPS || memberships.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("Part-group memberships must be bounded, unique and sorted by part instance and group.".into());
    }
    let part_instances: BTreeSet<_> = memberships.iter().map(|membership| membership.part_instance).collect();
    let groups: BTreeSet<_> = memberships.iter().map(|membership| membership.group).collect();
    if !part_instances.into_iter().all(part_instance_exists) || !groups.into_iter().all(group_exists) {
        return Err("Part-group memberships must name existing part instances and groups.".into());
    }
    Ok(())
}

impl Project {
    /// Inspect immutable project-level part groups without reading membership.
    pub fn part_groups(&self) -> &Catalog {
        &self.part_groups
    }

    /// Every explicit part-group membership, sorted by part-instance key and then group UUID.
    pub fn part_group_memberships(&self) -> &[Membership] {
        &self.part_group_memberships
    }

    /// The groups a part instance belongs to, in UUID order. Unknown part instances belong to none.
    pub fn groups_of_part_instance(&self, part_instance: PartInstanceKey) -> impl Iterator<Item = Uuid> + '_ {
        let start = self.part_group_memberships.partition_point(|membership| membership.part_instance < part_instance);
        self.part_group_memberships[start..]
            .iter()
            .take_while(move |membership| membership.part_instance == part_instance)
            .map(|membership| membership.group)
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
    pub(crate) fn validate_part_group_memberships(&self, memberships: &[Membership]) -> Result<(), String> {
        let groups: BTreeSet<Uuid> = self.part_groups.groups.iter().map(|group| group.id).collect();
        check_memberships(memberships, |key| self.has_part_instance(key), |group| groups.contains(&group))
    }

    /// Drop memberships of part instances this snapshot no longer holds, so a deleted part
    /// instance takes its memberships with it in the same history step. Undo restores both.
    pub(crate) fn prune_part_group_memberships(&mut self) {
        if self.part_group_memberships.iter().any(|membership| !self.has_part_instance(membership.part_instance)) {
            let live: BTreeSet<_> =
                self.part_group_memberships.iter().map(|membership| membership.part_instance).filter(|key| self.has_part_instance(*key)).collect();
            std::sync::Arc::make_mut(&mut self.part_group_memberships).retain(|membership| live.contains(&membership.part_instance));
        }
    }
}
