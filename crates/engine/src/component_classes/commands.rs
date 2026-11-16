//! Catalog and membership transactions through the shared command path.
use super::*;
use crate::{CommandError, Session};
use serde_json::{Value, json};
use std::sync::Arc;

/// Read a typed component address. Membership in the current project is checked by the caller.
fn parse_component(value: &Value) -> Result<ComponentKey, CommandError> {
    value.as_str().and_then(component_key).ok_or_else(|| CommandError::InvalidParams("Expected a component ID such as component-1.".into()))
}

/// Read one class UUID parameter.
fn parse_class(value: &Value) -> Result<Uuid, CommandError> {
    serde_json::from_value(value.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))
}

/// Report one component's memberships with its addressable ID.
fn membership(project: &Project, component: ComponentKey) -> Value {
    json!({"component":component_address(component),"classes":project.classes_of_component(component).collect::<Vec<_>>()})
}

impl Project {
    /// Prepare a candidate for class persistence, adopting the current folder and occurrence
    /// metadata when a legacy snapshot is edited. Undo restores the earlier representation.
    fn class_candidate(&self) -> Result<Project, CommandError> {
        let mut candidate = if self.layout.is_none() { self.with_board_folders(".").map_err(CommandError::InvalidParams)? } else { self.clone() };
        if !candidate.has_sheet_occurrences() {
            candidate = candidate.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
        }
        Ok(candidate)
    }
}

impl Session {
    /// Author one atomic component-class catalog transaction. Stale, invalid and no-op requests
    /// publish no catalog or history change. Removing a class that any component belongs to is
    /// refused and names those components.
    pub(crate) fn edit_component_class(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let mut catalog = (*original.component_classes).clone();
        let id = if command == CREATE {
            if catalog.definitions.len() >= MAX_CLASSES {
                return Err(CommandError::Unavailable("Component-class catalog is full.".into()));
            }
            crate::fresh_id()?
        } else {
            parse_class(&params["class"])?
        };
        if command == REMOVE {
            if catalog.get(id).is_none() {
                return Err(CommandError::InvalidParams("Unknown component class.".into()));
            }
            let members: Vec<_> = original
                .component_class_bindings
                .iter()
                .filter(|binding| binding.class == id)
                .map(|binding| component_address(binding.component))
                .collect();
            if !members.is_empty() {
                return Err(CommandError::InvalidParams(format!(
                    "The component class still has members ({}); remove them from the class first.",
                    members.join(", ")
                )));
            }
            catalog.definitions.retain(|class| class.id != id);
        } else {
            let name = params["name"].as_str().ok_or_else(|| CommandError::InvalidParams("Missing class name.".into()))?.to_owned();
            crate::class_catalog::validate_name(&name).map_err(CommandError::InvalidParams)?;
            let revision = if command == CREATE {
                1
            } else {
                let previous = catalog.get(id).ok_or_else(|| CommandError::InvalidParams("Unknown component class.".into()))?;
                if previous.name == name {
                    return Ok(());
                }
                previous.revision.checked_add(1).ok_or_else(|| CommandError::Unavailable("Class revision exhausted.".into()))?
            };
            catalog.definitions.retain(|class| class.id != id);
            catalog.definitions.push(Definition { id, revision, name });
        }
        catalog.definitions.sort_by_key(|class| class.id);
        catalog.validate().map_err(CommandError::InvalidParams)?;
        let mut candidate = original.class_candidate()?;
        candidate.component_classes = Arc::new(catalog);
        self.change(candidate)
    }

    /// Replace one component's complete class set in one undo step. An unchanged set is a no-op;
    /// an unknown component or class refuses the whole request.
    pub(crate) fn set_component_classes(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let component = parse_component(&params["component"])?;
        if !original.components.contains_key(&component) {
            return Err(CommandError::InvalidParams("Unknown component.".into()));
        }
        let classes: BTreeSet<_> = crate::net_classes::parse_set(&params["classes"])?.into_iter().collect();
        if classes.iter().any(|class| original.component_classes.get(*class).is_none()) {
            return Err(CommandError::InvalidParams("Unknown component class.".into()));
        }
        if original.classes_of_component(component).eq(classes.iter().copied()) {
            return Ok(());
        }
        let mut candidate = original.class_candidate()?;
        let bindings = Arc::make_mut(&mut candidate.component_class_bindings);
        bindings.retain(|binding| binding.component != component);
        bindings.extend(classes.into_iter().map(|class| Binding { component, class }));
        bindings.sort();
        candidate.validate_component_class_bindings(&candidate.component_class_bindings).map_err(CommandError::InvalidParams)?;
        self.change(candidate)
    }

    /// Return catalog or membership data even when MCP suppresses full project inspection.
    /// Neither query changes the document, its history or its revision.
    pub(crate) fn component_class_query(&self, command: &str, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        if command == INSPECT {
            return Ok(json!({"revision":self.revision(),"component_classes":project.component_classes()}));
        }
        let memberships = if let Some(value) = params.get("component") {
            let component = parse_component(value)?;
            if !project.components.contains_key(&component) {
                return Err(CommandError::InvalidParams("Unknown component.".into()));
            }
            vec![membership(project, component)]
        } else {
            let components: BTreeSet<_> = project.component_class_bindings.iter().map(|binding| binding.component).collect();
            components.into_iter().map(|component| membership(project, component)).collect()
        };
        Ok(json!({"revision":self.revision(),"component_memberships":memberships}))
    }
}
