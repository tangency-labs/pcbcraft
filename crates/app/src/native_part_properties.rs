//! Native part selection forms capture view authority and submit one shared engine transaction.
use crate::{Dialog, PcbApp, ViewInput};
use pcbcraft_chrome::{properties, shell};
use pcbcraft_engine::part_instances::{SET_PART, Selection};
use serde_json::json;
use std::collections::BTreeMap;

use crate::requirement_input::Draft;

/// A part form owns no source writes, board geometry or project mutation before Apply.
#[derive(Debug)]
pub(super) struct Edit {
    pub(super) source: ViewInput,
    pub(super) update_review: Option<Box<crate::board_update::Edit>>,
    component: String,
    selection: Selection,
    original_manufacturer: Option<pcbcraft_engine::library::parts::Reference>,
    drafts: BTreeMap<String, Draft>,
    error: Option<String>,
}
impl Edit {
    /// Parse a full candidate before sending any field to the shared engine.
    fn candidate(&self) -> Result<Selection, String> {
        let mut selection = self.selection.clone();
        selection.requirements.clear();
        if let Some(category) = &selection.resolved.category {
            for (id, draft) in &self.drafts {
                if selection.resolved.definition.fixed_parameters.contains_key(id) {
                    continue;
                }
                let field = category.fields.get(id).ok_or("Unknown category field.")?;
                if let Some(requirement) = draft.requirement(field).map_err(|error| format!("{}: {error}", field.name))? {
                    selection.requirements.insert(id.clone(), requirement);
                }
            }
        }
        selection.validate()?;
        Ok(selection)
    }
}
impl PcbApp {
    /// Capture the explicitly selected part and source view; later source or project changes cannot retarget it.
    pub(super) fn begin_native_part_properties(&mut self, component: String) {
        let Some(source) = self.views.active.input(&self.session) else {
            return;
        };
        let Some(part) =
            self.session.project().and_then(|project| project.components().find(|part| part.id() == component)).and_then(|part| part.part.clone())
        else {
            return;
        };
        let effective = part.effective_requirements();
        let drafts = part
            .resolved
            .category
            .as_ref()
            .map(|category| category.fields.keys().map(|id| (id.clone(), Draft::new(effective.get(id)))).collect())
            .unwrap_or_default();
        let original_manufacturer = part.manufacturer.as_ref().map(|record| record.reference());
        self.dialog = Dialog::NativePart(Box::new(Edit {
            source,
            update_review: None,
            component,
            selection: part,
            original_manufacturer,
            drafts,
            error: None,
        }));
    }
    /// Package, MPN and all typed requirement edits commit once; cancellation and errors preserve the snapshot.
    /// When opened by a board review, Cancel restores it and Apply refreshes its invalidated plan.
    pub(super) fn native_part_properties_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        let reference = self
            .session
            .project()
            .and_then(|project| project.components().find(|part| part.id() == edit.component))
            .map(|part| part.reference().to_owned())
            .unwrap_or_default();
        let action = pcbcraft_chrome::catalog::property_form(ctx, "native_part_selection", "Part selection", 760.0, "Apply", |ui| {
            properties::identity(ui, &reference, &edit.selection.resolved.definition.name);
            properties::field(ui, "Package", |ui| {
                let label = edit
                    .selection
                    .package
                    .as_ref()
                    .and_then(|id| edit.selection.resolved.definition.packages.get(id))
                    .map_or("Unassigned", |package| package.name.as_str());
                egui::ComboBox::from_id_salt("native_package")
                    .width(ui.available_width())
                    .truncate()
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut edit.selection.package, None, "Unassigned");
                        for (id, package) in &edit.selection.resolved.definition.packages {
                            ui.selectable_value(&mut edit.selection.package, Some(id.clone()), &package.name);
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Package"));
            });
            properties::field(ui, "Manufacturer part", |ui| {
                let label = edit
                    .selection
                    .manufacturer
                    .as_ref()
                    .map(|record| format!("{} · {}", record.manufacturer, record.mpn))
                    .unwrap_or_else(|| "Generic".into());
                let mut records = self.session.libraries().manufacturers_for(&edit.selection.resolved.definition.reference());
                if let Some(current) = &edit.selection.manufacturer {
                    records.retain(|record| record.reference() != current.reference());
                    records.push(current.clone());
                }
                egui::ComboBox::from_id_salt("native_manufacturer")
                    .width(ui.available_width())
                    .truncate()
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(edit.selection.manufacturer.is_none(), "Generic").clicked() {
                            edit.selection.manufacturer = None;
                        }
                        for record in records {
                            let selected = edit.selection.manufacturer.as_ref().is_some_and(|current| current.reference() == record.reference());
                            if ui.selectable_label(selected, format!("{} · {}", record.manufacturer, record.mpn)).clicked() {
                                edit.selection.package = Some(record.package.clone());
                                edit.selection.manufacturer = Some(record);
                            }
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Manufacturer part"));
            });
            if let Some(category) = &edit.selection.resolved.category {
                properties::section(ui, "Design requirements");
                for (id, field) in &category.fields {
                    if let Some(draft) = edit.drafts.get_mut(id) {
                        ui.push_id(id, |ui| {
                            let fixed = edit.selection.resolved.definition.fixed_parameters.contains_key(id);
                            ui.add_enabled_ui(!fixed, |ui| {
                                draft.show(ui, &if fixed { format!("{} (fixed)", field.name) } else { field.name.clone() })
                            });
                        });
                    }
                }
            }
            if let Some(manufacturer) = &edit.selection.manufacturer {
                properties::section(ui, "Manufacturer specifications");
                for (id, value) in &manufacturer.specifications {
                    let name = edit
                        .selection
                        .resolved
                        .category
                        .as_ref()
                        .and_then(|category| category.fields.get(id))
                        .map_or(id.as_str(), |field| field.name.as_str());
                    properties::field(ui, name, |ui| {
                        ui.label(value.display());
                    });
                }
                if let Some(datasheet) = &manufacturer.datasheet {
                    properties::field(ui, "Datasheet", |ui| {
                        ui.hyperlink(datasheet);
                    });
                }
                if edit.candidate().is_ok_and(|candidate| candidate.compatibility()["status"] == "does_not_meet_requirements") {
                    ui.label(shell::error("The selected manufacturer part does not meet the current requirements."));
                }
            }
            if let Some(error) = &edit.error {
                ui.label(shell::error(error));
            }
        });
        if action.primary {
            let result = edit.candidate().and_then(|selection| {
                if !self.views.active.accepts(edit.source, &self.session) {
                    return Err("View or project changed; reopen part selection.".into());
                }
                let mut params = json!({
                    "component": edit.component,
                    "package": selection.package,
                    "requirements": selection.requirements,
                    "expected_revision": edit.source.revision
                });
                let manufacturer = selection.manufacturer.as_ref().map(|record| record.reference());
                if manufacturer != edit.original_manufacturer {
                    params["manufacturer"] = json!(manufacturer);
                    if let Some(record) = &selection.manufacturer {
                        params["manufacturer_digest"] = json!(pcbcraft_engine::library::sources::manufacturer_digest(record));
                    }
                }
                self.session.apply(SET_PART, &params).map_err(|error| error.to_string())?;
                Ok(())
            });
            match result {
                Ok(()) => {
                    self.views.sync(&self.session);
                    self.dialog = Dialog::None;
                    if let Some(mut review) = edit.update_review.take() {
                        review.invalidate_review();
                        self.resume_board_update(review);
                    }
                    self.repaint_editors(ctx);
                }
                Err(error) => {
                    edit.error = Some(error);
                    self.dialog = Dialog::NativePart(Box::new(edit));
                }
            }
        } else if action.close {
            self.dialog = Dialog::None;
            if let Some(review) = edit.update_review.take() {
                self.resume_board_update(review);
            }
            self.repaint_editors(ctx);
        } else {
            self.dialog = Dialog::NativePart(Box::new(edit));
        }
    }
}
