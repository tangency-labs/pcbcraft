//! Project-wide component table: one row per physical component from `part.inspect_fields`'s
//! rows, a display-only filter and sort, and a grouped field edit that is previewed and applied
//! through the shared `part.set_fields` command as one undo step.
use crate::{Dialog, PcbApp};
use pcbcraft_chrome::{
    controls,
    library_workspace::{self as tables, ColumnSort, TableEntry},
    properties, shell,
};
use pcbcraft_engine::{
    Project,
    library::parts::PadIds,
    part_instances::{ComponentRow, SET_FIELDS},
    schematic::fields::{self, STANDARD},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Standard fields shown as their own columns, after reference, value and footprint.
const FIXED_FIELDS: [(&str, &str); 3] = [("manufacturer", "Manufacturer"), ("manufacturer_part_number", "Part number"), ("datasheet", "Datasheet")];

/// What one column shows and what a grouped edit of it writes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Column {
    Reference,
    Value,
    Footprint,
    /// A standard field by stable ID.
    Standard(&'static str),
    /// Other fields grouped by name, case-insensitively: custom fields have their own ID per part.
    Named(String),
}

impl Column {
    /// The header text, which is also the grouped edit's field choice.
    fn label(&self) -> String {
        match self {
            Self::Reference => "Reference".into(),
            Self::Value => "Value".into(),
            Self::Footprint => "Footprint".into(),
            Self::Standard(id) => FIXED_FIELDS.iter().chain(&STANDARD).find(|(key, _)| key == id).map_or(*id, |(_, name)| *name).into(),
            Self::Named(name) => name.clone(),
        }
    }

    /// The cell text of `row`; an absent field or assignment is empty.
    fn cell<'a>(&self, row: &'a ComponentRow) -> &'a str {
        match self {
            Self::Reference => &row.reference,
            Self::Value => &row.value,
            Self::Footprint => row.footprint.as_ref().map_or("", |footprint| footprint.name.as_str()),
            Self::Standard(id) => row.fields.get(*id).map_or("", |field| field.value.as_str()),
            Self::Named(name) => named(row, name).map_or("", |(_, field)| field.value.as_str()),
        }
    }
}

/// The custom field of `row` with this name, compared without case.
fn named<'a>(row: &'a ComponentRow, name: &str) -> Option<(&'a String, &'a fields::Field)> {
    row.fields.iter().find(|(id, field)| !STANDARD.iter().any(|(key, _)| key == id) && field.name.eq_ignore_ascii_case(name))
}

/// Fixed columns, then the description when any part has one, then every custom field name in
/// natural order. Columns follow the data, never the display order.
fn columns(rows: &[ComponentRow]) -> Vec<Column> {
    let mut columns = vec![Column::Reference, Column::Value, Column::Footprint];
    columns.extend(FIXED_FIELDS.iter().map(|(id, _)| Column::Standard(id)));
    if rows.iter().any(|row| row.fields.contains_key("description")) {
        columns.push(Column::Standard("description"));
    }
    let mut names = BTreeMap::new();
    for (id, field) in rows.iter().flat_map(|row| &row.fields) {
        if !STANDARD.iter().any(|(key, _)| key == id) {
            names.entry(field.name.to_lowercase()).or_insert_with(|| field.name.clone());
        }
    }
    let mut names: Vec<_> = names.into_values().collect();
    names.sort_by(|a, b| pcbcraft_engine::authoring::natural_cmp(a, b));
    columns.extend(names.into_iter().map(Column::Named));
    columns
}

/// A dry-run reply held for review, with the exact rows and revision Apply submits.
#[derive(Debug)]
struct Preview {
    edits: Value,
    revision: u64,
    changes: Vec<Value>,
}

/// Table state owned by the open dialog. Filter, sort and selection never touch the project.
#[derive(Debug, Default)]
pub(super) struct Table {
    filter: String,
    sort: ColumnSort,
    selected: BTreeSet<String>,
    anchor: Option<String>,
    field: String,
    value: String,
    footprint: String,
    preview: Option<Preview>,
    error: Option<String>,
}

/// Whether any shown cell contains every space-separated word of `filter`, ignoring case.
fn matches(row: &ComponentRow, columns: &[Column], filter: &str) -> bool {
    filter.split_whitespace().all(|word| {
        let word = word.to_lowercase();
        columns.iter().any(|column| column.cell(row).to_lowercase().contains(&word))
    })
}

/// A footprint choice's key in the edit dropdown.
fn footprint_key(library: &str, id: &str, revision: u32) -> String {
    format!("{library}/{id}/{revision}")
}

/// Readable text of one changed field for the preview: a field's value, a package's name or
/// address, or "(none)" when absent.
fn change_text(value: &Value) -> String {
    match value {
        Value::Null => "(none)".into(),
        Value::String(text) => text.clone(),
        Value::Object(object) if object.contains_key("definition") => format!(
            "{} / {} r{}",
            object.get("library").and_then(Value::as_str).unwrap_or_default(),
            object.get("definition").and_then(Value::as_str).unwrap_or_default(),
            object.get("definition_revision").map(Value::to_string).unwrap_or_default()
        ),
        Value::Object(object) => object.get("value").and_then(Value::as_str).unwrap_or_default().to_owned(),
        other => other.to_string(),
    }
}

/// The preview's name for a changed field: its own name when it carries one.
fn change_field(change: &Value) -> String {
    let field = change["field"].as_str().unwrap_or_default();
    let named = [&change["new"], &change["old"]].into_iter().find_map(|value| value.get("name").and_then(Value::as_str));
    match (field, named) {
        ("reference", _) => "Reference".into(),
        ("value", _) => "Value".into(),
        ("footprint", _) => "Footprint".into(),
        (_, Some(name)) => name.into(),
        (id, None) => id.into(),
    }
}

impl Table {
    /// Build one `part.set_fields` row per target that sets the chosen column to the typed value.
    /// An empty value removes a field; a footprint takes the catalog default mapping, or matching
    /// terminal and pad numbers, and the engine judges whether that mapping fits.
    fn edits(&self, project: &Project, rows: &[&ComponentRow], column: &Column) -> Result<Vec<Value>, String> {
        let value = self.value.trim();
        let footprint = match column {
            Column::Footprint => Some(
                project
                    .footprint_catalog()
                    .into_iter()
                    .find(|d| footprint_key(&d.library, &d.id, d.revision) == self.footprint)
                    .ok_or("Choose a footprint to assign.")?,
            ),
            _ => None,
        };
        rows.iter()
            .map(|row| {
                let mut edit = json!({"component": row.component});
                match column {
                    Column::Reference => edit["reference"] = json!(value),
                    Column::Value => edit["value"] = json!(value),
                    Column::Footprint => {
                        let definition = footprint.as_ref().ok_or("Choose a footprint to assign.")?;
                        let mapping = project.default_footprint_mapping(&row.component, definition).unwrap_or_else(|| {
                            project
                                .components()
                                .find(|part| part.id() == row.component)
                                .map(|part| {
                                    part.definition.pins().iter().map(|pin| (pin.number.clone(), PadIds::from(pin.number.as_str()))).collect()
                                })
                                .unwrap_or_default()
                        });
                        edit["footprint"] = json!({"library":definition.library,"definition":definition.id,
                            "definition_revision":definition.revision,"mapping":mapping});
                    }
                    Column::Standard(id) => {
                        let name = row
                            .fields
                            .get(*id)
                            .map_or_else(|| STANDARD.iter().find(|(key, _)| key == id).map_or(*id, |(_, n)| *n).to_owned(), |f| f.name.clone());
                        let id = (*id).to_owned();
                        edit["fields"] = json!({id: (!value.is_empty()).then(|| json!({"name": name, "value": value}))});
                    }
                    Column::Named(name) => match named(row, name) {
                        Some((id, field)) => edit["fields"] = json!({id: (!value.is_empty()).then(|| json!({"name": field.name, "value": value}))}),
                        None if !value.is_empty() => {
                            let id = fields::new_id()?;
                            edit["fields"] = json!({id: {"name": name, "value": value}});
                        }
                        None => {}
                    },
                }
                Ok(edit)
            })
            .collect()
    }

    /// Apply a row click: plain selects one row, Ctrl/Command toggles it, and Shift selects the
    /// shown rows between the last clicked row and this one.
    fn click(&mut self, shown: &[&ComponentRow], index: usize, modifiers: egui::Modifiers) {
        let Some(row) = shown.get(index) else { return };
        let key = row.component.clone();
        let anchor = self.anchor.as_ref().and_then(|anchor| shown.iter().position(|row| &row.component == anchor));
        if let (true, Some(anchor)) = (modifiers.shift, anchor) {
            self.selected = shown[anchor.min(index)..=anchor.max(index)].iter().map(|row| row.component.clone()).collect();
            return;
        }
        if modifiers.command {
            if !self.selected.remove(&key) {
                self.selected.insert(key.clone());
            }
        } else {
            self.selected = BTreeSet::from([key.clone()]);
        }
        self.anchor = Some(key);
    }
}

/// Name the component a refusal starts with by its reference; the engine names it by its id.
fn refusal(error: &str, rows: &[ComponentRow]) -> String {
    let named =
        error.split_once(": ").and_then(|(id, rest)| rows.iter().find(|row| row.component == id).map(|row| format!("{}: {rest}", row.reference)));
    named.unwrap_or_else(|| error.to_owned())
}

impl PcbApp {
    /// Open the table over the current project; nothing is captured until a preview.
    pub(super) fn open_component_table(&mut self) {
        if self.session.project().is_some() {
            self.dialog = Dialog::ComponentTable(Box::default());
        }
    }

    /// Draw the table and its grouped edit. A preview is a dry run of the exact rows Apply sends;
    /// Apply carries the preview's revision, so an edit made elsewhere since is refused, and the
    /// engine's refusals show as they are. Rows the filter hides are never edited.
    pub(super) fn component_table_dialog(&mut self, ctx: &egui::Context, mut table: Box<Table>) {
        let Some(project) = self.session.snapshot() else { return };
        let rows = project.component_rows();
        let columns = columns(&rows);
        let mut shown: Vec<&ComponentRow> = rows.iter().filter(|row| matches(row, &columns, &table.filter)).collect();
        table.sort.apply(&mut shown, |row, column| columns.get(column).map_or("", |c| c.cell(row)));
        let targets: Vec<&ComponentRow> = shown.iter().copied().filter(|row| table.selected.contains(&row.component)).collect();
        let headers: Vec<String> = columns.iter().map(Column::label).collect();
        let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let entries: Vec<TableEntry> = shown
            .iter()
            .map(|row| TableEntry {
                key: row.component.clone(),
                cells: columns.iter().map(|column| column.cell(row).to_owned()).collect(),
                selected: table.selected.contains(&row.component),
                branch: None,
                nested: false,
            })
            .collect();
        if !headers.contains(&table.field) {
            table.field = "Value".into();
        }
        let (mut preview, mut apply, mut cancel, mut close) = (false, false, false, false);
        let mut clicked = None;
        let closing = shell::dialog_width(ctx, "component_table", "Component fields", 1040., |ui| {
            controls::search(ui, ui.make_persistent_id("component_table_filter"), &mut table.filter, "Filter components", "Filter components");
            ui.label(format!("{} components · {} shown · {} selected", rows.len(), shown.len(), targets.len()));
            if let Some(index) =
                tables::sortable_table(ui, "component_table", &header_refs, &mut table.sort, &entries, "No matching components", 360.)
            {
                clicked = Some((index, ui.input(|i| i.modifiers)));
            }
            let options: Vec<(&str, String)> = header_refs.iter().map(|label| (*label, (*label).to_owned())).collect();
            properties::choice(ui, "Field", "component_table_field", &mut table.field, &options);
            if table.field == "Footprint" {
                let catalog = project.footprint_catalog();
                let keys: Vec<String> = catalog.iter().map(|d| footprint_key(&d.library, &d.id, d.revision)).collect();
                let footprints: Vec<(&str, String)> = catalog
                    .iter()
                    .zip(&keys)
                    .map(|(d, key)| (key.as_str(), format!("{} · {} / {} r{}", d.name, d.library, d.id, d.revision)))
                    .collect();
                properties::choice(ui, "Footprint", "component_table_footprint", &mut table.footprint, &footprints);
            } else {
                ui.horizontal(|ui| {
                    let label = ui.label("New value");
                    controls::text(ui, &mut table.value).labelled_by(label.id);
                });
            }
            preview = controls::enabled_button(ui, !targets.is_empty(), format!("Preview change to {} selected", targets.len())).clicked();
            if let Some(error) = &table.error {
                ui.label(shell::error(refusal(error, &rows)));
            }
            if let Some(review) = &table.preview {
                if review.changes.is_empty() {
                    ui.label("The selected components already have this value.");
                } else {
                    let changes: Vec<TableEntry> = review
                        .changes
                        .iter()
                        .enumerate()
                        .map(|(index, change)| TableEntry {
                            key: index.to_string(),
                            cells: vec![
                                change["reference"].as_str().unwrap_or_default().to_owned(),
                                change_field(change),
                                change_text(&change["old"]),
                                change_text(&change["new"]),
                            ],
                            selected: false,
                            branch: None,
                            nested: false,
                        })
                        .collect();
                    ui.label(format!("{} field changes on {} components", changes.len(), review.edits.as_array().map_or(0, Vec::len)));
                    ui.allocate_ui(egui::vec2(ui.available_width(), 180.), |ui| {
                        tables::table(ui, &["Component", "Field", "Old", "New"], &changes, |_| {})
                    });
                }
                ui.horizontal(|ui| {
                    apply = controls::enabled_button(ui, !review.changes.is_empty(), "Apply").clicked();
                    cancel = controls::button(ui, "Cancel").clicked();
                });
            }
            close = controls::button(ui, "Close").clicked();
        });
        if cancel {
            table.preview = None;
        }
        if let Some((index, modifiers)) = clicked {
            table.click(&shown, index, modifiers);
        }
        if preview {
            table.preview = None;
            table.error = None;
            let column = columns.iter().find(|column| column.label() == table.field).cloned().unwrap_or(Column::Value);
            match table.edits(&project, &targets, &column) {
                Ok(edits) => {
                    let revision = self.session.revision();
                    let params = json!({"edits": edits, "dry_run": true, "expected_revision": revision});
                    match self.session.execute(SET_FIELDS, &params) {
                        Ok(reply) => {
                            let changes = reply["changes"].as_array().cloned().unwrap_or_default();
                            table.preview = Some(Preview { edits: params["edits"].clone(), revision, changes });
                        }
                        Err(error) => table.error = Some(error.to_string()),
                    }
                }
                Err(error) => table.error = Some(error),
            }
        }
        if apply && let Some(review) = table.preview.take() {
            self.run(SET_FIELDS, &json!({"edits": review.edits, "expected_revision": review.revision}));
            table.error = self.message.take();
        }
        if !close && !closing {
            self.dialog = Dialog::ComponentTable(table);
        }
    }
}
