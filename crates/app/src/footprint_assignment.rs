//! Revision-bound package selection works in either editor; assignment never creates geometry.
use crate::{Dialog, PcbApp, ViewInput, Workspace};
use pcbcraft_chrome::{shell, units};
use pcbcraft_engine::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT, board::definition::FootprintDefinition};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug)]
pub(super) struct Edit {
    pub(super) source: ViewInput,
    pub(super) update_review: Option<Box<crate::board_update::Edit>>,
    component: String,
    component_browser: bool,
    component_query: String,
    definition: Option<Arc<FootprintDefinition>>,
    mapping: BTreeMap<String, String>,
    query: String,
    library: String,
    compatible: bool,
    mounting: Mounting,
    camera: crate::library_preview::FootprintCamera,
    x: String,
    y: String,
    units: units::Unit,
    error: Option<String>,
    reveal_error: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mounting {
    #[default]
    All,
    Surface,
    Through,
}

impl Edit {
    /// Restore an exact pinned choice; unassigned components start with an empty explicit map.
    fn select_component(&mut self, project: &pcbcraft_engine::Project) {
        self.camera = Default::default();
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
        self.reveal_error = false;
    }

    /// Reveal a validation failure once, allowing subsequent scrolling back to its inputs.
    fn fail(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.reveal_error = true;
    }

    /// Search tokens, explicit library and physical mounting filters compose with optional pin count.
    fn matches(&self, d: &FootprintDefinition, pin_count: Option<usize>) -> bool {
        let text = format!("{} {} {}", d.library, d.id, d.name).to_lowercase();
        (self.library.is_empty() || self.library == d.library)
            && self.query.split_whitespace().all(|word| text.contains(&word.to_lowercase()))
            && (!self.compatible || pin_count == Some(d.pads.len()))
            && match self.mounting {
                Mounting::All => true,
                Mounting::Through => d.pads.iter().any(|p| p.drill_nm.is_some()),
                Mounting::Surface => d.pads.iter().all(|p| p.drill_nm.is_none()),
            }
    }

    /// Placement uses the exact assignment, never a different preview candidate or draft mapping.
    fn matches_assignment(&self, project: &pcbcraft_engine::Project) -> bool {
        project.footprint_assignments().iter().find(|a| a["component"] == self.component).is_some_and(|a| {
            self.definition.as_ref().is_some_and(|d| {
                a["library"] == d.library && a["definition"] == d.id && a["definition_revision"] == d.revision && a["mapping"] == json!(self.mapping)
            })
        })
    }
}

impl PcbApp {
    /// Capture the originating view/project/revision. A schematic action supplies its component;
    /// the PCB entry point offers all project components, including those not yet placed.
    /// Package selection needs no board; placement requires an existing physical target.
    pub(super) fn begin_footprint_assignment(&mut self, component: Option<String>) {
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(project) = self.session.project() else { return };
        let component_browser = component.is_none();
        let Some(component) = component.or_else(|| project.components().next().map(|c| c.id())) else { return };
        let mut edit = Edit {
            source,
            update_review: None,
            component,
            component_browser,
            component_query: String::new(),
            definition: None,
            mapping: BTreeMap::new(),
            query: String::new(),
            library: String::new(),
            compatible: true,
            mounting: Mounting::All,
            camera: Default::default(),
            x: "0".into(),
            y: "0".into(),
            units: self.preferences.units,
            error: None,
            reveal_error: false,
        };
        edit.select_component(project);
        self.dialog = Dialog::Footprint(Box::new(edit));
    }

    /// Show package provenance and an explicit terminal map. Assign and Place are separate undoable
    /// commands; a stale form closes before dispatch. Placed choices remain proposals until
    /// reviewed update acceptance, and legacy boards first need explicit adoption.
    /// Source/library files are never edited by this dialog.
    pub(super) fn footprint_assignment_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        if !self.views.view(edit.source.view()).is_some_and(|v| v.input(&self.session) == Some(edit.source)) {
            self.message = Some("View or project changed; reopen footprint assignment to continue.".into());
            if let Some(mut review) = edit.update_review.take() {
                review.invalidate_review();
                self.resume_board_update(review);
            }
            return;
        }
        let Some(project) = self.session.snapshot() else { return };
        let catalog = project.footprint_catalog();
        let component = project.components().find(|c| c.id() == edit.component);
        let placed = project
            .board_target()
            .and_then(|target| project.board_view(target))
            .is_some_and(|board| board.footprints().any(|f| f.component == edit.component));
        let assigned = project.footprint_assignments().into_iter().find(|a| a["component"] == edit.component);
        let excluded = component.is_some_and(|c| c.metadata.attributes.exclude_from_board);
        let editable =
            !placed || project.board_target().and_then(|target| project.board_view(target)).is_some_and(|board| !board.needs_update_initialization());
        let place_allowed = edit.update_review.as_ref().is_none_or(|review| review.allows_placement());
        let placement =
            (place_allowed && project.board_target().is_some() && self.views.active.workspace == Workspace::Board && assigned.is_some() && !placed)
                .then(|| {
                    edit.matches_assignment(&project)
                        && !excluded
                        && assigned.as_ref().is_some_and(|assignment| assignment["pending_update"] != true && assignment["pending_removal"] != true)
                });
        let action = pcbcraft_chrome::catalog::footprint_form(ctx, editable && edit.definition.is_some(), placement, |ui| {
            let wide = ui.available_width() >= 900.;
            pcbcraft_chrome::catalog::footprint_panes(ui, |ui, region| {
                use pcbcraft_chrome::{catalog::Region, properties as form};
                match region {
                    Region::Libraries => {
                        if edit.component_browser {
                            form::section(ui, "Components");
                            let label = ui.label(pcbcraft_chrome::shell::dim("Find component"));
                            ui.add(form::input(&mut edit.component_query).desired_width(ui.available_width()).hint_text("Reference or value"))
                                .labelled_by(label.id);
                            let query = edit.component_query.to_lowercase();
                            let components: Vec<_> =
                                project.components().filter(|c| format!("{} {}", c.reference(), c.value()).to_lowercase().contains(&query)).collect();
                            if components.is_empty() {
                                ui.label("No components match");
                            }
                            let row_height = pcbcraft_chrome::Tokens::get().control_height;
                            egui::ScrollArea::vertical().id_salt("assignment_components").max_height(if wide { 250. } else { 120. }).show_rows(
                                ui,
                                row_height,
                                components.len(),
                                |ui, rows| {
                                    for component in &components[rows] {
                                        let id = component.id();
                                        let name = format!("{} · {}", component.reference(), component.value());
                                        if ui
                                            .add_sized(
                                                egui::vec2(ui.available_width(), row_height),
                                                egui::Button::selectable(edit.component == id, name).right_text(()).truncate(),
                                            )
                                            .clicked()
                                            && edit.component != id
                                        {
                                            edit.component = id;
                                            edit.select_component(&project);
                                        }
                                    }
                                },
                            );
                            ui.separator();
                        }
                        form::section(ui, "Libraries");
                        if ui.available_width() < 190. {
                            egui::ScrollArea::vertical().id_salt("assignment_libraries").show(ui, |ui| {
                                if ui.selectable_label(edit.library.is_empty(), "All libraries").clicked() {
                                    edit.library.clear();
                                }
                                for library in catalog.iter().map(|d| d.library.as_str()).collect::<std::collections::BTreeSet<_>>() {
                                    if ui.selectable_label(edit.library == library, library).clicked() {
                                        edit.library = library.into();
                                    }
                                }
                            });
                        } else {
                            form::choice(
                                ui,
                                "Library",
                                ui.scope_id().with("footprint_library"),
                                &mut edit.library,
                                &std::iter::once(("", "All libraries".into()))
                                    .chain(
                                        catalog
                                            .iter()
                                            .map(|d| d.library.as_str())
                                            .collect::<std::collections::BTreeSet<_>>()
                                            .into_iter()
                                            .map(|id| (id, id.into())),
                                    )
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }
                    Region::Results => {
                        if let Some(component) = project.components().find(|c| c.id() == edit.component) {
                            form::identity(ui, component.reference(), component.value());
                        }
                        ui.label(if excluded {
                            "Excluded from board"
                        } else if placed && assigned.as_ref().is_some_and(|assignment| assignment["pending_update"] == true) {
                            "Placed · update pending"
                        } else if placed {
                            "Placed"
                        } else if assigned.is_some() {
                            "Assigned · not placed"
                        } else {
                            "Unassigned"
                        });
                        ui.add_space(pcbcraft_chrome::Tokens::get().space_8);
                        ui.add_enabled_ui(editable, |ui| {
                            let label = ui.label(pcbcraft_chrome::shell::dim("Search footprints"));
                            ui.add(form::input(&mut edit.query).desired_width(ui.available_width()).hint_text("Name or package"))
                                .labelled_by(label.id);
                            ui.horizontal_wrapped(|ui| {
                                pcbcraft_chrome::controls::checkbox(ui, &mut edit.compatible, "Match pin count");
                                pcbcraft_chrome::controls::Choice::from_id_salt("mounting_filter")
                                    .selected_text(match edit.mounting {
                                        Mounting::All => "All mounting types",
                                        Mounting::Surface => "Surface mount",
                                        Mounting::Through => "Through hole",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut edit.mounting, Mounting::All, "All mounting types");
                                        ui.selectable_value(&mut edit.mounting, Mounting::Surface, "Surface mount");
                                        ui.selectable_value(&mut edit.mounting, Mounting::Through, "Through hole");
                                    })
                                    .response
                                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Mounting type"));
                            });
                            let pin_count = project.components().find(|c| c.id() == edit.component).map(|c| c.definition.pins().len());
                            let matches: Vec<_> = catalog.iter().filter(|d| edit.matches(d, pin_count)).cloned().collect();
                            ui.label(pcbcraft_chrome::shell::dim(format!("{} of {} footprints", matches.len(), catalog.len())));
                            let height = if wide { ui.available_height().max(120.) } else { 180. };
                            egui::ScrollArea::vertical().id_salt("package_results").max_height(height).show(ui, |ui| {
                                if matches.is_empty() {
                                    ui.label("No footprints match");
                                    if pcbcraft_chrome::controls::button(ui, "Clear filters").clicked() {
                                        edit.query.clear();
                                        edit.library.clear();
                                        edit.compatible = false;
                                        edit.mounting = Mounting::All;
                                    }
                                }
                                for definition in matches {
                                    let label = format!("{} / {} · r{}", definition.library, definition.name, definition.revision);
                                    let detail = format!(
                                        "{} · {} pads · {} · r{}",
                                        definition.library,
                                        definition.pads.len(),
                                        if definition.pads.iter().any(|p| p.drill_nm.is_some()) { "Through hole" } else { "SMD" },
                                        definition.revision
                                    );
                                    let selected = edit.definition.as_ref().is_some_and(|d| d == &definition);
                                    if pcbcraft_chrome::catalog::footprint_result(ui, &label, &definition.name, &detail, selected).clicked()
                                        && !selected
                                    {
                                        edit.mapping = project.default_footprint_mapping(&edit.component, &definition).unwrap_or_default();
                                        edit.definition = Some(definition);
                                        edit.camera = Default::default();
                                        edit.error = None;
                                        edit.reveal_error = false;
                                    }
                                }
                            });
                        });
                    }
                    Region::Preview => {
                        let mut details = |ui: &mut egui::Ui| {
                            let Some(definition) = &edit.definition else {
                                form::section(ui, "Preview");
                                ui.label("No footprint selected");
                                return;
                            };
                            form::identity(
                                ui,
                                &definition.name,
                                &format!("{} / {} · revision {}", definition.library, definition.id, definition.revision),
                            );
                            crate::library_preview::interactive_footprint(ui, definition, &mut edit.camera);
                            form::group(ui, "Pin mapping", |ui| {
                                if let Some(component) = project.components().find(|c| c.id() == edit.component) {
                                    ui.add_enabled_ui(editable, |ui| {
                                        let pins = component.definition.pins();
                                        let numbers: std::collections::BTreeSet<_> = pins.iter().map(|p| p.number.as_str()).collect();
                                        let pads: std::collections::BTreeSet<_> = definition.pads.iter().map(|p| p.number.as_str()).collect();
                                        form::mapping_header(ui);
                                        for pin in pins {
                                            let mapped = edit.mapping.entry(pin.number.clone()).or_default();
                                            let options: Vec<_> = std::iter::once(("", "Choose pad".into()))
                                                .chain(definition.pads.iter().map(|pad| (pad.number.as_str(), format!("Pad {}", pad.number))))
                                                .collect();
                                            form::mapping_choice(
                                                ui,
                                                &pin.name.as_ref().filter(|name| *name != &pin.number).map_or_else(
                                                    || format!("Terminal {}", pin.number),
                                                    |name| format!("Terminal {} · {name}", pin.number),
                                                ),
                                                ui.scope_id().with((&edit.component, &pin.number)),
                                                mapped,
                                                &options,
                                            );
                                        }
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(ui.available_width(), pcbcraft_chrome::Tokens::get().control_height),
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .add_enabled(numbers == pads, pcbcraft_chrome::settings::action_button("Match numbers", false))
                                                    .on_hover_text("Replace all connections with matching terminal and pad numbers")
                                                    .clicked()
                                                {
                                                    edit.mapping = numbers.into_iter().map(|id| (id.into(), id.into())).collect();
                                                }
                                            },
                                        );
                                    });
                                }
                            });
                            ui.collapsing("Source and license", |ui| {
                                ui.label(&definition.license);
                                ui.add(egui::Label::new(&definition.source.reference).wrap());
                            });
                            if placed {
                                ui.label(if editable {
                                    "Assign records the proposed footprint choice. Update PCB from schematic applies it to the placed footprint."
                                } else {
                                    "Use current state in Update PCB from schematic before changing this placed package."
                                });
                            }
                            if placement.is_some() {
                                form::card(ui, "Board position", |ui| {
                                    form::text(ui, &format!("X ({})", edit.units.symbol()), &mut edit.x);
                                    form::text(ui, &format!("Y ({})", edit.units.symbol()), &mut edit.y);
                                });
                            }
                            if let Some(error) = &edit.error {
                                let response = ui.label(shell::error(error));
                                if std::mem::take(&mut edit.reveal_error) {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                            }
                        };
                        if wide {
                            egui::ScrollArea::vertical().id_salt("package_details").show(ui, details);
                        } else {
                            details(ui);
                        }
                    }
                }
            });
        });
        let assign = action.primary;
        let place = action.secondary;
        if place && !edit.matches_assignment(&project) {
            edit.fail("Assign the selected footprint and mapping before placing it.");
            self.dialog = Dialog::Footprint(Box::new(edit));
            return;
        }
        if assign || place {
            let mut params = json!({"component":edit.component,"expected_revision":edit.source.revision});
            if let Some(board) = project.board_target() {
                params["board"] = json!(board.board);
            } else if place {
                edit.fail("Create a board before placing footprints.");
                self.dialog = Dialog::Footprint(Box::new(edit));
                return;
            }
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
                        edit.fail("Enter valid X and Y lengths.");
                        self.dialog = Dialog::Footprint(Box::new(edit));
                        return;
                    }
                }
                PLACE_ASSIGNED_FOOTPRINT
            };
            match self.session.apply(command, &params) {
                Ok(_) => {
                    self.message = None;
                    if assign && let Some(review) = edit.update_review.as_mut() {
                        review.invalidate_review();
                    }
                    self.views.sync(&self.session);
                    if place
                        && let Some(footprint) = self
                            .session
                            .project()
                            .and_then(|p| p.board_target().and_then(|target| p.board_view(target)))
                            .and_then(|board| board.footprints().find(|f| f.component == edit.component))
                    {
                        self.views.active.camera.request_center(footprint.position);
                        self.views.select_board(std::collections::BTreeSet::from([footprint.id]), &self.session, self.preferences.selection_follow);
                    }
                    self.repaint_editors(ctx);
                    if assign && let Some(review) = edit.update_review.take() {
                        self.resume_board_update(review);
                    } else if assign && let Some(source) = self.views.view(edit.source.view()).and_then(|v| v.input(&self.session)) {
                        edit.source = source;
                        edit.error = None;
                        self.dialog = Dialog::Footprint(Box::new(edit));
                    } else if place && let Some(review) = edit.update_review.take() {
                        self.resume_board_update(review);
                    }
                }
                Err(error) => {
                    edit.fail(error.to_string());
                    self.dialog = Dialog::Footprint(Box::new(edit));
                }
            }
        } else if action.close {
            if let Some(review) = edit.update_review.take() {
                self.resume_board_update(review);
                self.repaint_editors(ctx);
            }
        } else {
            self.dialog = Dialog::Footprint(Box::new(edit));
        }
    }
}
