//! Normalize recognized bundled addresses without replacing customized pinned content.
use crate::library::{
    parts::Reference,
    sources::{Kind, NativeSource, ResolvedPart, inventory::Inventory, matches_pinned},
};
use crate::{
    Project,
    board::definition::{FootprintDefinition, Key},
    schematic::SymbolDefinition,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock},
};
#[cfg(test)]
mod tests;

/// Both sides come from this build's retained immutable bundled revisions, never personal sources.
struct Bundled {
    old: NativeSource,
    current: NativeSource,
}
static BUNDLED: LazyLock<Option<Bundled>> = LazyLock::new(|| {
    let old = NativeSource::load(&super::documents()).ok()?;
    let (_, documents) = crate::library::bundled_documents().find(|(library, _)| *library == "basic-components")?;
    Some(Bundled { old, current: NativeSource::load(&documents).ok()? })
});

/// A per-project plan excludes destinations already occupied by different pinned content.
#[derive(Default)]
pub(crate) struct Replacements {
    symbols: BTreeMap<Key, Arc<SymbolDefinition>>,
    footprints: BTreeMap<Key, Arc<FootprintDefinition>>,
}
impl Replacements {
    /// Match exact historical content (allowing absent legacy attribution), including revision.
    /// A customized destination keeps both original addresses rather than silently merging them.
    fn collect(inventory: &Inventory) -> Self {
        let mut replacements = Self::default();
        for entry in inventory.entries() {
            if let Some(symbol) = entry.item.symbol() {
                if let Some(mut target) = symbol_target(&symbol) {
                    let key = crate::library::sources::Key {
                        library: target.library.clone(),
                        kind: Kind::Symbol,
                        id: target.id.clone(),
                        revision: target.revision(),
                    };
                    if let Some(existing) = inventory.item(&key).and_then(|item| item.symbol()) {
                        if !matches_pinned(&existing, &target) {
                            continue;
                        }
                        target = existing;
                    }
                    replacements.symbols.insert(symbol_key(&symbol), target);
                }
            } else if let Some(footprint) = entry.item.footprint()
                && let Some(mut target) = footprint_target(&footprint)
            {
                let key = crate::library::sources::Key {
                    library: target.library.clone(),
                    kind: Kind::Footprint,
                    id: target.id.clone(),
                    revision: target.revision,
                };
                if let Some(existing) = inventory.item(&key).and_then(|item| item.footprint()) {
                    if existing != target {
                        continue;
                    }
                    target = existing;
                }
                replacements.footprints.insert(footprint_key(&footprint), target);
            }
        }
        replacements
    }

    /// Historical logical references move only when an actual pinned symbol qualified for migration.
    pub(crate) fn symbol_library(&self, library: &str, id: &str, revision: u32) -> Option<&str> {
        self.symbols.get(&(library.into(), id.into(), revision)).map(|definition| definition.library.as_str())
    }
    /// Every physical reference uses the same planned replacement, including accepted history.
    pub(crate) fn footprint_library(&self, key: &Key) -> Option<&str> {
        self.footprints.get(key).map(|definition| definition.library.as_str())
    }

    /// Rekey only planned assets, refusing any unexpected collision before publishing the candidate.
    pub(crate) fn definitions(&self, definitions: &mut BTreeMap<Key, Arc<FootprintDefinition>>) -> Result<(), String> {
        let mut updated = BTreeMap::new();
        for (key, definition) in std::mem::take(definitions) {
            let definition = self.footprints.get(&key).cloned().unwrap_or(definition);
            let key = footprint_key(&definition);
            if updated.get(&key).is_some_and(|existing| existing != &definition) {
                return Err("Conflicting pinned footprint during library migration".into());
            }
            updated.insert(key, definition);
        }
        *definitions = updated;
        Ok(())
    }

    /// Preserve the part's fields and geometry while normalizing only proven dependency addresses.
    fn part(&self, resolved: &ResolvedPart) -> Result<ResolvedPart, String> {
        let mut result = resolved.clone();
        if let Some(symbol) = self.symbols.get(&symbol_key(&resolved.symbol)) {
            result.symbol = symbol.clone();
        }
        let definition = Arc::make_mut(&mut result.definition);
        definition.symbol = Reference { library: result.symbol.library.clone(), id: result.symbol.id.clone(), revision: result.symbol.revision() };
        for (id, footprint) in &mut result.footprints {
            if let Some(target) = self.footprints.get(&footprint_key(footprint)) {
                *footprint = target.clone();
            }
            let package = definition.packages.get_mut(id).ok_or("Missing pinned package during library migration")?;
            package.footprint = Reference { library: footprint.library.clone(), id: footprint.id.clone(), revision: footprint.revision };
            package.validate_definitions(&result.symbol, footprint)?;
        }
        definition.validate()?;
        Ok(result)
    }
}

/// Upgrade a fully validated native project in memory. A current file returns its original Arc;
/// legacy pins qualify by content, not by ID alone. Disk changes remain owned by ordinary Save.
pub(crate) fn project(mut project: Arc<Project>) -> Result<Arc<Project>, String> {
    let legacy = project.components().any(|component| {
        component.definition.library == "starter"
            || component.part.as_ref().is_some_and(|selection| selection.resolved.footprints.values().any(|footprint| footprint.library == "starter"))
    }) || project.board.definitions.keys().any(|key| key.0 == "starter")
        || project.schematic_packages.definitions.keys().any(|key| key.0 == "starter");
    if !legacy {
        return Ok(project);
    }
    let inventory = Inventory::from_project(&project)?;
    let replacements = Replacements::collect(&inventory);
    if replacements.symbols.is_empty() && replacements.footprints.is_empty() {
        return Ok(project);
    }
    let candidate = Arc::make_mut(&mut project);
    let mut parts = BTreeMap::<Reference, Arc<ResolvedPart>>::new();
    for component in Arc::make_mut(&mut candidate.components).values_mut() {
        if let Some(symbol) = replacements.symbols.get(&symbol_key(&component.definition)) {
            component.definition = symbol.clone();
        }
        if let Some(selection) = &mut component.part {
            let reference = selection.resolved.definition.reference();
            let resolved = match parts.get(&reference) {
                Some(resolved) => resolved.clone(),
                None => {
                    let resolved = Arc::new(replacements.part(&selection.resolved)?);
                    parts.insert(reference, resolved.clone());
                    resolved
                }
            };
            selection.resolved = resolved;
        }
    }
    for sheet in Arc::make_mut(&mut candidate.sheets) {
        for placement in Arc::make_mut(&mut sheet.symbols) {
            if let Some(symbol) = replacements.symbols.get(&symbol_key(&placement.definition)) {
                placement.definition = symbol.clone();
            }
        }
    }
    let choices = Arc::make_mut(&mut candidate.schematic_packages);
    for assignment in choices.assignments.values_mut() {
        if let Some(library) = replacements.footprint_library(&assignment.key()) {
            assignment.library = library.into();
        }
    }
    replacements.definitions(&mut choices.definitions)?;
    Arc::make_mut(&mut candidate.board).migrate_library_addresses(&replacements)?;
    crate::part_instances::storage::validate_project(candidate)?;
    if let Some(baseline) = &candidate.board.baseline {
        baseline.validate_assets(candidate)?;
    }
    Ok(project)
}

/// Source part resolution uses the same address normalization as project opens, so an old part
/// can still be reused after Save. Target catalog conflicts remain normal resolution failures.
pub(crate) fn resolved_part(resolved: ResolvedPart, catalog: &crate::library::sources::Catalog) -> Result<ResolvedPart, String> {
    let mut replacements = Replacements::default();
    if let Some(target) = symbol_target(&resolved.symbol) {
        let selected =
            catalog.symbol(&target.library, &target.id, target.revision()).ok_or("Migrated bundled symbol is unavailable or conflicting")?;
        if !matches_pinned(&selected, &target) {
            return Err("Migrated bundled symbol content conflicts".into());
        }
        replacements.symbols.insert(symbol_key(&resolved.symbol), selected);
    }
    for footprint in resolved.footprints.values() {
        if let Some(target) = footprint_target(footprint) {
            let selected =
                catalog.footprint(&target.library, &target.id, target.revision).ok_or("Migrated bundled footprint is unavailable or conflicting")?;
            if selected != target {
                return Err("Migrated bundled footprint content conflicts".into());
            }
            replacements.footprints.insert(footprint_key(footprint), selected);
        }
    }
    if replacements.symbols.is_empty() && replacements.footprints.is_empty() { Ok(resolved) } else { replacements.part(&resolved) }
}

/// Only retained bundled revisions with unchanged geometry and attribution can use the new ID.
fn symbol_target(symbol: &SymbolDefinition) -> Option<Arc<SymbolDefinition>> {
    if symbol.library != "starter" {
        return None;
    }
    let bundled = BUNDLED.as_ref()?;
    let original =
        bundled.old.items().iter().filter_map(|item| item.symbol()).find(|old| old.id == symbol.id && old.revision() == symbol.revision())?;
    if !matches_pinned(symbol, &original) {
        return None;
    }
    bundled.current.items().iter().filter_map(|item| item.symbol()).find(|target| target.id == symbol.id && target.revision() == symbol.revision())
}
/// Physical migration keeps the revision, pad IDs, mapping semantics and every geometry field.
fn footprint_target(footprint: &FootprintDefinition) -> Option<Arc<FootprintDefinition>> {
    if footprint.library != "starter" {
        return None;
    }
    let bundled = BUNDLED.as_ref()?;
    if !bundled.old.items().iter().filter_map(|item| item.footprint()).any(|old| old.as_ref() == footprint) {
        return None;
    }
    bundled
        .current
        .items()
        .iter()
        .filter_map(|item| item.footprint())
        .find(|target| target.id == footprint.id && target.revision == footprint.revision)
}
/// Symbol and footprint addresses share the same tuple vocabulary without sharing their kind.
fn symbol_key(symbol: &SymbolDefinition) -> Key {
    (symbol.library.clone(), symbol.id.clone(), symbol.revision())
}
/// Stable physical asset address used by choices, placements and accepted packages.
fn footprint_key(footprint: &FootprintDefinition) -> Key {
    (footprint.library.clone(), footprint.id.clone(), footprint.revision)
}
