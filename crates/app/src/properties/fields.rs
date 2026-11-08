//! Editable component metadata and physical field appearance share the property transaction.
use super::*;
use pcbcraft_engine::schematic::{
    TextAlign,
    fields::{self, Field, Layout, Layouts, Metadata},
};
use std::collections::BTreeMap;

/// Text drafts preserve invalid physical input until corrected or cancelled.
#[derive(Clone, Debug)]
pub(super) struct Appearance {
    layout: Layout,
    automatic: bool,
    x: String,
    y: String,
    height: String,
    original: Layout,
}

impl Appearance {
    /// Capture an explicit layout or the field's automatic default in the active display unit.
    fn new(layout: Layout, unit: Unit) -> Self {
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
    fn read(&self, unit: Unit) -> Result<Layout, String> {
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
    pub(super) fn set_units(&mut self, from: Unit, to: Unit) {
        for value in [&mut self.x, &mut self.y, &mut self.height] {
            if let Ok(nm) = units::parse_length(value, from) {
                *value = units::edit_label(nm, to);
            }
        }
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
    pub(super) fn metadata_fields(&mut self, ui: &mut Ui) {
        chrome::text(ui, "Reference", &mut self.reference);
        chrome::text(ui, "Value", &mut self.value);
        for (id, name) in fields::STANDARD {
            let mut value = self.metadata.fields.get(id).map(|f| f.value.clone()).unwrap_or_default();
            if chrome::text(ui, name, &mut value).changed() {
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
                chrome::text(ui, "Field value", &mut field.value);
                if ui.button("Remove field").clicked() {
                    remove = Some(id.clone());
                }
            });
        }
        if let Some(id) = remove {
            self.metadata.fields.remove(&id);
            self.appearance.remove(&id);
        }
        if ui.add_enabled(self.metadata.fields.len() < 64, egui::Button::new("Add field")).clicked() {
            let id = match fields::new_id() {
                Ok(id) => id,
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            };
            let name =
                (1..=65).map(|n| format!("Field {n}")).find(|name| !self.metadata.fields.values().any(|f| f.name == *name)).unwrap_or_default();
            self.metadata.fields.insert(id.clone(), Field { name, value: String::new() });
            self.appearance.insert(id, Appearance::new(Layout { visible: false, ..Layout::default() }, self.units));
        }
    }

    /// Each field's controls edit the same draft as canvas movement, with offsets in symbol space.
    pub(super) fn field_appearance(&mut self, ui: &mut Ui) {
        let names: Vec<_> = [("reference".to_owned(), "Reference".to_owned()), ("value".to_owned(), "Value".to_owned())]
            .into_iter()
            .chain(self.metadata.fields.iter().map(|(id, f)| (id.clone(), f.name.clone())))
            .collect();
        for (id, name) in names {
            let Some(draft) = self.appearance.get_mut(&id) else {
                continue;
            };
            ui.push_id(&id, |ui| {
                egui::CollapsingHeader::new(&name).id_salt("field_appearance").show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut draft.layout.visible, "Show");
                        ui.checkbox(&mut draft.layout.show_name, "Show name");
                        ui.checkbox(&mut draft.automatic, "Automatic position");
                    });
                    if !draft.automatic {
                        chrome::text(ui, "Text X", &mut draft.x);
                        chrome::text(ui, "Text Y", &mut draft.y);
                    }
                    chrome::text(ui, "Text height", &mut draft.height);
                    chrome::rotation(ui, &mut draft.layout.rotation);
                    chrome::field(ui, "Align", |ui| {
                        egui::ComboBox::from_id_salt("text_align")
                            .selected_text(match draft.layout.align {
                                TextAlign::Start => "Left",
                                TextAlign::Middle => "Center",
                                TextAlign::End => "Right",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut draft.layout.align, TextAlign::Start, "Left");
                                ui.selectable_value(&mut draft.layout.align, TextAlign::Middle, "Center");
                                ui.selectable_value(&mut draft.layout.align, TextAlign::End, "Right");
                            });
                    });
                    if ui.button("Reset appearance").clicked() {
                        *draft = Appearance::new(Layout { visible: matches!(id.as_str(), "reference" | "value"), ..Layout::default() }, self.units);
                        // Keep reset distinguishable from the original explicit appearance.
                        draft.original = self.original_layout.get(&id).cloned().unwrap_or_else(|| draft.original.clone());
                    }
                });
            });
        }
    }

    /// Attributes store component intent independently of displayed text; consumers inspect the
    /// same flags through the engine. Unsupported exporters do not fabricate output from them.
    pub(super) fn attributes(&mut self, ui: &mut Ui) {
        let a = &mut self.metadata.attributes;
        ui.checkbox(&mut a.do_not_populate, "Do not populate");
        ui.checkbox(&mut a.exclude_from_bom, "Exclude from bill of materials");
        ui.checkbox(&mut a.exclude_from_board, "Exclude from board");
        ui.checkbox(&mut a.exclude_from_simulation, "Exclude from simulation");
        ui.checkbox(&mut a.exclude_from_position_files, "Exclude from position files");
    }
}
