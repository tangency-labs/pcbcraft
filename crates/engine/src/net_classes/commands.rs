//! Catalog transactions and read-only explicit-class resolution through the shared command path.
use super::*;
use crate::{CommandError, Session};
use serde_json::{Value, json};
use std::sync::Arc;

/// Catalog discovery uses the same field ranges as native validation.
pub(crate) fn rules_schema() -> Value {
    let fields: BTreeMap<_, _> = Property::ALL
        .into_iter()
        .map(|property| {
            let (min, max) = property.range();
            (property.as_str(), json!({"type":"integer","minimum":min,"maximum":max}))
        })
        .collect();
    json!({"type":"object","properties":fields,"additionalProperties":false})
}

/// Explicit sets may be empty; repetitions are rejected at the command boundary.
pub(crate) fn set_schema() -> Value {
    json!({"type":"array","maxItems":MAX_CLASSES,"uniqueItems":true,"items":{"type":"string","format":"uuid"}})
}

/// Parse typed class-set parameters; catalog membership is checked by the resolver.
pub(crate) fn parse_set(value: &Value) -> Result<Vec<Uuid>, CommandError> {
    let ids: Vec<Uuid> = serde_json::from_value(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
    if ids.len() > MAX_CLASSES || ids.iter().any(Uuid::is_nil) || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(CommandError::InvalidParams("Class sets require unique non-nil UUIDs within the catalog limit.".into()));
    }
    Ok(ids)
}

impl Session {
    /// Author one atomic class transaction. Stale/invalid/no-op requests publish no catalog or
    /// history changes. Legacy storage is adopted only after the complete candidate validates.
    pub(crate) fn edit_net_class(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let mut catalog = (*original.net_classes).clone();
        let id = if command == CREATE {
            if catalog.definitions.len() >= MAX_CLASSES {
                return Err(CommandError::Unavailable("Net-class catalog is full.".into()));
            }
            crate::fresh_id()?
        } else {
            serde_json::from_value(params["class"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?
        };
        if command == REMOVE {
            if catalog.get(id).is_none() {
                return Err(CommandError::InvalidParams("Unknown net class.".into()));
            }
            catalog.definitions.retain(|class| class.id != id);
        } else {
            let name = params["name"].as_str().ok_or_else(|| CommandError::InvalidParams("Missing class name.".into()))?.to_owned();
            validate_name(&name).map_err(CommandError::InvalidParams)?;
            let rules: Rules = serde_json::from_value(params["rules"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
            validate_rules(&rules).map_err(CommandError::InvalidParams)?;
            let revision = if command == CREATE {
                1
            } else {
                let previous = catalog.get(id).ok_or_else(|| CommandError::InvalidParams("Unknown net class.".into()))?;
                if previous.name == name && previous.rules == rules {
                    return Ok(());
                }
                previous.revision.checked_add(1).ok_or_else(|| CommandError::Unavailable("Class revision exhausted.".into()))?
            };
            catalog.definitions.retain(|class| class.id != id);
            catalog.definitions.push(Definition { id, revision, name, rules });
        }
        catalog.definitions.sort_by_key(|class| class.id);
        catalog.validate().map_err(CommandError::InvalidParams)?;
        let mut candidate =
            if original.layout.is_none() { original.with_board_folders(".").map_err(CommandError::InvalidParams)? } else { original.clone() };
        if !candidate.has_sheet_occurrences() {
            candidate = candidate.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
        }
        candidate.net_classes = Arc::new(catalog);
        self.change(candidate)
    }

    /// Return catalog/resolution data even when MCP suppresses full project inspection. Omitted
    /// board uses project built-ins, never an implicit first board. A supplied board is validated.
    pub(crate) fn net_class_query(&self, command: &str, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        if command == INSPECT {
            return Ok(json!({"revision":self.revision(),"net_classes":project.net_classes()}));
        }
        let classes = parse_set(&params["classes"])?;
        let board = params.get("board").map(|value| project.resolve_board(value.as_str())).transpose()?;
        let resolution = if let Some(other) = params.get("other_classes") {
            json!(project.resolve_class_clearance(&classes, &parse_set(other)?, board)?)
        } else {
            json!(project.resolve_net_classes(&classes, board)?)
        };
        Ok(json!({"revision":self.revision(),"resolution":resolution}))
    }
}
