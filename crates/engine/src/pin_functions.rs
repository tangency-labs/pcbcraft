//! A placed part selects at most one declared alternate per logical terminal.
use crate::{
    CommandError, Project, Session,
    schematic::{ElectricalType, PartInstance, PinDefinition, SymbolPlacement},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

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

/// Sparse authored selections in one snapshot. Copying a snapshot shares this table;
/// editing a choice copies at most MAX_CHOICES owners and only one owner's terminal map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selections {
    owners: BTreeMap<u64, Arc<BTreeMap<String, String>>>,
    count: usize,
}

impl Selections {
    /// Return the bounded map for one owner without visiting unrelated components.
    pub(crate) fn owner(&self, id: u64) -> Option<&BTreeMap<String, String>> {
        #[cfg(test)]
        OWNER_LOOKUPS.set(OWNER_LOOKUPS.get() + 1);
        self.owners.get(&id).map(Arc::as_ref)
    }

    /// Replace one owner's choices and maintain the project limit without a component scan.
    /// Empty owners are omitted so the number of map entries is also bounded by MAX_CHOICES.
    pub(crate) fn replace(&mut self, id: u64, choices: BTreeMap<String, String>) -> Result<(), String> {
        let count = self.count - self.owner(id).map_or(0, BTreeMap::len) + choices.len();
        if count > MAX_CHOICES {
            return Err("Alternate pin function selection limit reached.".into());
        }
        if choices.is_empty() {
            self.owners.remove(&id);
        } else {
            self.owners.insert(id, Arc::new(choices));
        }
        self.count = count;
        Ok(())
    }

    /// Drop a retired owner without keeping a tombstone in the sparse table.
    pub(crate) fn remove(&mut self, id: u64) {
        if let Some(choices) = self.owners.remove(&id) {
            self.count -= choices.len();
        }
    }
}

impl Project {
    /// Adopt modern folder and occurrence metadata only on a candidate carrying choices.
    /// This also lets paste retain choices when the destination was opened from legacy storage.
    pub(crate) fn with_pin_function_storage(&self) -> Result<Self, String> {
        let project = if let Some(layout) = &self.layout {
            if self.numeric_id_high_water.is_some() { self.clone() } else { self.with_board_folders(&layout.root)? }
        } else {
            self.with_board_folders(".")?
        };
        if project.has_sheet_occurrences() { Ok(project) } else { project.with_sheet_occurrences() }
    }

    /// Count sparse choices without visiting any components or pin appearances.
    pub(crate) fn pin_function_count(&self) -> usize {
        self.pin_functions.count
    }

    /// Return the explicit selection in this snapshot; absence uses the primary function.
    pub fn pin_function(&self, part: &PartInstance, terminal: &str) -> Option<&str> {
        self.pin_functions.owner(part.ordinal)?.get(terminal).map(String::as_str)
    }

    /// Read the effective metadata of a pin in this part's pinned definition.
    /// An undeclared or unselected alternate uses the primary function.
    pub fn effective_pin<'a>(&self, part: &PartInstance, pin: &'a PinDefinition) -> (Option<&'a str>, ElectricalType) {
        let choices = if pin.alternates.is_empty() { None } else { self.pin_functions.owner(part.ordinal) };
        effective(choices, pin)
    }

    /// Draw the addressed unit with its part's effective terminal names.
    /// Resolve sparse choice data once, even for a unit with many terminal appearances.
    pub fn symbol_graphics(&self, symbol: &SymbolPlacement) -> Vec<crate::schematic::Shape> {
        let choices = if symbol.definition.format() == crate::library::SYMBOL_FORMAT_V5 { self.pin_functions.owner(symbol.component) } else { None };
        symbol.graphics(choices)
    }
}

/// Resolve a pin against one already-addressed selection table.
pub(crate) fn effective<'a>(choices: Option<&BTreeMap<String, String>>, pin: &'a PinDefinition) -> (Option<&'a str>, ElectricalType) {
    (!pin.alternates.is_empty())
        .then(|| choices.and_then(|by_terminal| by_terminal.get(&pin.number)))
        .flatten()
        .and_then(|name| pin.alternate(name))
        .map_or((pin.name.as_deref(), pin.electrical_type), |function| (Some(function.name.as_str()), function.electrical_type))
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
        let previous = original.pin_function(part, target.terminal);
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
            let adopt_storage = original.layout.is_none() || !original.has_sheet_occurrences() || original.numeric_id_high_water.is_none();
            if dry_run {
                if adopt_storage {
                    original.with_pin_function_storage().map_err(CommandError::InvalidParams)?;
                }
            } else {
                let mut project =
                    if adopt_storage { original.with_pin_function_storage().map_err(CommandError::InvalidParams)? } else { original.clone() };
                let mut choices = project.pin_functions.owner(id).cloned().unwrap_or_default();
                if let Some(name) = next {
                    choices.insert(target.terminal.into(), name.into());
                } else {
                    choices.remove(target.terminal);
                }
                Arc::make_mut(&mut project.pin_functions).replace(id, choices).map_err(CommandError::InvalidParams)?;
                if adopt_storage {
                    self.change(project)?;
                } else {
                    // Existing storage already carries its allocation watermark. This edit owns
                    // no identity or topology, and its only new value was validated above.
                    self.publish_change(Arc::new(project), self.revision);
                }
            }
        }
        report["revision"] = json!(self.revision());
        Ok(report)
    }
}

#[cfg(test)]
thread_local! {
    /// One test thread counts lookups without affecting other concurrently tested projects.
    static OWNER_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// A modern selection must not enter the general whole-project preparation path.
    pub(crate) static FULL_PREPARATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Count comparisons for the same addressed placement lookup the property form uses.
    pub(crate) static PLACEMENT_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
