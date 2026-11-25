//! Prepare one all-or-nothing edit for a selection of non-geometric library updates.
use super::{Catalog, Inventory, Key, Kind, inspect_revision, latest_key};
use crate::{CommandError, Project, Session, board::assignment::component_ids, part_instances::Selection, schematic::SymbolDefinition};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::Arc,
};

/// A generic part keeps its exact dependency closure; a plain symbol replaces its drawing pin.
enum Replacement {
    Part(Selection),
    Symbol(Arc<SymbolDefinition>),
}

/// Validate the same selected roots for preview and apply. Only apply publishes the prepared
/// snapshot through the normal project transaction and undo path.
pub(in crate::library::sources) fn run(session: &mut Session, params: &Value, apply: bool) -> Result<Value, CommandError> {
    let project = session.project.as_deref().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
    session.check_revision(params)?;
    let ids = component_ids(&params["instances"]).ok_or_else(|| CommandError::InvalidParams("Select unique component IDs.".into()))?;
    let (candidate, changes) = prepare(project, session.libraries(), ids).map_err(CommandError::InvalidParams)?;
    let project_id = project.id();
    let catalog_generation = session.libraries().generation();
    let preview = format!("{:x}", Sha256::digest(json!([project_id, session.revision(), changes]).to_string()));
    if apply {
        if params["preview"] != preview {
            return Err(CommandError::InvalidParams("Library adoption no longer matches the reviewed changes; preview again.".into()));
        }
        session.change(candidate)?;
    }
    Ok(
        json!({"project":project_id,"revision":session.revision(),"catalog_generation":catalog_generation,"preview":preview,"applied":apply,"changes":changes}),
    )
}

/// Derive an immutable replacement plan before touching even a candidate snapshot. Reject the
/// entire batch if a selection cannot be adopted or if its final project pins disagree.
fn prepare(project: &Project, catalog: &Catalog, mut ids: Vec<u64>) -> Result<(Project, Vec<Value>), String> {
    let inventory = Inventory::from_project(project)?;
    let mut replacements = BTreeMap::new();
    let mut changes = Vec::new();
    // Many instances usually share a few pinned roots; discover each root once, as `inspect` does.
    let mut discoveries: BTreeMap<Key, Value> = BTreeMap::new();
    ids.sort_unstable();
    for id in ids {
        let instance = project.components.get(&id).ok_or_else(|| format!("No part instance component-{id}."))?;
        let key = if let Some(selection) = &instance.part {
            let part = &selection.resolved.definition;
            Key { library: part.library.clone(), kind: Kind::Part, id: part.id.clone(), revision: part.revision }
        } else {
            let symbol = &instance.definition;
            Key { library: symbol.library.clone(), kind: Kind::Symbol, id: symbol.id.clone(), revision: symbol.revision() }
        };
        let discovery: &Value = match discoveries.entry(key.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let pinned = inventory.item(entry.key()).ok_or("Missing pinned update root.")?;
                entry.insert(inspect_revision(&inventory, pinned, catalog))
            }
        };
        let invalid = |reason: &str| format!("{} ({}): {reason}", instance.reference(), instance.id());
        if discovery["status"] != "Update available" {
            return Err(invalid(discovery["diagnostic"].as_str().or_else(|| discovery["status"].as_str()).unwrap_or("No update available.")));
        }
        let target = latest_key(&key, catalog).ok_or("No loaded revision to adopt.")?;
        let loaded = catalog.item(target)?;
        if let Some(existing) = inventory.item(target)
            && existing.canonical != loaded.canonical
        {
            return Err(invalid("Candidate content conflicts with another project pin at that address."));
        }
        let (replacement, fields) = if let Some(selection) = &instance.part {
            let part = loaded.part().ok_or("The candidate is not a generic part.")?;
            let resolved = catalog.resolve_part(&part.reference())?;
            let old = &selection.resolved;
            let mut comparable = (*part).clone();
            comparable.name.clone_from(&old.definition.name);
            comparable.revision = old.definition.revision;
            if comparable != *old.definition
                || resolved.symbol != old.symbol
                || resolved.category != old.category
                || resolved.footprints != old.footprints
            {
                return Err(invalid("The update changes part semantics or exact dependencies; only a name change is supported."));
            }
            if selection.manufacturer.is_some() {
                return Err(invalid("The orderable selection names the old exact generic part revision; it cannot be carried forward."));
            }
            let mut next = selection.clone();
            next.resolved = Arc::new(resolved.clone());
            next.validate().map_err(|reason| invalid(&reason))?;
            (Replacement::Part(next), changed_text([("name", &old.definition.name, &part.name)]))
        } else {
            let next = loaded.symbol().ok_or("The candidate is not a symbol.")?;
            let old = &instance.definition;
            if !same_symbol_structure(old, &next) {
                return Err(invalid("The update changes symbol structure or electrical semantics; only name and default value are supported."));
            }
            let fields = changed_text([("name", &old.name, &next.name), ("default_value", &old.value, &next.value)]);
            (Replacement::Symbol(next), fields)
        };
        changes.push(json!({"part_instance":instance.id(),"reference":instance.reference(),"from":key,"to":target,
            "digest":discovery["latest_digest"],"fields":fields}));
        replacements.insert(id, replacement);
    }

    let mut candidate = project.clone();
    let components = Arc::make_mut(&mut candidate.components);
    let mut drawings = BTreeMap::new();
    for (id, replacement) in replacements {
        let component = components.get_mut(&id).ok_or("Part instance disappeared during preparation.")?;
        match replacement {
            Replacement::Part(selection) => component.part = Some(selection),
            Replacement::Symbol(symbol) => {
                component.definition = symbol.clone();
                drawings.insert(id, symbol);
            }
        }
    }
    if !drawings.is_empty() {
        for sheet in Arc::make_mut(&mut candidate.sheets) {
            if sheet.symbols.iter().any(|placement| drawings.contains_key(&placement.component)) {
                for placement in Arc::make_mut(&mut sheet.symbols) {
                    if let Some(symbol) = drawings.get(&placement.component) {
                        placement.definition = symbol.clone();
                    }
                }
            }
        }
    }
    Inventory::from_project(&candidate)?;
    Ok((candidate, changes))
}

/// Require structural and semantic equality aside from the revision, name and default value.
/// Even a default change leaves the instance's already-authored value alone.
fn same_symbol_structure(old: &SymbolDefinition, next: &SymbolDefinition) -> bool {
    old.format_version == next.format_version
        && old.provenance == next.provenance
        && old.behavior == next.behavior
        && old.defaults == next.defaults
        && old.category == next.category
        && old.reference_prefix == next.reference_prefix
        && old.pins == next.pins
        && old.units == next.units
}

/// Include only text that actually changes; exact from/to pins describe a revision-only update.
fn changed_text<const N: usize>(fields: [(&str, &String, &String); N]) -> Vec<Value> {
    fields
        .into_iter()
        .filter(|(_, before, after)| before != after)
        .map(|(field, before, after)| json!({"field":field,"before":before,"after":after}))
        .collect()
}
