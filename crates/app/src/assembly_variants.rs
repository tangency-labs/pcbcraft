//! Board-local population variants share engine commands with MCP and export from one snapshot.
use crate::{Dialog, PcbApp};
use pcbcraft_chrome::{controls, shell};
use pcbcraft_engine::{CREATE_ASSEMBLY_VARIANT, DELETE_ASSEMBLY_VARIANT, EXPORT_ASSEMBLY, SET_ASSEMBLY_VARIANT};
use serde_json::{Value, json};
use uuid::Uuid;

/// The selected variant is local dialog state; opening or previewing it never authors a project edit.
#[derive(Debug)]
pub(super) struct Edit {
    project: Uuid,
    board: Uuid,
    opening: u64,
    selected: Option<Uuid>,
    name: String,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopulationChoice {
    Inherit,
    Fitted,
    Dnp,
}

impl PopulationChoice {
    /// Describe the choice without conflating inherited base state with an explicit override.
    fn label(self, base_fitted: bool) -> String {
        match self {
            Self::Inherit => format!("Inherit base ({})", if base_fitted { "fitted" } else { "DNP" }),
            Self::Fitted => "Fitted".into(),
            Self::Dnp => "DNP".into(),
        }
    }

    /// Translate presentation state to the command's sparse override contract.
    fn fitted(self) -> Option<bool> {
        match self {
            Self::Inherit => None,
            Self::Fitted => Some(true),
            Self::Dnp => Some(false),
        }
    }
}

#[derive(Debug)]
enum Action {
    None,
    Create,
    Rename(Uuid),
    Delete(Uuid),
    Population { variant: Uuid, component: String, fitted: Option<bool> },
    Export(Option<Uuid>),
}

impl PcbApp {
    /// Open assembly management for the project's explicit physical board.
    pub(super) fn begin_assembly_variants(&mut self) {
        if self.home || self.files.busy() || self.project_action_pending() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        let Some(project) = self.session.project() else { return };
        let Some(board) = project.board_target() else {
            self.message = Some("Create a physical board before managing assembly variants.".into());
            return;
        };
        self.message = None;
        self.dialog = Dialog::AssemblyVariants(Edit {
            project: project.id(),
            board: board.board,
            opening: self.session.history_root_revision(),
            selected: None,
            name: String::new(),
            error: None,
        });
    }

    /// Export Base BOM and placement files through the same read-only command used by MCP.
    pub(super) fn export_base_assembly(&mut self, ctx: &egui::Context) {
        self.export_assembly(ctx, None);
    }

    /// Keep every action tied to the same open project and board while allowing normal edits.
    fn context_matches(&self, edit: &Edit) -> bool {
        self.session.history_root_revision() == edit.opening
            && self
                .session
                .project()
                .is_some_and(|project| project.id() == edit.project && project.board_target().is_some_and(|board| board.board == edit.board))
            && !self.files.busy()
    }

    /// Draw shared modal chrome and dispatch every mutation through the engine command catalog.
    pub(super) fn assembly_variants_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        let current = self.context_matches(&edit);
        let previous_selection = edit.selected;
        let project = self.session.snapshot();
        if let Some(project) = project.as_deref() {
            if edit.selected.is_some_and(|selected| !project.assembly_variants().any(|variant| variant.id == selected)) {
                edit.selected = None;
                edit.name.clear();
            }
            if let Some(selected) = edit.selected
                && let Some(variant) = project.assembly_variants().find(|variant| variant.id == selected)
                && edit.name.is_empty()
            {
                edit.name = variant.name.clone();
            }
        }
        let selected_variant =
            project.as_deref().and_then(|project| edit.selected.and_then(|id| project.assembly_variants().find(|variant| variant.id == id)));
        let selected_label = selected_variant.map_or_else(|| "Base".to_owned(), |variant| variant.name.clone());
        let mut action = Action::None;
        let closing = shell::scrolling_dialog_width(ctx, "assembly_variants", "Assembly Variants", 720.0, |ui| {
            if !current {
                ui.label(shell::error("The project or board changed. Close this window and open Variants again."));
                return;
            }
            let Some(project) = project.as_deref() else {
                ui.label(shell::error("No project is open."));
                return;
            };
            ui.label("Base DNP settings are inherited until a variant overrides them. Selection here is temporary and does not dirty the project.");
            let choice = controls::Choice::from_label("Assembly").selected_text(selected_label.clone()).width(260.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut edit.selected, None, "Base");
                for variant in project.assembly_variants() {
                    ui.selectable_value(&mut edit.selected, Some(variant.id), variant.name.as_str());
                }
            });
            let _ = choice;
            ui.horizontal_wrapped(|ui| {
                ui.label("Variant name");
                controls::text(ui, &mut edit.name);
                if controls::enabled_button(ui, current && !edit.name.trim().is_empty(), "Create").clicked() {
                    action = Action::Create;
                }
                if let Some(variant) = selected_variant {
                    if controls::enabled_button(ui, current && !edit.name.trim().is_empty(), "Rename").clicked() {
                        action = Action::Rename(variant.id);
                    }
                    if controls::button(ui, "Delete variant").clicked() {
                        action = Action::Delete(variant.id);
                    }
                }
            });
            ui.separator();
            ui.strong(format!("Population for {selected_label}"));
            egui::ScrollArea::vertical().id_salt("assembly_population_rows").max_height(340.0).auto_shrink([false, false]).show(ui, |ui| {
                for component in project.components() {
                    let component_id = component.id();
                    let numeric_id = component_id.strip_prefix("component-").and_then(|id| id.parse::<u64>().ok());
                    let base_fitted = !component.metadata.attributes.do_not_populate;
                    let override_choice = selected_variant.and_then(|variant| numeric_id.and_then(|id| variant.population.get(&id))).copied();
                    let mut selected_choice = match override_choice {
                        Some(true) => PopulationChoice::Fitted,
                        Some(false) => PopulationChoice::Dnp,
                        None => PopulationChoice::Inherit,
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{} · {}", component.reference(), component.value()));
                        if component.metadata.attributes.exclude_from_board {
                            ui.label("Outside this board");
                        }
                        if let Some(variant) = selected_variant {
                            ui.add_enabled_ui(current && !component.metadata.attributes.exclude_from_board, |ui| {
                                let response = controls::Choice::from_id_salt(("assembly_component", component_id.clone()))
                                    .selected_text(selected_choice.label(base_fitted))
                                    .width(190.0)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut selected_choice,
                                            PopulationChoice::Inherit,
                                            PopulationChoice::Inherit.label(base_fitted),
                                        );
                                        ui.selectable_value(&mut selected_choice, PopulationChoice::Fitted, "Fitted");
                                        ui.selectable_value(&mut selected_choice, PopulationChoice::Dnp, "DNP");
                                    });
                                if response.response.changed() || selected_choice.fitted() != override_choice {
                                    action =
                                        Action::Population { variant: variant.id, component: component_id.clone(), fitted: selected_choice.fitted() };
                                }
                            });
                        } else {
                            ui.label(if base_fitted { "Fitted in Base" } else { "DNP in Base" });
                        }
                    });
                }
            });
            ui.label("DNP omits a part from BOM and placement outputs. BOM exclusion and placement-file exclusion remain independent; no population choice changes footprints, routing or copper.");
            ui.horizontal_wrapped(|ui| {
                if controls::button(ui, "Export BOM and placement").clicked() {
                    action = Action::Export(edit.selected);
                }
            });
            if let Some(error) = edit.error.as_deref() {
                ui.label(shell::error(error));
            }
        });
        if closing {
            return;
        }
        if edit.selected != previous_selection {
            edit.name = edit
                .selected
                .and_then(|id| project.as_deref().and_then(|project| project.assembly_variants().find(|variant| variant.id == id)))
                .map_or_else(String::new, |variant| variant.name.clone());
        }
        let result = match action {
            Action::None => {
                self.dialog = Dialog::AssemblyVariants(edit);
                return;
            }
            Action::Create => {
                let before = self
                    .session
                    .project()
                    .map(|project| project.assembly_variants().map(|variant| variant.id).collect::<std::collections::BTreeSet<_>>());
                let params = json!({"board":edit.board,"expected_revision":self.session.revision(),"name":edit.name});
                self.run_variant_command(CREATE_ASSEMBLY_VARIANT, &params).map(|_| {
                    edit.selected = self
                        .session
                        .project()
                        .and_then(|project| project.assembly_variants().find(|variant| !before.as_ref().is_some_and(|ids| ids.contains(&variant.id))))
                        .map(|variant| variant.id);
                    edit.name = edit
                        .selected
                        .and_then(|id| self.session.project().and_then(|project| project.assembly_variants().find(|variant| variant.id == id)))
                        .map_or_else(String::new, |variant| variant.name.clone());
                })
            }
            Action::Rename(variant) => {
                let params = json!({"board":edit.board,"expected_revision":self.session.revision(),"variant":variant,"name":edit.name});
                self.run_variant_command(SET_ASSEMBLY_VARIANT, &params).map(|_| ())
            }
            Action::Delete(variant) => {
                let params = json!({"board":edit.board,"expected_revision":self.session.revision(),"variant":variant});
                self.run_variant_command(DELETE_ASSEMBLY_VARIANT, &params).map(|_| {
                    edit.selected = None;
                    edit.name.clear();
                })
            }
            Action::Population { variant, component, fitted } => {
                let params =
                    json!({"board":edit.board,"expected_revision":self.session.revision(),"variant":variant,"component":component,"fitted":fitted});
                self.run_variant_command(SET_ASSEMBLY_VARIANT, &params).map(|_| ())
            }
            Action::Export(variant) => {
                self.export_assembly(ctx, variant);
                if let Some(error) = self.message.take() { Err(error) } else { Ok(()) }
            }
        };
        match result {
            Ok(()) => {
                edit.error = None;
                self.views.sync(&self.session);
                self.repaint_editors(ctx);
            }
            Err(error) => edit.error = Some(error),
        }
        self.dialog = Dialog::AssemblyVariants(edit);
    }

    /// Execute a catalogued command and keep errors in the current modal draft.
    fn run_variant_command(&mut self, command: &str, params: &Value) -> Result<(), String> {
        self.file_notice = None;
        self.message = None;
        self.session.execute(command, params).map_err(|error| error.to_string())?;
        self.views.sync(&self.session);
        Ok(())
    }

    /// Capture the explicit base/variant export once, then save it through the bounded archive worker.
    fn export_assembly(&mut self, ctx: &egui::Context, variant: Option<Uuid>) {
        let Some(project) = self.session.project() else { return };
        let Some(target) = project.board_target() else { return };
        let project_name = project.name.clone();
        let variant_name =
            variant.and_then(|id| project.assembly_variants().find(|candidate| candidate.id == id).map(|candidate| candidate.name.clone()));
        self.file_notice = None;
        self.message = None;
        let params = json!({"board":target.board,"variant":variant});
        let result = self
            .session
            .execute(EXPORT_ASSEMBLY, &params)
            .map_err(|error| error.to_string())
            .and_then(|result| {
                serde_json::from_value::<Vec<pcbcraft_engine::board::fabrication::Output>>(result["assembly"]["files"].clone())
                    .map_err(|error| error.to_string())
            })
            .and_then(|outputs| pcbcraft_engine::board::fabrication::zip(&outputs))
            .and_then(|bytes| {
                let name = crate::board_outputs::assembly_archive_name(&project_name, variant_name.as_deref());
                self.fabrication_exports.start_for(ctx, crate::board_outputs::Purpose::Assembly, name, bytes)
            });
        self.message = result.err();
    }
}
