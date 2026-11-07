//! Revision-bound package selection works in either editor; assignment never creates geometry.
use crate::{Dialog, PcbApp, ViewInput, Workspace};
use pcbcraft_chrome::{shell, units};
use pcbcraft_engine::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT, board::definition::FootprintDefinition};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug)]
pub(super) struct Edit {
    pub(super) source: ViewInput,
    component: String,
    definition: Option<Arc<FootprintDefinition>>,
    mapping: BTreeMap<String, String>,
    query: String,
    x: String,
    y: String,
    units: units::Unit,
    error: Option<String>,
}

impl Edit {
    /// Restore an exact pinned choice; unassigned components start with an empty explicit map.
    fn select_component(&mut self, project: &pcbcraft_engine::Project) {
        self.definition = None;
        self.mapping.clear();
        if let Some(a) = project.footprint_assignments().into_iter().find(|a| a["component"] == self.component) {
            self.definition = project
                .footprint_catalog()
                .into_iter()
                .find(|d| a["library"] == d.library && a["definition"] == d.id && a["definition_revision"] == d.revision);
            self.mapping = serde_json::from_value(a["mapping"].clone()).unwrap_or_default();
        }
        self.error = None;
    }
}

impl PcbApp {
    /// Capture the originating view/project/revision. A schematic action supplies its component;
    /// the PCB entry point offers all project components, including those not yet placed.
    pub(super) fn begin_footprint_assignment(&mut self, component: Option<String>) {
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(project) = self.session.project() else { return };
        let Some(component) = component.or_else(|| project.components().next().map(|c| c.id())) else { return };
        let mut edit = Edit {
            source,
            component,
            definition: None,
            mapping: BTreeMap::new(),
            query: String::new(),
            x: "0".into(),
            y: "0".into(),
            units: self.preferences.units,
            error: None,
        };
        edit.select_component(project);
        self.dialog = Dialog::Footprint(edit);
    }

    /// Show package provenance and an explicit terminal map. Assign and Place are separate undoable
    /// commands; a stale form closes before dispatch. Already placed packages stay read-only until
    /// reviewed remapping exists. Source/library files are never edited by this dialog.
    pub(super) fn footprint_assignment_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        if !self.views.view(edit.source.view()).is_some_and(|v| v.input(&self.session) == Some(edit.source)) {
            self.message = Some("View or project changed; reopen footprint assignment to continue.".into());
            return;
        }
        let Some(project) = self.session.snapshot() else { return };
        let mut assign = false;
        let mut place = false;
        let mut cancel = false;
        let closing = shell::scrolling_dialog(ctx, "footprint_assignment", "Assign Footprint", |ui| {
            let previous = edit.component.clone();
            let component = project.components().find(|c| c.id() == edit.component);
            egui::ComboBox::from_label("Component")
                .selected_text(component.map_or("Choose component".into(), |c| format!("{} · {}", c.reference(), c.value())))
                .show_ui(ui, |ui| {
                    for c in project.components() {
                        ui.selectable_value(&mut edit.component, c.id(), format!("{} · {}", c.reference(), c.value()));
                    }
                });
            if previous != edit.component {
                edit.select_component(&project);
            }
            let assigned = project.footprint_assignments().into_iter().find(|a| a["component"] == edit.component);
            let placed = project.board_view(project.board_target()).is_some_and(|board| board.footprints().any(|f| f.component == edit.component));
            ui.label(if placed {
                "Placed"
            } else if assigned.is_some() {
                "Assigned · not placed"
            } else {
                "Unassigned"
            });
            ui.add_enabled_ui(!placed, |ui| {
                let label = ui.label("Search footprints");
                ui.add(egui::TextEdit::singleline(&mut edit.query)).labelled_by(label.id);
                let old = edit.definition.as_ref().map(|d| (d.library.clone(), d.id.clone(), d.revision));
                egui::ComboBox::from_label("Package")
                    .selected_text(edit.definition.as_ref().map_or("Choose footprint".into(), |d| format!("{} · r{}", d.name, d.revision)))
                    .show_ui(ui, |ui| {
                        for d in project.footprint_catalog() {
                            let text = format!("{} / {} · r{}", d.library, d.name, d.revision);
                            if !text.to_lowercase().contains(&edit.query.to_lowercase()) && !d.id.contains(&edit.query.to_lowercase()) {
                                continue;
                            }
                            if ui.selectable_label(edit.definition.as_ref().is_some_and(|current| current == &d), text).clicked() {
                                edit.definition = Some(d);
                            }
                        }
                    });
                if old != edit.definition.as_ref().map(|d| (d.library.clone(), d.id.clone(), d.revision)) {
                    edit.mapping.clear();
                }
            });
            if let Some(definition) = &edit.definition {
                ui.label(format!("{} / {} · revision {}", definition.library, definition.id, definition.revision));
                ui.label(&definition.license);
                ui.add(egui::Label::new(&definition.source.reference).truncate()).on_hover_text(&definition.source.reference);
                if let Some(component) = project.components().find(|c| c.id() == edit.component) {
                    ui.add_enabled_ui(!placed, |ui| {
                        for pin in component.definition.pins() {
                            let mapped = edit.mapping.entry(pin.number.clone()).or_default();
                            egui::ComboBox::from_label(format!("Terminal {}", pin.number))
                                .selected_text(if mapped.is_empty() { "Choose pad".into() } else { format!("Pad {mapped}") })
                                .show_ui(ui, |ui| {
                                    for pad in &definition.pads {
                                        ui.selectable_value(mapped, pad.number.clone(), format!("Pad {}", pad.number));
                                    }
                                });
                        }
                    });
                }
            }
            if placed {
                ui.label("Changing a placed package requires a reviewed board update.");
            }
            if let Some(error) = &edit.error {
                ui.label(shell::error(error.as_str()));
            }
            ui.horizontal(|ui| {
                assign = ui.add_enabled(!placed && edit.definition.is_some(), egui::Button::new("Assign")).clicked();
                cancel = ui.button("Close").clicked();
            });
            if self.views.active.workspace == Workspace::Board && assigned.is_some() && !placed {
                ui.separator();
                ui.horizontal(|ui| {
                    let label = ui.label(format!("X ({})", edit.units.symbol()));
                    ui.text_edit_singleline(&mut edit.x).labelled_by(label.id);
                });
                ui.horizontal(|ui| {
                    let label = ui.label(format!("Y ({})", edit.units.symbol()));
                    ui.text_edit_singleline(&mut edit.y).labelled_by(label.id);
                });
                let unchanged = assigned.as_ref().is_some_and(|a| {
                    edit.definition.as_ref().is_some_and(|d| {
                        a["library"] == d.library
                            && a["definition"] == d.id
                            && a["definition_revision"] == d.revision
                            && a["mapping"] == json!(edit.mapping)
                    })
                });
                place = ui.add_enabled(unchanged, egui::Button::new("Place on board")).clicked();
            }
        });
        if assign || place {
            let mut params = json!({"board": project.board_target().board,"component":edit.component,"expected_revision":edit.source.revision});
            let command = if assign {
                let Some(d) = &edit.definition else { return };
                params["library"] = json!(d.library);
                params["definition"] = json!(d.id);
                params["definition_revision"] = json!(d.revision);
                params["mapping"] = json!(edit.mapping);
                ASSIGN_FOOTPRINT
            } else {
                match (units::parse_length(&edit.x, edit.units), units::parse_length(&edit.y, edit.units)) {
                    (Ok(x), Ok(y)) => {
                        params["x_nm"] = json!(x);
                        params["y_nm"] = json!(y);
                    }
                    _ => {
                        edit.error = Some("Enter valid X and Y lengths.".into());
                        self.dialog = Dialog::Footprint(edit);
                        return;
                    }
                }
                PLACE_ASSIGNED_FOOTPRINT
            };
            match self.session.apply(command, &params) {
                Ok(_) => {
                    self.message = None;
                    self.views.sync(&self.session);
                    if place
                        && let Some(footprint) = self
                            .session
                            .project()
                            .and_then(|p| p.board_view(p.board_target()))
                            .and_then(|board| board.footprints().find(|f| f.component == edit.component))
                    {
                        self.views.active.camera.request_center(footprint.position);
                        self.views.select_board(std::collections::BTreeSet::from([footprint.id]), &self.session, self.preferences.selection_follow);
                    }
                    self.repaint_editors(ctx);
                    if assign && let Some(source) = self.views.view(edit.source.view()).and_then(|v| v.input(&self.session)) {
                        edit.source = source;
                        edit.error = None;
                        self.dialog = Dialog::Footprint(edit);
                    }
                }
                Err(error) => {
                    edit.error = Some(error.to_string());
                    self.dialog = Dialog::Footprint(edit);
                }
            }
        } else if !(closing || cancel) {
            self.dialog = Dialog::Footprint(edit);
        }
    }
}
