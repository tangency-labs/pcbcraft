//! Native dependency closure storage is shared by folders, archives and schematic clipboard payloads.
use super::{Record, Selection};
use crate::{
    library::{
        parts::{Category, ManufacturerPart, PartDefinition, Reference},
        sources::ResolvedPart,
    },
    persistence::Documents,
    schematic::SymbolDefinition,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Bounded immutable decode caches keep repeated instances from reparsing the same library records.
#[derive(Default)]
pub(crate) struct Reader {
    parts: BTreeMap<Reference, Arc<ResolvedPart>>,
    manufacturers: BTreeMap<Reference, Arc<ManufacturerPart>>,
}
/// Stable paths derive only from validated kind and immutable identity, never source file locations.
fn path(kind: &str, reference: &Reference) -> Result<String, String> {
    reference.validate()?;
    Ok(format!("library/{kind}/{}/{}.r{}.json", reference.library, reference.id, reference.revision))
}
/// Reject content collisions before inserting a second dependency under an existing identity.
pub(crate) fn insert(documents: &mut Documents, path: String, bytes: Vec<u8>) -> Result<(), String> {
    if bytes.len() > crate::persistence::MAX_DOCUMENT_BYTES {
        return Err("Pinned dependency exceeds its native document limit.".into());
    }
    if documents.get(&path).is_some_and(|existing| existing != &bytes) {
        return Err(format!("Conflicting pinned definition {path}."));
    }
    documents.insert(path, bytes);
    Ok(())
}
/// Write a canonical JSON dependency using the same bytes as the existing footprint encoder.
fn write<T: serde::Serialize>(documents: &mut Documents, kind: &str, reference: &Reference, value: &T) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    insert(documents, path(kind, reference)?, bytes)
}
/// Collect one part's complete offline closure, including unselected package choices and optional MPN.
pub(crate) fn collect(selection: &Selection, documents: &mut Documents) -> Result<(), String> {
    selection.validate()?;
    write(documents, "parts", &selection.resolved.definition.reference(), selection.resolved.definition.as_ref())?;
    if let Some(category) = &selection.resolved.category {
        write(documents, "categories", &category.reference(), category.as_ref())?;
    }
    for footprint in selection.resolved.footprints.values() {
        write(
            documents,
            "footprints",
            &Reference { library: footprint.library.clone(), id: footprint.id.clone(), revision: footprint.revision },
            footprint.as_ref(),
        )?;
    }
    if let Some(manufacturer) = &selection.manufacturer {
        write(documents, "manufacturer-parts", &manufacturer.reference(), manufacturer.as_ref())?;
    }
    Ok(())
}
/// Read one declared dependency, marking it used only after its path and encoding pass validation.
fn text<'a>(documents: &'a Documents, used: &mut BTreeSet<String>, kind: &str, reference: &Reference) -> Result<&'a str, String> {
    let path = path(kind, reference)?;
    let bytes = documents.get(&path).ok_or_else(|| format!("Missing pinned dependency {path}."))?;
    let text = std::str::from_utf8(bytes).map_err(|error| format!("{path}: {error}"))?;
    used.insert(path);
    Ok(text)
}
impl Reader {
    /// Rebuild exact relationships from native files. Missing, foreign or conflicting records reject the project.
    pub(crate) fn selection(
        &mut self,
        record: Record,
        symbol: Arc<SymbolDefinition>,
        documents: &Documents,
        used: &mut BTreeSet<String>,
    ) -> Result<Selection, String> {
        let resolved = if let Some(resolved) = self.parts.get(&record.definition) {
            if resolved.symbol.as_ref() != Some(&symbol) {
                return Err("One pinned part refers to inconsistent symbol content.".into());
            }
            resolved.clone()
        } else {
            let part = Arc::new(PartDefinition::read(text(documents, used, "parts", &record.definition)?)?);
            if part.reference() != record.definition {
                return Err("Pinned part identity does not match its file.".into());
            }
            let category = part
                .category
                .as_ref()
                .map(|binding| {
                    let category = Arc::new(Category::read(text(documents, used, "categories", &binding.definition)?)?);
                    binding.validate_category(&category)?;
                    Ok::<_, String>(category)
                })
                .transpose()?;
            let mut footprints = BTreeMap::new();
            for (id, package) in &part.packages {
                let footprint = crate::board::definition::read_saved(text(documents, used, "footprints", &package.footprint)?.as_bytes())?;
                package.validate_definitions(&symbol, &footprint)?;
                footprints.insert(id.clone(), footprint);
            }
            let resolved = Arc::new(ResolvedPart { definition: part, symbol: Some(symbol), category, footprints });
            self.parts.insert(record.definition, resolved.clone());
            resolved
        };
        let manufacturer = record
            .manufacturer
            .map(|reference| {
                if let Some(record) = self.manufacturers.get(&reference) {
                    return Ok(record.clone());
                }
                let value = Arc::new(ManufacturerPart::read(text(documents, used, "manufacturer-parts", &reference)?)?);
                if value.reference() != reference {
                    return Err("Pinned manufacturer identity does not match its file.".into());
                }
                value.validate_part(&resolved)?;
                self.manufacturers.insert(reference, value.clone());
                Ok::<_, String>(value)
            })
            .transpose()?;
        let selection = Selection { resolved, package: record.package, manufacturer, requirements: record.requirements };
        selection.validate()?;
        Ok(selection)
    }
}

/// Check project relationships after ordinary decoding or an isolated placement transaction.
/// Supplied package choices must agree with the part's native map and pinned footprint content.
pub(crate) fn validate_project(project: &crate::Project) -> Result<(), String> {
    let mut meanings = BTreeMap::<String, crate::library::parts::FieldType>::new();
    for part in project.components.values() {
        let Some(selection) = &part.part else {
            continue;
        };
        selection.validate()?;
        if Some(&part.definition) != selection.resolved.symbol.as_ref() {
            return Err("Part instance and pinned part symbol content differ.".into());
        }
        if let Some(value) = selection.primary_value()
            && part.value != value
        {
            return Err("Visible Value disagrees with the canonical primary requirement.".into());
        }
        if let Some(category) = &selection.resolved.category {
            for (id, field) in &category.fields {
                if let Some(previous) = meanings.insert(id.clone(), field.value_type.clone())
                    && !previous.same_meaning(&field.value_type)
                {
                    return Err(format!("Conflicting pinned meaning for category field {id}."));
                }
            }
        }
        let assigned = project.package_assignments().get(&part.ordinal);
        match &selection.package {
            Some(id) => {
                let package = selection.resolved.definition.packages.get(id).ok_or("Unknown selected package.")?;
                let assignment = assigned.ok_or("Selected part package has no pinned assignment.")?;
                if assignment.key() != (package.footprint.library.clone(), package.footprint.id.clone(), package.footprint.revision)
                    || assignment.mapping.iter().map(|(terminal, pads)| (terminal.clone(), pads.pads().to_vec())).collect::<BTreeMap<_, _>>()
                        != package.mapping
                    || assignment.unconnected_pads != package.unconnected_pads
                {
                    return Err("Part package and assigned terminal mapping disagree.".into());
                }
                let footprint = project.assignment_definition(&assignment.key()).ok_or("Part has no pinned physical definition.")?;
                if selection.resolved.footprints.get(id).is_none_or(|pinned| pinned != footprint) {
                    return Err("Part and assignment footprint content differ.".into());
                }
            }
            None if assigned.is_some() => return Err("Part without a package has an independent physical assignment.".into()),
            None => {}
        }
    }
    Ok(())
}
