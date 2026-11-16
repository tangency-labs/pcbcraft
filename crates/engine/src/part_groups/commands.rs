//! Catalog and membership transactions through the shared command path.
use super::*;
use crate::{CommandError, Session};
use serde_json::{Value, json};
use std::sync::Arc;

/// Read a typed part-instance address. Membership in the current project is checked by the caller.
fn parse_part_instance(value: &Value) -> Result<PartInstanceKey, CommandError> {
    value
        .as_str()
        .and_then(part_instance_key)
        .ok_or_else(|| CommandError::InvalidParams(format!("Expected a part-instance ID such as {}.", part_instance_address(1))))
}

/// Read one group UUID parameter.
fn parse_group(value: &Value) -> Result<Uuid, CommandError> {
    serde_json::from_value(value.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))
}

/// Report one part instance's memberships with its addressable ID.
fn membership(project: &Project, part_instance: PartInstanceKey) -> Value {
    json!({"part_instance":part_instance_address(part_instance),"groups":project.groups_of_part_instance(part_instance).collect::<Vec<_>>()})
}

impl Project {
    /// Prepare a candidate for group persistence, adopting the current folder and occurrence
    /// metadata when a legacy snapshot is edited. Undo restores the earlier representation.
    fn group_candidate(&self) -> Result<Project, CommandError> {
        let mut candidate = if self.layout.is_none() { self.with_board_folders(".").map_err(CommandError::InvalidParams)? } else { self.clone() };
        if !candidate.has_sheet_occurrences() {
            candidate = candidate.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
        }
        Ok(candidate)
    }
}

impl Session {
    /// Author one atomic part-group catalog transaction. Stale, invalid and no-op requests
    /// publish no catalog or history change. Removing a group that any part instance belongs to
    /// is refused and names those part instances.
    pub(crate) fn edit_part_group(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let mut catalog = (*original.part_groups).clone();
        let id = if command == CREATE {
            if catalog.groups.len() >= MAX_GROUPS {
                return Err(CommandError::Unavailable("Part-group catalog is full.".into()));
            }
            crate::fresh_id()?
        } else {
            parse_group(&params["group"])?
        };
        if command == REMOVE {
            if catalog.get(id).is_none() {
                return Err(CommandError::InvalidParams("Unknown part group.".into()));
            }
            let members: Vec<_> = original
                .part_group_memberships
                .iter()
                .filter(|membership| membership.group == id)
                .map(|membership| part_instance_address(membership.part_instance))
                .collect();
            if !members.is_empty() {
                return Err(CommandError::InvalidParams(format!(
                    "The part group still has members ({}); remove them from the group first.",
                    members.join(", ")
                )));
            }
            catalog.groups.retain(|group| group.id != id);
        } else {
            let name = params["name"].as_str().ok_or_else(|| CommandError::InvalidParams("Missing group name.".into()))?.to_owned();
            crate::named_catalog::validate_name("group", &name).map_err(CommandError::InvalidParams)?;
            let revision = if command == CREATE {
                1
            } else {
                let previous = catalog.get(id).ok_or_else(|| CommandError::InvalidParams("Unknown part group.".into()))?;
                if previous.name == name {
                    return Ok(());
                }
                previous.revision.checked_add(1).ok_or_else(|| CommandError::Unavailable("Group revision exhausted.".into()))?
            };
            catalog.groups.retain(|group| group.id != id);
            catalog.groups.push(PartGroup { id, revision, name });
        }
        catalog.groups.sort_by_key(|group| group.id);
        catalog.validate().map_err(CommandError::InvalidParams)?;
        let mut candidate = original.group_candidate()?;
        candidate.part_groups = Arc::new(catalog);
        self.change(candidate)
    }

    /// Replace one part instance's complete group set in one undo step. An unchanged set is a
    /// no-op; an unknown part instance or group refuses the whole request.
    pub(crate) fn set_part_instance_groups(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let part_instance = parse_part_instance(&params["part_instance"])?;
        if !original.has_part_instance(part_instance) {
            return Err(CommandError::InvalidParams("Unknown part instance.".into()));
        }
        let groups: BTreeSet<_> = crate::net_classes::parse_set(&params["groups"])?.into_iter().collect();
        if groups.iter().any(|group| original.part_groups.get(*group).is_none()) {
            return Err(CommandError::InvalidParams("Unknown part group.".into()));
        }
        if original.groups_of_part_instance(part_instance).eq(groups.iter().copied()) {
            return Ok(());
        }
        let mut candidate = original.group_candidate()?;
        Arc::make_mut(&mut candidate.part_group_memberships).replace(part_instance, &groups);
        candidate.validate_part_group_memberships(candidate.part_group_memberships.iter()).map_err(CommandError::InvalidParams)?;
        self.change(candidate)
    }

    /// Return catalog or membership data even when MCP suppresses full project inspection.
    /// Neither query changes the document, its history or its revision.
    pub(crate) fn part_group_query(&self, command: &str, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        if command == INSPECT {
            return Ok(json!({"revision":self.revision(),"part_groups":project.part_groups()}));
        }
        let memberships = if let Some(value) = params.get("part_instance") {
            let part_instance = parse_part_instance(value)?;
            if !project.has_part_instance(part_instance) {
                return Err(CommandError::InvalidParams("Unknown part instance.".into()));
            }
            vec![membership(project, part_instance)]
        } else {
            project.part_group_memberships.part_instances().map(|part_instance| membership(project, part_instance)).collect()
        };
        Ok(json!({"revision":self.revision(),"part_instance_memberships":memberships}))
    }
}
