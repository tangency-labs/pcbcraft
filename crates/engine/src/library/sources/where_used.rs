//! Exact, on-demand reverse lookup over the current project's authored records.
use super::{Catalog, Key, Kind, inventory::Inventory, updates};
use crate::{CommandError, Project, Session, library::parts::Reference};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

/// An exact item query, including the source comparison at the time of inspection.
#[derive(Debug, Serialize)]
pub struct Report {
    pub project: uuid::Uuid,
    pub revision: u64,
    pub catalog_generation: u64,
    pub item: Key,
    pub revision_info: Option<Value>,
    /// `None` means the loaded sources cannot establish whether this is the newest revision.
    pub newest_revision: Option<bool>,
    pub instances: Vec<Instance>,
}

/// One logical instance, regardless of unit count or matching assignment and placement.
/// A footprint retained after schematic deletion keeps the former instance ID and designator.
#[derive(Debug, Serialize)]
pub struct Instance {
    pub part_instance: String,
    pub reference: Option<String>,
    pub part: Option<Reference>,
    /// The exact queried footprint is the instance's desired package.
    pub assigned: bool,
    /// The desired package is logical intent only because the component is excluded from the board.
    /// A real retained placement, if any, is still reported separately in `board`.
    pub assignment_excluded: bool,
    pub board: Option<BoardUse>,
}

/// Board-local use. An assigned package may not have been placed yet.
#[derive(Debug, Serialize)]
pub struct BoardUse {
    pub id: uuid::Uuid,
    pub name: String,
    pub footprint: Option<String>,
    pub pending_removal: bool,
}

impl Session {
    /// Report actual uses of one exact native revision without editing the project or catalog.
    /// Both the library preview and the catalog command use this projection. An absent item
    /// has no uses; opening a project and supplying a valid immutable address are required.
    pub fn library_where_used(&self, key: &Key) -> Result<Report, CommandError> {
        let project = self.project().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        super::super::check_id(&key.library).and_then(|()| super::super::check_id(&key.id)).map_err(CommandError::InvalidParams)?;
        if key.revision == 0 {
            return Err(CommandError::InvalidParams("Definition references require a positive revision.".into()));
        }
        inspect(project, self.libraries(), self.revision(), key).map_err(CommandError::InvalidParams)
    }
}

/// Reuse the revision provider's source/conflict rules once, not once per instance.
/// Inventory defines the broader saved closure; only the live relationships below are uses.
fn inspect(project: &Project, catalog: &Catalog, revision: u64, key: &Key) -> Result<Report, String> {
    let inventory = Inventory::from_project(project)?;
    let revision_info = inventory.item(key).or_else(|| catalog.item(key).ok()).map(|item| updates::inspect_revision(&inventory, item, catalog));
    let newest_revision = match revision_info.as_ref().and_then(|info| info["status"].as_str()) {
        Some("Matches source") => Some(true),
        Some("Update available") => Some(false),
        _ => None,
    };
    Ok(Report {
        project: project.id(),
        revision,
        catalog_generation: catalog.generation(),
        item: key.clone(),
        revision_info,
        newest_revision,
        instances: instances(project, key),
    })
}

/// Walk components once and physical placements once, matching exact addresses before
/// merging by component ordinal. Do not resolve geometry, pad mappings or net snapshots.
fn instances(project: &Project, key: &Key) -> Vec<Instance> {
    let matches = |library: &str, id: &str, revision: u32| (library, id, revision) == (key.library.as_str(), key.id.as_str(), key.revision);
    let board = project.boards().next(); // This project model registers at most one physical board.
    let assignments = project.package_assignments();
    let mut uses = BTreeMap::new();
    for component in project.components() {
        let ordinal = component.ordinal;
        let assigned =
            key.kind == Kind::Footprint && assignments.get(&ordinal).is_some_and(|a| matches(&a.library, &a.definition, a.definition_revision));
        let selection = component.part.as_ref();
        let used = match key.kind {
            Kind::Footprint => assigned,
            Kind::Symbol => matches(&component.definition.library, &component.definition.id, component.definition.revision()),
            Kind::Part => selection.is_some_and(|part| {
                let item = &part.resolved.definition;
                matches(&item.library, &item.id, item.revision)
            }),
            Kind::Category => match selection {
                Some(part) => part.resolved.category.as_ref().is_some_and(|item| matches(&item.library, &item.id, item.revision)),
                None => component.definition.category.as_ref().is_some_and(|binding| {
                    let item = &binding.definition;
                    matches(&item.library, &item.id, item.revision)
                }),
            },
            Kind::ManufacturerPart => {
                selection.and_then(|part| part.manufacturer.as_ref()).is_some_and(|item| matches(&item.library, &item.id, item.revision))
            }
        };
        if used {
            let mut row = instance(project, ordinal);
            row.assigned = assigned;
            row.assignment_excluded = assigned && !project.component_is_board_eligible(ordinal);
            if assigned && !row.assignment_excluded {
                row.board =
                    board.map(|board| BoardUse { id: board.target.board, name: board.name().into(), footprint: None, pending_removal: false });
            }
            uses.insert(ordinal, row);
        }
    }
    if let Some(board) = board {
        for footprint in project.board.footprints() {
            let row = if key.kind == Kind::Footprint {
                if !footprint.library.as_deref().is_some_and(|library| matches(library, &footprint.definition, footprint.definition_revision)) {
                    continue;
                }
                uses.entry(footprint.component).or_insert_with(|| instance(project, footprint.component))
            } else if let Some(row) = uses.get_mut(&footprint.component) {
                row
            } else {
                continue;
            };
            row.board = Some(BoardUse {
                id: board.target.board,
                name: board.name().into(),
                footprint: Some(format!("footprint-{}", footprint.id)),
                pending_removal: !project.component_is_board_eligible(footprint.component),
            });
        }
    }
    uses.into_values().collect()
}

/// Use the schematic designator when present and the accepted board designator for a retained
/// placement. Do not present an accepted placement as a newly selected part or assignment.
fn instance(project: &Project, ordinal: u64) -> Instance {
    let component = project.components.get(&ordinal);
    let reference = component
        .map(|component| component.reference())
        .or_else(|| project.board.baseline.as_ref()?.components.get(&ordinal).map(|component| component.reference.as_str()));
    Instance {
        part_instance: format!("component-{ordinal}"),
        reference: reference.map(str::to_owned),
        part: component.and_then(|component| component.part.as_ref()).map(|part| part.resolved.definition.reference()),
        assigned: false,
        assignment_excluded: false,
        board: None,
    }
}
