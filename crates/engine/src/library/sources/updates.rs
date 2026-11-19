//! Revision discovery for existing project instances; adoption remains a separate transaction.
use super::{Catalog, Definition, Key, Kind, PartDefinition, Reference, inventory::Inventory, matches_pinned};
use crate::{CommandError, Project, board::footprint_delta, part_groups::part_instance_key};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) mod adoption;
#[cfg(test)]
mod preview_tests;
#[cfg(test)]
mod tests;

/// Inspect the latest loaded revision of each instance's update roots. Native parts use their
/// owning part, not independently replaceable dependencies. Symbol-only instances also expose
/// their standalone footprint choice. No project, history or source registration is changed.
pub(super) fn inspect(project: &Project, catalog: &Catalog, revision: u64) -> Result<Value, String> {
    let inventory = Inventory::from_project(project)?;
    let mut reports = BTreeMap::new();
    let mut instances = Vec::new();
    let mut outdated = Vec::new();
    for instance in project.components() {
        let mut keys = BTreeSet::new();
        if let Some(selection) = &instance.part {
            let part = &selection.resolved.definition;
            keys.insert(Key { library: part.library.clone(), kind: Kind::Part, id: part.id.clone(), revision: part.revision });
        } else {
            let symbol = &instance.definition;
            keys.insert(Key { library: symbol.library.clone(), kind: Kind::Symbol, id: symbol.id.clone(), revision: symbol.revision() });
            if let Some(assignment) = project.package_assignments().get(&instance.ordinal) {
                let (library, id, revision) = assignment.key();
                keys.insert(Key { library, kind: Kind::Footprint, id, revision });
            }
        }
        let mut items = Vec::new();
        for key in keys {
            let report = match reports.entry(key) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let item = inventory.item(entry.key()).ok_or("Missing pinned update root")?;
                    entry.insert(inspect_revision(&inventory, item, catalog))
                }
            };
            items.push(report.clone());
        }
        if items.iter().any(|item| item["status"] == "Update available") {
            outdated.push(instance.id());
        }
        instances.push(json!({"part_instance":instance.id(),"reference":instance.reference(),"items":items}));
    }
    Ok(json!({"project":project.id(),"revision":revision,"catalog_generation":catalog.generation(),
        "board":project.board_target().map(|target| target.board),"instances":instances,"outdated_instances":outdated}))
}

/// Compare a placed instance's pinned geometry against the same update root as discovery.
/// Native parts retain their selected package ID; a missing package is not replaced with any other.
pub(super) fn preview_footprint(project: &Project, catalog: &Catalog, revision: u64, params: &Value) -> Result<Value, CommandError> {
    let update = footprint_update(project, catalog, params)?;
    let delta = footprint_delta::compare(&project.board, update.placement, update.pinned, &update.candidate, params["pad"].as_str())?;
    let acceptance = params.get("pad").is_none().then(|| update.acceptance(project, catalog, revision, &delta));
    Ok(json!({"project": project.id(), "board": update.board, "revision": revision, "catalog_generation": catalog.generation(),
        "part_instance": update.instance.id(), "reference": update.instance.reference(), "footprint": format!("footprint-{}", update.placement.id),
        "pinned_root": update.root, "candidate_root": update.latest, "candidate_root_digest": update.digest,
        "pinned_footprint": footprint_key(update.pinned), "candidate_footprint": footprint_key(&update.candidate), "delta": delta,
        "current_mapping": update.assignment.mapping, "current_unconnected_pads": update.assignment.unconnected_pads,
        "candidate_package": update.native.and_then(|(part, package)| part.definition.packages.get(package)), "acceptance": acceptance}))
}

/// The immutable address of one exact footprint definition.
fn footprint_key(definition: &crate::board::definition::FootprintDefinition) -> Key {
    Key { library: definition.library.clone(), id: definition.id.clone(), kind: Kind::Footprint, revision: definition.revision }
}

struct FootprintUpdate<'a> {
    board: uuid::Uuid,
    instance: &'a crate::PartInstance,
    placement: &'a crate::Footprint,
    assignment: &'a crate::board::assignment::Assignment,
    pinned: &'a crate::board::definition::FootprintDefinition,
    candidate: std::sync::Arc<crate::board::definition::FootprintDefinition>,
    root: Key,
    latest: &'a Key,
    digest: Value,
    native: Option<(&'a super::ResolvedPart, &'a str)>,
}

impl FootprintUpdate<'_> {
    /// Bind review to the exact target, project revision, catalog generation and candidate content.
    fn acceptance(&self, project: &Project, catalog: &Catalog, revision: u64, delta: &footprint_delta::Delta<'_>) -> String {
        use sha2::{Digest, Sha256};
        let dependency_digest = self.native.map(|(part, _)| super::part_digest(part));
        let value = json!([
            "footprint-review",
            project.id(),
            self.board,
            revision,
            catalog.generation(),
            self.instance.ordinal,
            self.placement,
            self.assignment,
            self.root,
            self.latest,
            self.digest,
            dependency_digest,
            delta
        ]);
        format!("{:x}", Sha256::digest(value.to_string()))
    }
}

/// Resolve the current update root with the same conflict checks for preview and apply.
fn footprint_update<'a>(project: &'a Project, catalog: &'a Catalog, params: &Value) -> Result<FootprintUpdate<'a>, CommandError> {
    let invalid = |message: &str| CommandError::InvalidParams(message.into());
    let target = project.resolve_board(params["board"].as_str())?;
    let instance = params["part_instance"]
        .as_str()
        .and_then(part_instance_key)
        .and_then(|id| project.components.get(&id))
        .ok_or_else(|| CommandError::NotFound("Unknown part instance ID".into()))?;
    let placement = project
        .board
        .footprint_for_part_instance(instance.ordinal)
        .ok_or_else(|| CommandError::Unavailable("Part instance has no placed footprint".into()))?;
    let pinned =
        project.board.definition_for(placement).ok_or_else(|| CommandError::Unavailable("Placed footprint has no pinned native geometry".into()))?;
    let assignment = project
        .package_assignments()
        .get(&instance.ordinal)
        .filter(|assignment| placement.key().as_ref() == Some(&assignment.key()))
        .ok_or_else(|| invalid("Placed footprint and current package choice differ; preview needs a matching pinned choice"))?;
    let (root, package) = if let Some(selection) = &instance.part {
        let part = &selection.resolved.definition;
        let package = selection.package.as_deref().ok_or_else(|| invalid("Part instance has no selected native package"))?;
        let old = selection.resolved.footprints.get(package).ok_or_else(|| invalid("Selected package has no pinned native geometry"))?;
        if footprint_key(old) != footprint_key(pinned) {
            return Err(invalid("Placed footprint does not match the pinned native package"));
        }
        (Key { library: part.library.clone(), kind: Kind::Part, id: part.id.clone(), revision: part.revision }, Some(package))
    } else {
        (footprint_key(pinned), None)
    };
    let inventory = Inventory::from_project(project).map_err(CommandError::InvalidParams)?;
    let item = inventory.item(&root).ok_or_else(|| invalid("Missing pinned update root"))?;
    let (status, latest, digest, diagnostic) = revision_details(&inventory, item, catalog);
    let Some(latest) = latest.filter(|_| status == "Update available") else {
        let reason = diagnostic.as_deref().unwrap_or("No unambiguous newer revision is loaded");
        return Err(CommandError::Unavailable(format!("{status}: {reason}")));
    };
    let (candidate, native) = if let Some(package) = package {
        let part = catalog
            .resolve_part(&Reference { library: latest.library.clone(), id: latest.id.clone(), revision: latest.revision })
            .map_err(CommandError::Unavailable)?;
        let candidate = part.footprints.get(package).cloned().ok_or_else(|| invalid("Newer part revision does not contain the selected package"))?;
        (candidate, Some((part, package)))
    } else {
        (
            catalog
                .item(latest)
                .map_err(CommandError::Unavailable)?
                .footprint()
                .ok_or_else(|| invalid("Newer revision has no footprint geometry"))?,
            None,
        )
    };
    Ok(FootprintUpdate { board: target.board, instance, placement, assignment, pinned, candidate, root, latest, digest, native })
}

/// Present the same discovery fields for every update root without computing any geometry.
fn inspect_revision(inventory: &Inventory, pinned: &super::Item, catalog: &Catalog) -> Value {
    let (status, latest, digest, diagnostic) = revision_details(inventory, pinned, catalog);
    json!({"pinned":pinned.key,"status":status,"latest":latest,"latest_digest":digest,"diagnostic":diagnostic})
}

/// Compare the immutable pin with the highest loaded address of the same kind and identity.
/// Conflicting or unresolved latest revisions do not fall back to an older candidate.
fn revision_details<'a>(inventory: &Inventory, pinned: &super::Item, catalog: &'a Catalog) -> (&'static str, Option<&'a Key>, Value, Option<String>) {
    let mut status = inventory.status(pinned, catalog);
    let latest = if status == "Project-only" {
        None
    } else {
        let mut first = pinned.key.clone();
        first.revision = 0;
        let mut last = pinned.key.clone();
        last.revision = u32::MAX;
        catalog.index.range(first..=last).next_back().map(|(key, _)| key)
    };
    let mut diagnostics = Vec::new();
    if status == "Conflict" {
        diagnostics.push("Pinned content conflicts with its loaded immutable address.".to_owned());
    }
    let mut digest = Value::Null;
    if let Some(key) = latest {
        let candidate = catalog.item(key).and_then(|item| {
            if let Some(part) = item.part() {
                // Inspect shared pins before resolving the whole part: an unrelated missing
                // dependency must not hide a same-address conflict.
                let dependency_conflict = pinned_dependency_conflict(inventory, catalog, &part);
                if let Some(error) = &dependency_conflict {
                    status = "Conflict";
                    diagnostics.push(error.clone());
                }
                catalog.resolve_part(&part.reference())?;
                if dependency_conflict.is_some() {
                    return Ok(Value::Null);
                }
            }
            Ok(item.inspect()["digest"].clone())
        });
        match candidate {
            Ok(value) => digest = value,
            Err(error) => {
                status = if status == "Conflict" || catalog.conflicting(key) { "Conflict" } else { "Unresolved" };
                diagnostics.push(error);
            }
        }
    }
    let diagnostic = (!diagnostics.is_empty()).then(|| diagnostics.join(" "));
    (status, latest, digest, diagnostic)
}

/// Identify changed content at an exact address shared by this candidate and the saved project.
/// A new address (including a new revision), or a missing source, is not an immutable conflict.
fn pinned_dependency_conflict(inventory: &Inventory, catalog: &Catalog, part: &PartDefinition) -> Option<String> {
    part.symbol
        .iter()
        .map(|reference| (Kind::Symbol, reference))
        .chain(part.category.iter().map(|binding| (Kind::Category, &binding.definition)))
        .chain(part.packages.values().map(|package| (Kind::Footprint, &package.footprint)))
        .find_map(|(kind, reference)| {
            let key = Key { library: reference.library.clone(), kind, id: reference.id.clone(), revision: reference.revision };
            let pinned = inventory.item(&key)?;
            let conflicts = if catalog.conflicting(&key) {
                true
            } else {
                let loaded = catalog.item(&key).ok()?;
                match (&pinned.definition, &loaded.definition) {
                    (Definition::Symbol(pinned), Definition::Symbol(loaded)) => !matches_pinned(pinned, loaded),
                    _ => pinned.canonical != loaded.canonical,
                }
            };
            conflicts.then(|| format!("Pinned {kind:?} {}/{} r{} conflicts with its loaded immutable address.", key.library, key.id, key.revision))
        })
}
