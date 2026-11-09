//! Component package choices are independent of board coordinates and placement IDs.
use super::{Footprint, definition};
use crate::{CommandError, Project, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// Pin a physical package and explicit logical mapping without creating a placement.
pub const ASSIGN_FOOTPRINT: &str = "pcb.assign_footprint";
/// Place a component's previously assigned package using its exact pinned definition.
pub const PLACE_ASSIGNED_FOOTPRINT: &str = "pcb.place_assigned_footprint";

/// Persist component identity and immutable package identity separately from board geometry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Assignment {
    pub component: u64,
    pub library: String,
    pub definition: String,
    pub definition_revision: u32,
    pub mapping: BTreeMap<String, String>,
}

impl Assignment {
    /// Address the exact pinned definition, never a current-library substitute.
    pub(crate) fn key(&self) -> definition::Key {
        (self.library.clone(), self.definition.clone(), self.definition_revision)
    }

    /// Construct placement vocabulary for the existing shared geometry/mapping validator.
    pub(crate) fn footprint(&self, id: u64, position: Point) -> Footprint {
        Footprint {
            id,
            component: self.component,
            library: Some(self.library.clone()),
            definition: self.definition.clone(),
            definition_revision: self.definition_revision,
            mapping: Some(self.mapping.clone()),
            pin_1_pad: 0,
            x_nm: position.x_nm,
            y_nm: position.y_nm,
            rotation: 0,
        }
    }

    /// Resolve the component and its exact definition before accepting an offline record.
    pub(crate) fn valid(&self, project: &Project) -> bool {
        let footprint = self.footprint(1, Point { x_nm: 0, y_nm: 0 });
        let logical = project.components.get(&self.component).map_or_else(
            || project.board.baseline.as_ref().is_some_and(|baseline| baseline.validate_assignment(self)),
            |component| project.board.mapping_valid(&footprint, &component.definition),
        );
        footprint.valid()
            && logical
            && (project.board.baseline.is_some()
                || project
                    .board
                    .by_component
                    .get(&self.component)
                    .and_then(|i| project.board.footprints().get(*i))
                    .is_none_or(|f| f.key() == Some(self.key()) && f.mapping.as_ref() == Some(&self.mapping)))
    }

    /// Publish stable identities with explicit placement status for GUI and MCP.
    pub(crate) fn inspect(&self, project: &Project) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        value["component"] = json!(format!("component-{}", self.component));
        value["board"] = json!(project.board_target().map(|target| target.board));
        value["placed"] = json!(project.board.by_component.contains_key(&self.component));
        if let Some(baseline) = &project.board.baseline {
            value["pending_update"] = json!(!baseline.validate_assignment(self));
        }
        if !project.components.contains_key(&self.component) {
            value["pending_removal"] = json!(true);
        }
        value
    }
}

impl Project {
    /// List built-in and project-pinned physical packages by immutable identity. Old pinned
    /// revisions remain selectable offline; conflicting current-source content is never adopted.
    pub fn footprint_catalog(&self) -> Vec<Arc<definition::FootprintDefinition>> {
        let mut items = self.board.definitions.clone();
        for id in definition::STARTER_IDS {
            if let Some(item) = definition::starter(id) {
                items.entry(item.key()).or_insert(item);
            }
        }
        items.into_values().collect()
    }

    /// Publish source identities, pad numbers and provenance without copying full drawings into
    /// every inspection response. Source registration/search can extend this bounded catalog later.
    pub(crate) fn inspect_footprint_catalog(&self) -> Value {
        json!(
            self.footprint_catalog()
                .iter()
                .map(|d| json!({
                    "library": d.library, "id": d.id, "revision": d.revision,
                    "name": d.name, "pads": d.pads.iter().map(|pad| &pad.number).collect::<Vec<_>>(),
                    "license": d.license, "source": d.source
                }))
                .collect::<Vec<_>>()
        )
    }

    /// Report explicit choices and physical legacy placements as assignments without migrating
    /// their saved bytes. Marker-only legacy placements remain outside the physical catalog.
    pub fn footprint_assignments(&self) -> Vec<Value> {
        let mut assignments = self.board.assignments.clone();
        for f in self.board.footprints() {
            if let (Some(library), Some(mapping)) = (&f.library, &f.mapping) {
                assignments.entry(f.component).or_insert_with(|| Assignment {
                    component: f.component,
                    library: library.clone(),
                    definition: f.definition.clone(),
                    definition_revision: f.definition_revision,
                    mapping: mapping.clone(),
                });
            }
        }
        assignments.values().map(|a| a.inspect(self)).collect()
    }
}

/// Canonical component identities are independent of a sheet's symbol placement IDs.
fn component_id(params: &Value, project: &Project) -> Result<u64, CommandError> {
    let text = params["component"].as_str().unwrap_or_default();
    text.strip_prefix("component-")
        .and_then(|id| id.parse::<u64>().ok())
        .filter(|id| text == format!("component-{id}") && project.components.contains_key(id))
        .ok_or_else(|| CommandError::InvalidParams("unknown component ID".into()))
}

impl Session {
    /// Validate identity and complete mappings before pinning intent. Baseline mode
    /// stages desired choices independently of accepted geometry; legacy placed
    /// changes remain refused, and identical choices create no history.
    pub(crate) fn assign_footprint(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let component = component_id(params, project)?;
        let assignment = Assignment {
            component,
            library: params["library"].as_str().unwrap_or_default().into(),
            definition: params["definition"].as_str().unwrap_or_default().into(),
            definition_revision: params["definition_revision"].as_u64().and_then(|r| r.try_into().ok()).unwrap_or(0),
            mapping: super::parse_mapping(&params["mapping"]).ok_or_else(|| CommandError::InvalidParams("invalid terminal-to-pad mapping".into()))?,
        };
        let key = assignment.key();
        let definition = project
            .board
            .definitions
            .get(&key)
            .cloned()
            .or_else(|| definition::starter(&assignment.definition).filter(|d| d.key() == key))
            .ok_or_else(|| CommandError::InvalidParams("unavailable footprint identity/revision".into()))?;
        let mut next = project.clone();
        Arc::make_mut(&mut next.board).definitions.insert(key, definition);
        if !assignment.valid(&next) {
            let message = if next.board.baseline.is_none() && next.board.by_component.contains_key(&component) {
                "Changing a placed package or mapping requires a reviewed board update."
            } else {
                "Map every terminal and pad exactly once."
            };
            return Err(CommandError::InvalidParams(message.into()));
        }
        if next.board.assignments.get(&component) == Some(&assignment)
            || (!next.board.assignments.contains_key(&component)
                && next
                    .board
                    .footprints()
                    .iter()
                    .any(|f| f.component == component && f.key() == Some(assignment.key()) && f.mapping.as_ref() == Some(&assignment.mapping)))
        {
            return Ok(());
        }
        Arc::make_mut(&mut next.board).assignments.insert(component, assignment);
        let used = next.board.used_package_keys();
        Arc::make_mut(&mut next.board).definitions.retain(|key, _| used.contains(key));
        if next.board.baseline.is_some() {
            crate::persistence::encode_documents(Arc::new(next.clone())).map_err(CommandError::InvalidParams)?;
        }
        self.change(next);
        Ok(())
    }

    /// Create one placement from a pinned assignment in one undo step. Source availability is
    /// irrelevant; duplicate placement, stale revisions and out-of-range positions leave it intact.
    /// Pending desired choices must be accepted before they can create physical geometry.
    pub(crate) fn place_assigned_footprint(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let component = component_id(params, project)?;
        let assignment = project
            .board
            .assignments
            .get(&component)
            .ok_or_else(|| CommandError::InvalidParams("component has no unplaced footprint assignment".into()))?;
        if project.board.baseline.as_ref().is_some_and(|baseline| !baseline.validate_assignment(assignment)) {
            return Err(CommandError::Unavailable("Apply the reviewed package update before placing this component.".into()));
        }
        let position = Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() };
        let footprint = assignment.footprint(self.next_object_id()?, position);
        let mut next = project.clone();
        Arc::make_mut(&mut next.board).append(footprint).map_err(|e| CommandError::InvalidParams(e.into()))?;
        self.change(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
