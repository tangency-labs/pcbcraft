//! Component table rows: one per physical component in project order, so a multi-unit part is
//! one row. The GUI table and MCP clients read the same rows and edit them with `part.set_fields`.
use crate::{Command, CommandError, Project, Session, schematic::fields::Field};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// List every physical component with the fields `part.set_fields` edits.
pub const INSPECT_FIELDS: &str = "part.inspect_fields";

/// Discoverable through the ordinary engine catalog by GUI and MCP clients.
pub(crate) const COMMAND: Command = Command {
    id: INSPECT_FIELDS,
    title: "Inspect Component Fields",
    summary: "List every physical component once, in project order, with its reference, value, placed unit count, component fields and footprint assignment. A multi-unit part is one row. Reading changes nothing; edit the rows with part.set_fields.",
    params: &[],
};

/// One physical component, however many schematic units of it are placed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComponentRow {
    /// Stable `component-N` identity that `part.set_fields` addresses.
    pub component: String,
    pub reference: String,
    /// The value as the symbol inspector shows it; submitting it unchanged keeps the value.
    pub value: String,
    /// Placed schematic units across every sheet.
    pub units: usize,
    /// Component fields by stable ID, in the shape `part.set_fields` writes.
    pub fields: BTreeMap<String, Field>,
    /// The package choice, or a physical legacy placement's package; None when unassigned.
    pub footprint: Option<FootprintRow>,
}

/// The pinned package a component is assigned to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FootprintRow {
    pub library: String,
    pub definition: String,
    pub definition_revision: u32,
    /// The pinned definition's display name, or its ID when the definition is not loaded.
    pub name: String,
}

impl Project {
    /// One row per part instance in project order. Units are counted, never listed, so the
    /// table edits each physical component once.
    pub fn component_rows(&self) -> Vec<ComponentRow> {
        let assignments = self.effective_package_assignments();
        let mut units = BTreeMap::<u64, usize>::new();
        for symbol in self.all_symbols() {
            *units.entry(symbol.component).or_default() += 1;
        }
        self.components
            .values()
            .map(|part| ComponentRow {
                component: part.id(),
                reference: part.reference.clone(),
                value: part.display_value().into_owned(),
                units: units.get(&part.ordinal).copied().unwrap_or_default(),
                fields: part.metadata.fields.clone(),
                footprint: assignments.get(&part.ordinal).map(|assignment| FootprintRow {
                    name: self.assignment_definition(&assignment.key()).map_or_else(|| assignment.definition.clone(), |d| d.name.clone()),
                    library: assignment.library.clone(),
                    definition: assignment.definition.clone(),
                    definition_revision: assignment.definition_revision,
                }),
            })
            .collect()
    }
}

impl Session {
    /// Read the component rows at the current revision without touching history.
    pub(crate) fn inspect_part_fields(&self) -> Result<Value, CommandError> {
        Ok(json!({"revision": self.revision, "components": self.open_project()?.component_rows()}))
    }
}
