//! Bulk component field edits: one validated, revision-checked transaction over physical components,
//! for a component table and MCP clients. Each row addresses a part instance, never a schematic unit.
use crate::{
    ASSIGN_FOOTPRINT, Command, CommandError, Param, ParamKind, Project, Session,
    schematic::{self, fields::Field},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Set reference, value, component fields and footprint assignments on several components at once.
pub const SET_FIELDS: &str = "part.set_fields";

/// Optional fields a component holds, as checked by its metadata.
const MAX_FIELDS: usize = 64;

/// Discoverable through the ordinary engine catalog by GUI and MCP clients.
pub(crate) const COMMAND: Command = Command {
    id: SET_FIELDS,
    title: "Edit Component Fields",
    summary: "Set reference, value, component fields and footprint assignments on several physical components in one undo step. The whole edit is validated first; unnamed fields keep their content. Returns every changed field; dry_run returns the same list without editing.",
    params: &[
        Param {
            name: "edits",
            required: true,
            summary: "One row per component ID: optional reference, value, fields (a null field removes it) and footprint",
            kind: ParamKind::PartFieldEdits,
        },
        Param {
            name: "dry_run",
            required: false,
            summary: "Validate and list the changes without editing the project or history",
            kind: ParamKind::Flag,
        },
        crate::EXPECTED_REVISION,
    ],
};

/// One table row. Omitted members keep their current content.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    component: String,
    reference: Option<String>,
    value: Option<String>,
    #[serde(default)]
    fields: BTreeMap<String, Option<Field>>,
    footprint: Option<Footprint>,
}

/// The exact choice `pcb.assign_footprint` takes, without its component and revision.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Footprint {
    library: String,
    definition: String,
    definition_revision: u64,
    mapping: Value,
    unconnected_pads: Option<Value>,
}

/// Publish the row shape; footprint members reuse `pcb.assign_footprint`'s parameter schemas.
pub(crate) fn edits_schema() -> Value {
    let field = json!({"oneOf":[{"type":"null"},{"type":"object","additionalProperties":false,"required":["name","value"],
        "properties":{"name":{"type":"string","minLength":1,"maxLength":64},"value":{"type":"string","maxLength":2048}}}]});
    let footprint = json!({"type":"object","additionalProperties":false,"required":["library","definition","definition_revision","mapping"],
        "properties":{"library":{"type":"string"},"definition":{"type":"string"},"definition_revision":ParamKind::Revision.schema(),
            "mapping":ParamKind::FootprintMapping.schema(),"unconnected_pads":ParamKind::UnconnectedPads.schema()}});
    json!({"type":"array","minItems":1,"maxItems":schematic::MAX_SYMBOLS,"items":{"type":"object","additionalProperties":false,"required":["component"],
        "properties":{"component":{"type":"string","pattern":"^component-[1-9][0-9]*$"},"reference":{"type":"string"},"value":{"type":"string"},
            "fields":{"type":"object","maxProperties":MAX_FIELDS,"additionalProperties":field},"footprint":footprint}}})
}

/// Parse the bounded row list. Explicit nulls are refused where the schema has no null,
/// and a component must be named in the canonical `component-N` form.
fn parse_edits(value: &Value) -> Result<Vec<Edit>, &'static str> {
    let items = value.as_array().filter(|items| (1..=schematic::MAX_SYMBOLS).contains(&items.len())).ok_or("must list 1 to 4096 edits")?;
    items
        .iter()
        .map(|item| {
            if ["reference", "value", "fields", "footprint"].iter().any(|name| item.get(*name).is_some_and(Value::is_null)) {
                return Err("rows take no null members; omit a member to keep it");
            }
            let edit: Edit = serde_json::from_value(item.clone()).map_err(|_| "rows take component, reference, value, fields and footprint")?;
            if ordinal(&edit.component).is_none() {
                return Err("rows name a component as component-N");
            }
            if edit.fields.len() > MAX_FIELDS {
                return Err("a row sets at most 64 fields");
            }
            Ok(edit)
        })
        .collect()
}

/// Check the row list for catalog validation.
pub(crate) fn validate_edits(value: &Value) -> Result<(), &'static str> {
    parse_edits(value).map(|_| ())
}

/// The numeric identity in a canonical `component-N` address.
fn ordinal(text: &str) -> Option<u64> {
    text.strip_prefix("component-").and_then(|id| id.parse::<u64>().ok()).filter(|id| *id > 0 && text == format!("component-{id}"))
}

/// Name the row in a parameter or lookup error, so a table can point at it.
fn row_error(component: &str, error: CommandError) -> CommandError {
    match error {
        CommandError::InvalidParams(message) => CommandError::InvalidParams(format!("{component}: {message}")),
        CommandError::NotFound(message) => CommandError::NotFound(format!("{component}: {message}")),
        other => other,
    }
}

impl Session {
    /// Apply every row to a candidate, validate the complete result, then publish it as one undo
    /// step. A refused row refuses all rows. A dry run, or a request that changes nothing,
    /// publishes nothing. Returns the revision and every changed `{component, reference, field,
    /// old, new}` in row order; `reference` is the component's reference before the edit.
    pub(crate) fn set_part_fields(&mut self, params: &Value) -> Result<Value, CommandError> {
        self.set_part_fields_with_commit(params, |_, _| Ok(()))
    }

    /// Validate the candidate and its native save before giving the host the exact reply to
    /// check. The host may refuse the reply or publication, leaving revision and history intact.
    /// The Boolean passed to `check_reply` indicates whether publication would change the project.
    pub fn set_part_fields_with_commit<E: From<CommandError>>(
        &mut self,
        params: &Value,
        check_reply: impl FnOnce(&Value, bool) -> Result<(), E>,
    ) -> Result<Value, E> {
        self.authorize_command(SET_FIELDS)?;
        self.refuse_stale(SET_FIELDS, params)?;
        self.validate_project_command(SET_FIELDS, params)?;
        let edits = parse_edits(&params["edits"]).map_err(|message| CommandError::InvalidParams(format!("`edits` {message}")))?;
        let before = self.open_project()?;
        let candidate = self.edited_components(before, &edits)?;
        let changes = changes(before, &candidate, &edits);
        let prepared = if changes.is_empty() {
            None
        } else {
            let (candidate, highest) = self.prepare_change(candidate)?;
            let candidate = self.retain_allocation_watermark(Arc::new(candidate));
            crate::persistence::encode_documents(candidate.clone()).map_err(CommandError::InvalidParams)?;
            (params["dry_run"].as_bool() != Some(true)).then_some((candidate, highest))
        };
        let revision = prepared.as_ref().map_or(self.revision, |(_, highest)| (self.revision + 1).max(*highest));
        let reply = json!({"revision": revision, "changes": changes});
        check_reply(&reply, prepared.is_some())?;
        if let Some((candidate, highest)) = prepared {
            self.publish_change(candidate, highest);
            self.report_committed(SET_FIELDS, params);
        }
        Ok(reply)
    }

    /// Build the edited project. Reference, value and field rules match the symbol inspector;
    /// references are unique across the complete result. Footprint rows run `pcb.assign_footprint`
    /// in a disposable session, so its pinning and mapping rules apply unchanged.
    fn edited_components(&self, before: &Project, edits: &[Edit]) -> Result<Project, CommandError> {
        let mut candidate = before.clone();
        let mut seen = BTreeSet::new();
        for edit in edits {
            let id = ordinal(&edit.component)
                .filter(|id| seen.insert(*id))
                .ok_or_else(|| CommandError::InvalidParams(format!("{}: name each component in one row only", edit.component)))?;
            let fail = |message: String| row_error(&edit.component, CommandError::InvalidParams(message));
            let part = Arc::make_mut(&mut candidate.components)
                .get_mut(&id)
                .ok_or_else(|| row_error(&edit.component, CommandError::NotFound("unknown component ID".into())))?;
            if let Some(reference) = &edit.reference {
                let reference = reference.trim();
                schematic::check_reference(reference).map_err(fail)?;
                reference.clone_into(&mut part.reference);
            }
            if let Some(value) = &edit.value {
                let mut value = value.trim().to_owned();
                if value != part.value
                    && let Some(selection) = &mut part.part
                    && let Some(canonical) = selection.set_primary_value(&value).map_err(fail)?
                {
                    value = canonical;
                }
                schematic::check_value(&value).map_err(fail)?;
                part.value = value;
            }
            for (key, field) in &edit.fields {
                match field {
                    Some(field) => part.metadata.fields.insert(key.clone(), field.clone()),
                    None => part.metadata.fields.remove(key),
                };
            }
            part.metadata.validate().map_err(fail)?;
            let metadata = part.metadata.clone();
            for symbol in candidate.all_symbols().filter(|symbol| symbol.component == id) {
                schematic::fields::validate_layouts(&symbol.field_layout, &metadata).map_err(fail)?;
            }
        }
        for edit in edits.iter().filter(|edit| edit.reference.is_some()) {
            let id = ordinal(&edit.component).unwrap_or_default();
            let reference = candidate.components.get(&id).map(|part| part.reference.as_str()).unwrap_or_default();
            if candidate.components.values().any(|other| other.ordinal != id && other.reference == reference) {
                return Err(CommandError::InvalidParams(format!("{}: another component is already {reference}", edit.component)));
            }
        }
        if edits.iter().all(|edit| edit.footprint.is_none()) {
            return Ok(candidate);
        }
        let mut transaction = Self {
            project: Some(Arc::new(candidate)),
            libraries: self.libraries.clone(),
            revision: self.revision,
            history_root_revision: self.history_root_revision,
            ..Self::default()
        };
        for edit in edits {
            let Some(footprint) = &edit.footprint else { continue };
            let mut row = Map::new();
            row.insert("component".into(), json!(edit.component));
            row.insert("library".into(), json!(footprint.library));
            row.insert("definition".into(), json!(footprint.definition));
            row.insert("definition_revision".into(), json!(footprint.definition_revision));
            row.insert("mapping".into(), footprint.mapping.clone());
            if let Some(pads) = &footprint.unconnected_pads {
                row.insert("unconnected_pads".into(), pads.clone());
            }
            row.insert("expected_revision".into(), json!(transaction.revision));
            transaction.apply(ASSIGN_FOOTPRINT, &Value::Object(row)).map_err(|error| row_error(&edit.component, error))?;
        }
        let candidate = transaction.open_project()?.clone();
        super::storage::validate_project(&candidate).map_err(CommandError::InvalidParams)?;
        Ok(candidate)
    }
}

/// List each changed field of each row: reference, value, component fields by ID, then the
/// footprint assignment. Absent fields and assignments are null.
fn changes(before: &Project, after: &Project, edits: &[Edit]) -> Vec<Value> {
    let mut changes = Vec::new();
    for edit in edits {
        let id = ordinal(&edit.component).unwrap_or_default();
        let (Some(old), Some(new)) = (before.components.get(&id), after.components.get(&id)) else { continue };
        let reference = old.reference.clone();
        let mut push = |field: &str, old: Value, new: Value| {
            if old != new {
                changes.push(json!({"component": edit.component, "reference": reference, "field": field, "old": old, "new": new}));
            }
        };
        push("reference", json!(old.reference), json!(new.reference));
        push("value", json!(old.value), json!(new.value));
        let keys: BTreeSet<_> = old.metadata.fields.keys().chain(new.metadata.fields.keys()).collect();
        for key in keys {
            push(key, json!(old.metadata.fields.get(key)), json!(new.metadata.fields.get(key)));
        }
        let assignment = |project: &Project| {
            project.package_assignments().get(&id).map(|assignment| {
                let mut value = serde_json::to_value(assignment).unwrap_or(Value::Null);
                if let Some(object) = value.as_object_mut() {
                    object.remove("component");
                }
                value
            })
        };
        push("footprint", json!(assignment(before)), json!(assignment(after)));
    }
    changes
}
