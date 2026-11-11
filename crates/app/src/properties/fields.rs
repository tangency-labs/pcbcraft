//! Editable component metadata and physical field appearance share the property transaction.
use super::*;
use pcbcraft_engine::schematic::{
    TextAlign,
    fields::{self, Field, Layout, Layouts, Metadata},
};
use std::collections::BTreeMap;

/// Text drafts preserve invalid physical input until corrected or cancelled.
#[derive(Clone, Debug)]
pub(crate) struct Appearance {
    layout: Layout,
    automatic: bool,
    x: String,
    y: String,
    height: String,
    original: Layout,
}

impl Appearance {
    /// Capture an explicit layout or the field's automatic default in the active display unit.
    pub(crate) fn new(layout: Layout, unit: Unit) -> Self {
        let point = layout.offset.unwrap_or(Point { x_nm: 0, y_nm: 0 });
        Self {
            automatic: layout.offset.is_none(),
            x: units::edit_label(point.x_nm, unit),
            y: units::edit_label(point.y_nm, unit),
            height: units::edit_label(layout.height_nm, unit),
            original: layout.clone(),
            layout,
        }
    }

    /// Parse complete physical values before changing any component field or placement.
    pub(crate) fn read(&self, unit: Unit) -> Result<Layout, String> {
        let mut layout = self.layout.clone();
        layout.offset = if self.automatic {
            None
        } else {
            Some(Point {
                x_nm: units::parse_length(&self.x, unit).map_err(|e| format!("Text X: {e}"))?,
                y_nm: units::parse_length(&self.y, unit).map_err(|e| format!("Text Y: {e}"))?,
            })
        };
        layout.height_nm = units::parse_length(&self.height, unit).map_err(|e| format!("Text height: {e}"))?;
        Ok(layout)
    }

    /// Reformat valid drafts without applying them or overwriting invalid input.
    pub(crate) fn set_units(&mut self, from: Unit, to: Unit) {
        for value in [&mut self.x, &mut self.y, &mut self.height] {
            if let Ok(nm) = units::parse_length(value, from) {
                *value = units::edit_label(nm, to);
            }
        }
    }
    /// Show the shared physical text controls; the caller owns default/reset policy.
    pub(crate) fn controls(&mut self, ui: &mut Ui, visibility: bool) -> bool {
        if visibility {
            pcbcraft_chrome::controls::checkbox(ui, &mut self.layout.visible, "Show net name");
        }
        ui.horizontal_wrapped(|ui| {
            pcbcraft_chrome::controls::checkbox(ui, &mut self.layout.show_name, "Show name");
            pcbcraft_chrome::controls::checkbox(ui, &mut self.automatic, "Automatic position");
        });
        if !self.automatic {
            chrome::text(ui, "Text X", &mut self.x);
            chrome::text(ui, "Text Y", &mut self.y);
        }
        chrome::text(ui, "Text height", &mut self.height);
        chrome::field(ui, "Text rotation", |ui| {
            egui::ComboBox::from_id_salt("field_rotation")
                .selected_text(format!("{}°", self.layout.rotation))
                .show_ui(ui, |ui| {
                    for rotation in [0, 90, 180, 270] {
                        ui.selectable_value(&mut self.layout.rotation, rotation, format!("{rotation}°"));
                    }
                })
                .response
                .accessible_name("Field rotation");
        });
        chrome::field(ui, "Align", |ui| {
            egui::ComboBox::from_id_salt("text_align")
                .selected_text(match self.layout.align {
                    TextAlign::Start => "Left",
                    TextAlign::Middle => "Center",
                    TextAlign::End => "Right",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.layout.align, TextAlign::Start, "Left");
                    ui.selectable_value(&mut self.layout.align, TextAlign::Middle, "Center");
                    ui.selectable_value(&mut self.layout.align, TextAlign::End, "Right");
                })
                .response
                .accessible_name("Field text alignment");
        });
        pcbcraft_chrome::controls::button(ui, "Reset appearance").clicked()
    }
}

/// Build editable appearance drafts without materializing absent automatic layouts in saves.
pub(super) fn appearances(metadata: &Metadata, layouts: &Layouts, unit: Unit) -> BTreeMap<String, Appearance> {
    ["reference", "value"]
        .into_iter()
        .chain(metadata.fields.keys().map(String::as_str))
        .map(|id| {
            let layout = layouts.get(id).cloned().unwrap_or_else(|| Layout { visible: matches!(id, "reference" | "value"), ..Layout::default() });
            (id.to_owned(), Appearance::new(layout, unit))
        })
        .collect()
}

impl Edit {
    /// Keep implicit defaults implicit; remove appearance only when its optional field is removed.
    pub(super) fn layouts(&self) -> Result<Layouts, String> {
        let mut layouts = self.original_layout.clone();
        layouts.retain(|id, _| fields::known(id, &self.metadata));
        for (id, draft) in &self.appearance {
            if !fields::known(id, &self.metadata) {
                continue;
            }
            let layout = draft.read(self.units)?;
            if layout != draft.original {
                layouts.insert(id.clone(), layout);
            }
        }
        fields::validate_layouts(&layouts, &self.metadata)?;
        Ok(layouts)
    }

    /// Standard optional fields remain absent until authored; custom fields have independent IDs.
    pub(super) fn metadata_fields(&mut self, ui: &mut Ui) -> bool {
        chrome::field_header(ui);
        let reference = row(ui, "Reference", &mut self.reference, self.appearance.get_mut("reference"), None);
        if !self.focused {
            reference.request_focus();
            self.focused = true;
        }
        let value = row(ui, "Value", &mut self.value, self.appearance.get_mut("value"), None);
        let enter = (reference.lost_focus() || value.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        for (id, name) in fields::STANDARD {
            let mut value = self.metadata.fields.get(id).map(|f| f.value.clone()).unwrap_or_default();
            if row(ui, name, &mut value, self.appearance.get_mut(id), (id == "datasheet").then_some(name)).changed() {
                if value.is_empty() {
                    self.metadata.fields.remove(id);
                    self.appearance.remove(id);
                } else {
                    self.metadata.fields.insert(id.into(), Field { name: name.into(), value });
                    self.appearance.entry(id.into()).or_insert_with(|| {
                        Appearance::new(
                            self.original_layout.get(id).cloned().unwrap_or_else(|| Layout { visible: false, ..Layout::default() }),
                            self.units,
                        )
                    });
                }
            }
        }
        let mut remove = None;
        for (id, field) in &mut self.metadata.fields {
            if fields::STANDARD.iter().any(|(key, _)| *key == id) {
                continue;
            }
            ui.push_id(id, |ui| {
                ui.separator();
                chrome::text(ui, "Field name", &mut field.name);
                row(ui, &field.name, &mut field.value, self.appearance.get_mut(id), Some(&field.name));
                if pcbcraft_chrome::controls::button(ui, "Remove field").clicked() {
                    remove = Some(id.clone());
                }
            });
        }
        if let Some(id) = remove {
            self.metadata.fields.remove(&id);
            self.appearance.remove(&id);
        }
        let mut added = None;
        ui.horizontal(|ui| {
            let enabled = self.metadata.fields.len() < 64;
            if pcbcraft_chrome::controls::enabled_button(ui, enabled, "Add field").clicked() {
                added = Some("Field");
            }
            if pcbcraft_chrome::controls::enabled_button(ui, enabled, "Add link").clicked() {
                added = Some("Link");
            }
        });
        if let Some(kind) = added {
            let id = match fields::new_id() {
                Ok(id) => id,
                Err(error) => {
                    self.error = Some(error);
                    return false;
                }
            };
            let name = (1..=65)
                .map(|n| format!("{kind} {n}"))
                .find(|name| !self.metadata.fields.values().any(|f| f.name.eq_ignore_ascii_case(name)))
                .unwrap_or_default();
            self.metadata.fields.insert(id.clone(), Field { name, value: String::new() });
            self.appearance.insert(id, Appearance::new(Layout { visible: false, ..Layout::default() }, self.units));
        }
        enter
    }

    /// Each field's controls edit the same draft as canvas movement, with offsets in symbol space.
    pub(super) fn field_appearance(&mut self, ui: &mut Ui) {
        let names: Vec<_> = [("reference".to_owned(), "Reference".to_owned()), ("value".to_owned(), "Value".to_owned())]
            .into_iter()
            .chain(self.metadata.fields.iter().map(|(id, f)| (id.clone(), f.name.clone())))
            .collect();
        if !names.iter().any(|(id, _)| id == &self.appearance_field) {
            self.appearance_field = "reference".into();
        }
        chrome::field(ui, "Field", |ui| {
            let current = names.iter().find(|(id, _)| id == &self.appearance_field).map(|(_, name)| name.as_str()).unwrap_or("Reference");
            egui::ComboBox::from_id_salt("appearance_field")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (id, name) in &names {
                        ui.selectable_value(&mut self.appearance_field, id.clone(), name);
                    }
                })
                .response
                .accessible_name("Field appearance target");
        });
        let Some(draft) = self.appearance.get_mut(&self.appearance_field) else { return };
        ui.push_id(&self.appearance_field, |ui| {
            if draft.controls(ui, false) {
                *draft = Appearance::new(
                    Layout { visible: matches!(self.appearance_field.as_str(), "reference" | "value"), ..Layout::default() },
                    self.units,
                );
                draft.original = self.original_layout.get(&self.appearance_field).cloned().unwrap_or_else(|| draft.original.clone());
            }
        });
    }

    /// Attributes store component intent independently of displayed text; consumers inspect the
    /// same flags through the engine. Unsupported exporters do not fabricate output from them.
    pub(super) fn attributes(&mut self, ui: &mut Ui) {
        let a = &mut self.metadata.attributes;
        pcbcraft_chrome::controls::checkbox(ui, &mut a.do_not_populate, "Do not populate");
        pcbcraft_chrome::controls::checkbox(ui, &mut a.exclude_from_bom, "Exclude from bill of materials");
        pcbcraft_chrome::controls::checkbox(ui, &mut a.exclude_from_board, "Exclude from board");
        pcbcraft_chrome::controls::checkbox(ui, &mut a.exclude_from_simulation, "Exclude from simulation");
        pcbcraft_chrome::controls::checkbox(ui, &mut a.exclude_from_position_files, "Exclude from pick-and-place files")
            .on_hover_text("Omit from assembly position exports. PCBcraft stores this setting; export generation is not implemented yet.");
    }
}

/// Field content and visibility share a row so routine editing does not require another page.
fn row(ui: &mut Ui, name: &str, value: &mut String, appearance: Option<&mut Appearance>, link: Option<&str>) -> egui::Response {
    chrome::field_row(ui, name, value, appearance.map(|appearance| &mut appearance.layout.visible), link)
}
