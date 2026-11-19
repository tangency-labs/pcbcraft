//! A placed part selects at most one declared alternate per logical terminal.
use crate::{
    CommandError, Project, Session,
    schematic::{PartInstance, PinDefinition},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Change one part-wide terminal selection without moving any wire endpoints.
pub const SET: &str = "part_instances.set_pin_function";
/// Alternate names share the native terminal-name bound.
pub(crate) const MAX_NAME: usize = 32;
/// A compact menu and bounded native record for one terminal.
pub const MAX_ALTERNATES: usize = 16;
/// Bound the additional entry-point payload independently of the number of placements.
pub(crate) const MAX_CHOICES: usize = 4096;

/// Ordered on disk by terminal number; an array makes duplicate terminals detectable.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Choice {
    terminal: String,
    function: String,
}

/// Resolve stored choices only against the pinned symbol, never the current library catalog.
pub(crate) fn read_choices(records: Vec<Choice>, part: &PartInstance) -> Result<BTreeMap<String, String>, String> {
    if records.is_empty() || records.len() > part.definition.pins().len() {
        return Err("Invalid alternate pin function record count.".into());
    }
    let mut choices = BTreeMap::new();
    for Choice { terminal, function } in records {
        let valid = part.definition.pin_index(&terminal).is_some_and(|index| part.definition.pins()[index].alternate(&function).is_some());
        if !valid || choices.insert(terminal, function).is_some() {
            return Err("Unknown or duplicate alternate pin function selection.".into());
        }
    }
    Ok(choices)
}

/// Omit empty selections so old records retain their exact encoding.
pub(crate) fn write_choices(choices: &BTreeMap<String, String>) -> Option<Vec<Choice>> {
    (!choices.is_empty())
        .then(|| choices.iter().map(|(terminal, function)| Choice { terminal: terminal.clone(), function: function.clone() }).collect())
}

impl Project {
    /// Adopt modern folder and occurrence metadata only on a candidate carrying choices.
    /// This also lets paste retain choices when the destination was opened from legacy storage.
    pub(crate) fn with_pin_function_storage(&self) -> Result<Self, String> {
        let project = if self.layout.is_some() { self.clone() } else { self.with_board_folders(".")? };
        if project.has_sheet_occurrences() { Ok(project) } else { project.with_sheet_occurrences() }
    }

    /// Count sparse selections, not all pin appearances.
    pub(crate) fn pin_function_count(&self) -> usize {
        self.components.values().map(|part| part.pin_functions.len()).sum()
    }
}

/// Accept trimmed single-line names as case-sensitive selection keys.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.trim() == name && name.chars().count() <= MAX_NAME && !name.chars().any(char::is_control)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Target<'a> {
    part: &'a str,
    terminal: &'a str,
}

impl Session {
    /// Resolve the pinned definition, then either report or commit the same validated change.
    /// A dry run or unchanged selection never creates history or adopts project metadata.
    pub(crate) fn set_pin_function(&mut self, params: &Value) -> Result<Value, CommandError> {
        self.check_revision(params)?;
        let invalid = |reason: &str| CommandError::InvalidParams(reason.to_owned());
        let target: Target<'_> = serde::Deserialize::deserialize(&params["target"]).map_err(|_| invalid("Expected a logical part terminal."))?;
        let id = crate::schematic::wiring::parse_id(target.part, "component-").ok_or_else(|| invalid("Invalid part instance identity."))?;
        let original = self.open_project()?;
        let part = original.components.get(&id).ok_or_else(|| invalid("Unknown part instance."))?;
        let pin =
            part.definition.pin_index(target.terminal).map(|index| &part.definition.pins()[index]).ok_or_else(|| invalid("Unknown terminal."))?;
        let next = params["function"].as_str();
        let alternate = next.map(|name| pin.alternate(name).ok_or_else(|| invalid("Unknown alternate pin function."))).transpose()?;
        let previous = part.pin_function(target.terminal);
        let changed = previous != next;
        let dry_run = params["dry_run"].as_bool().unwrap_or(false);
        if previous.is_none() && next.is_some() && original.pin_function_count() >= MAX_CHOICES {
            return Err(CommandError::Unavailable("Alternate pin function selection limit reached.".into()));
        }
        let effective = |function: Option<&crate::schematic::AlternatePinFunction>, pin: &PinDefinition| match function {
            Some(function) => json!({"function":function.name,"name":function.name,"electrical_type":function.electrical_type}),
            None => json!({"function":null,"name":pin.name,"electrical_type":pin.electrical_type}),
        };
        let mut report = json!({"target":params["target"],"dry_run":dry_run,"changed":changed,
            "before":effective(previous.and_then(|name| pin.alternate(name)), pin),"after":effective(alternate, pin)});
        if changed {
            if dry_run {
                if original.layout.is_none() || !original.has_sheet_occurrences() {
                    original.with_pin_function_storage().map_err(CommandError::InvalidParams)?;
                }
            } else {
                let mut project = original.with_pin_function_storage().map_err(CommandError::InvalidParams)?;
                let part = std::sync::Arc::make_mut(&mut project.components).get_mut(&id).ok_or_else(|| invalid("Unknown part instance."))?;
                if let Some(name) = next {
                    part.pin_functions.insert(target.terminal.into(), name.into());
                } else {
                    part.pin_functions.remove(target.terminal);
                }
                self.change(project)?;
            }
        }
        report["revision"] = json!(self.revision());
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
