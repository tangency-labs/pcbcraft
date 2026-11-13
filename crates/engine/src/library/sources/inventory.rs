//! Read-only inventory of the exact native dependencies carried by one project.
use super::{Catalog, Definition, Item, Key, Kind, ResolvedPart, matches_pinned};
use crate::{Project, board::definition::FootprintDefinition, library::parts::Reference};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[cfg(test)]
mod tests;

/// One row per immutable address, with logical references deduplicated across units and uses.
#[derive(Clone, Debug)]
pub struct Entry {
    pub item: Item,
    pub used_by: BTreeSet<String>,
}

/// A disposable projection; it never loads sources or changes the project's pinned definitions.
#[derive(Debug, Default)]
pub struct Inventory {
    entries: BTreeMap<Key, Entry>,
    parts: BTreeMap<Reference, Arc<ResolvedPart>>,
    names: BTreeMap<String, String>,
}

impl Inventory {
    /// Collect the same dependency closure as native Save, including unselected packages and
    /// accepted board history. Unreferenced definition-cache entries are deliberately excluded.
    pub fn from_project(project: &Project) -> Result<Self, String> {
        let mut inventory = Self::default();
        for component in project.components() {
            let used_by = component.reference();
            inventory.insert(Definition::Symbol(component.definition.clone()), used_by)?;
            if let Some(selection) = &component.part {
                let resolved = &selection.resolved;
                inventory.parts.insert(resolved.definition.reference(), resolved.clone());
                inventory.insert(Definition::Part(resolved.definition.clone()), used_by)?;
                if let Some(category) = &resolved.category {
                    inventory.insert(Definition::Category(category.clone()), used_by)?;
                }
                for footprint in resolved.footprints.values() {
                    inventory.insert(Definition::Footprint(footprint.clone()), used_by)?;
                }
                if let Some(manufacturer) = &selection.manufacturer {
                    inventory.insert(Definition::ManufacturerPart(manufacturer.clone()), used_by)?;
                }
            }
        }
        for (assignments, definitions) in [
            (&project.schematic_packages.assignments, &project.schematic_packages.definitions),
            (&project.board.assignments, &project.board.definitions),
        ] {
            for assignment in assignments.values() {
                let definition = definitions.get(&assignment.key()).ok_or("Missing pinned package definition")?;
                let reference = project.components.get(&assignment.component).map(|component| component.reference()).unwrap_or("Board");
                inventory.insert(Definition::Footprint(definition.clone()), reference)?;
            }
        }
        for footprint in project.board.footprints() {
            if let Some(key) = footprint.key() {
                let definition = project.board.definitions.get(&key).ok_or("Missing pinned board footprint")?;
                let reference = project.components.get(&footprint.component).map(|component| component.reference()).unwrap_or("Retained on board");
                inventory.insert(Definition::Footprint(definition.clone()), reference)?;
            }
        }
        if let Some(baseline) = &project.board.baseline {
            for key in baseline.package_keys() {
                let definition = project.board.definitions.get(&key).ok_or("Missing accepted board package")?;
                inventory.insert(Definition::Footprint(definition.clone()), "Accepted board history")?;
            }
        }
        Ok(inventory)
    }

    /// Iterate exact revisions in stable address order, without collapsing older project pins.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// Resolve only this project's saved record, even if a registered source conflicts or is offline.
    pub fn item(&self, key: &Key) -> Option<&Item> {
        self.entries.get(key).map(|entry| &entry.item)
    }

    /// Saved collection names are presentation metadata; missing sources never imply Project-only.
    pub fn library_name<'a>(&'a self, library: &'a str) -> &'a str {
        self.names.get(library).map(String::as_str).unwrap_or(library)
    }

    /// Use the exact dependency graph captured with the placed generic part.
    pub fn resolve_part(&self, reference: &Reference) -> Result<&ResolvedPart, String> {
        self.parts.get(reference).map(Arc::as_ref).ok_or_else(|| "Generic part is not pinned in this project".into())
    }

    /// Resolve package geometry from the project's closure rather than from current source content.
    pub fn footprint(&self, reference: &Reference) -> Option<Arc<FootprintDefinition>> {
        self.item(&Key { library: reference.library.clone(), kind: Kind::Footprint, id: reference.id.clone(), revision: reference.revision })?
            .footprint()
    }

    /// Distinguish deliberate project variants, source absence and same-address content conflicts.
    /// Newer source revisions are information only; this method never adopts an update.
    pub fn status(&self, item: &Item, catalog: &Catalog) -> &'static str {
        if item.key.library.starts_with("project-") {
            return "Project-only";
        }
        if catalog.conflicting(&item.key) {
            return "Conflict";
        }
        let candidates = catalog.index.values().map(|(source, index)| &catalog.sources[source].items[*index]);
        let mut highest = 0;
        let mut exact = None;
        for candidate in candidates
            .filter(|candidate| candidate.key.library == item.key.library && candidate.key.kind == item.key.kind && candidate.key.id == item.key.id)
        {
            highest = highest.max(candidate.key.revision);
            if candidate.key == item.key {
                exact = Some(candidate);
            }
        }
        if let Some(candidate) = exact {
            let matches = match (&item.definition, &candidate.definition) {
                (Definition::Symbol(pinned), Definition::Symbol(source)) => matches_pinned(pinned, source),
                _ => item.canonical == candidate.canonical,
            };
            if !matches {
                return "Conflict";
            }
        }
        if highest > item.key.revision {
            "Update available"
        } else if exact.is_some() {
            "Matches source"
        } else {
            "Source unavailable"
        }
    }

    /// Canonicalize each unique address once; reject ambiguous pins instead of picking a winner.
    fn insert(&mut self, definition: Definition, used_by: &str) -> Result<(), String> {
        let key = match &definition {
            Definition::Symbol(value) => Key { library: value.library.clone(), kind: Kind::Symbol, id: value.id.clone(), revision: value.revision() },
            Definition::Footprint(value) => {
                Key { library: value.library.clone(), kind: Kind::Footprint, id: value.id.clone(), revision: value.revision }
            }
            Definition::Part(value) => Key { library: value.library.clone(), kind: Kind::Part, id: value.id.clone(), revision: value.revision },
            Definition::Category(value) => {
                Key { library: value.library.clone(), kind: Kind::Category, id: value.id.clone(), revision: value.revision }
            }
            Definition::ManufacturerPart(value) => {
                Key { library: value.library.clone(), kind: Kind::ManufacturerPart, id: value.id.clone(), revision: value.revision }
            }
        };
        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.item.definition != definition {
                return Err("Conflicting definitions in project inventory".into());
            }
            entry.used_by.insert(used_by.into());
            return Ok(());
        }
        let (name, license, canonical) = match &definition {
            Definition::Symbol(value) => {
                if let Some(provenance) = &value.provenance {
                    self.names.entry(value.library.clone()).or_insert_with(|| provenance.library_name.clone());
                }
                (
                    &value.name,
                    value.provenance.as_ref().map(|p| p.license.clone()).unwrap_or_default(),
                    serde_json::to_value(super::saved_copy(value)),
                )
            }
            Definition::Footprint(value) => (&value.name, value.license.clone(), serde_json::to_value(value.as_ref())),
            Definition::Part(value) => (&value.name, value.license.clone(), serde_json::to_value(value.as_ref())),
            Definition::Category(value) => (&value.name, value.license.clone(), serde_json::to_value(value.as_ref())),
            Definition::ManufacturerPart(value) => (&value.name, value.license.clone(), serde_json::to_value(value.as_ref())),
        };
        let canonical = canonical.map_err(|error| error.to_string())?;
        let item = Item { key: key.clone(), name: name.clone(), license, file: String::new(), definition, canonical };
        self.entries.insert(key, Entry { item, used_by: BTreeSet::from([used_by.into()]) });
        Ok(())
    }
}
