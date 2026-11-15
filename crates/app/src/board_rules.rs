//! A disposable rule draft uses the shared settings modal and engine transaction.
use crate::{Dialog, PcbApp, ViewInput};
use pcbcraft_chrome::{controls, rule_form as form, shell, units};
use pcbcraft_engine::board::{rules::Values, view::BoardTarget};
use pcbcraft_engine::net_classes::Property;
use serde_json::json;

/// Captured revision and exact source integers keep stale drafts and rounded units from editing.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    board: BoardTarget,
    unit: units::Unit,
    builtin: bool,
    fields: Vec<Field>,
    error: Option<String>,
}

/// One labelled dimension retains its source precision until the user edits its text.
#[derive(Debug)]
struct Field {
    property: Property,
    label: &'static str,
    original: i64,
    initial: String,
    text: String,
}

impl Edit {
    /// Stage exact built-in dimensions when resetting intent, so disabled fields display the
    /// values Apply will restore. Cancelling still retains the original authored record.
    fn reset_values(&mut self) {
        let values = json!(Values::default());
        for field in &mut self.fields {
            field.original = values[field.property.as_str()].as_i64().unwrap_or_default();
            field.initial = units::edit_label(field.original, self.unit);
            field.text = field.initial.clone();
        }
        self.error = None;
    }

    /// Preserve untouched integers while parsing edited fields in the captured display unit.
    fn values(&self) -> Result<Values, String> {
        let mut value = json!({});
        for Field { property, original, initial, text, .. } in &self.fields {
            value[property.as_str()] =
                json!(if initial == text { *original } else { units::parse_length(text, self.unit).map_err(|e| e.to_string())? });
        }
        let values: Values = serde_json::from_value(value).map_err(|e| e.to_string())?;
        values.validate()?;
        Ok(values)
    }
}

impl PcbApp {
    /// Open the physical board's rules from either editor without mutating the design.
    #[cfg(test)]
    pub(super) fn begin_board_rules(&mut self) {
        if !matches!(self.dialog, Dialog::None) || self.files.busy() || self.project_action_pending() {
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else { return };
        self.begin_board_rules_from(source);
    }

    /// Keep a shared Rules transition bound to its original view, including split/detached hosts.
    pub(super) fn begin_board_rules_from(&mut self, source: ViewInput) {
        if !matches!(self.dialog, Dialog::None) || !self.views.view(source.view()).is_some_and(|view| view.accepts(source, &self.session)) {
            return;
        }
        let Some(project) = self.views.view(source.view()).and_then(|view| view.snapshot(&self.session)) else { return };
        let Some(board) = project.board_target().and_then(|t| project.board_view(t)) else { return };
        let unit = self.preferences.units;
        let values = json!(board.effective_rules().values);
        let fields = Property::ALL
            .into_iter()
            .map(|property| {
                let original = values[property.as_str()].as_i64().unwrap_or_default();
                let text = units::edit_label(original, unit);
                Field { property, label: crate::rule_diagram::label(property), original, initial: text.clone(), text }
            })
            .collect();
        self.dialog =
            Dialog::BoardRules(Box::new(Edit { source, board: board.target, unit, builtin: board.authored_rules().is_none(), fields, error: None }));
    }

    /// Shared form chrome keeps Apply/Cancel visible while dimensions scroll at small sizes.
    /// Cancellation or stale project/view authority discards the draft without a command.
    pub(super) fn board_rules_dialog(&mut self, ctx: &egui::Context, mut edit: Box<Edit>) {
        let current = self.views.view(edit.source.view()).is_some_and(|v| v.accepts(edit.source, &self.session));
        let value = edit.values();
        let valid = edit.builtin || value.is_ok();
        let action = pcbcraft_chrome::catalog::settings_form_width(ctx, "Board Rules", 800., "Apply", current && valid, |ui| {
            egui::ScrollArea::vertical().id_salt("board_rules_fields").auto_shrink([false, false]).show(ui, |ui| {
                pcbcraft_chrome::workspace::describe_scroll_viewport(ui);
                form::body(ui, |ui| {
                    if controls::checkbox(ui, &mut edit.builtin, "Use built-in defaults").changed() && edit.builtin {
                        edit.reset_values();
                    }
                    for preferred in [false, true] {
                        let mut fields: Vec<_> = edit.fields.iter_mut().filter(|field| field.property.is_preference() == preferred).collect();
                        form::rows(
                            ui,
                            if preferred { "board_preferences" } else { "board_limits" },
                            if preferred { "Routing preferences" } else { "Hard limits" },
                            [if edit.builtin { "Built-in value" } else { "Board value" }, "Constraint"],
                            &mut fields,
                            |ui, field, cell| match cell {
                                form::Cell::Rule => {
                                    ui.label(field.label);
                                }
                                form::Cell::Diagram => crate::rule_diagram::show(ui, field.property),
                                form::Cell::Value => {
                                    form::length_field(ui, &mut field.text, !edit.builtin, field.label);
                                }
                                form::Cell::Source => {
                                    ui.label(shell::dim(if preferred {
                                        "Preference"
                                    } else if field.property.is_maximum() {
                                        "Upper limit"
                                    } else {
                                        "Lower limit"
                                    }));
                                }
                            },
                        );
                    }
                    if !current {
                        ui.label(shell::error("The project changed. Close and reopen Rules to continue."));
                    }
                    if let Some(error) = edit.error.as_deref().or_else(|| if edit.builtin { None } else { value.as_ref().err().map(String::as_str) })
                    {
                        ui.label(shell::error(error));
                    }
                });
            });
        });
        if action.close {
            return;
        }
        if action.primary && current && valid {
            let rules = if edit.builtin {
                serde_json::Value::Null
            } else {
                match edit.values() {
                    Ok(values) => json!(values),
                    Err(error) => {
                        edit.error = Some(error);
                        self.dialog = Dialog::BoardRules(edit);
                        return;
                    }
                }
            };
            self.run(pcbcraft_engine::SET_RULES, &json!({"board":edit.board.board,"expected_revision":edit.source.revision,"rules":rules}));
            if let Some(error) = self.message.take() {
                edit.error = Some(error);
            } else {
                self.repaint_editors(ctx);
                return;
            }
        }
        self.dialog = Dialog::BoardRules(edit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::click;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use pcbcraft_engine::persistence::encode_documents;

    /// Exercise real ribbon/modal controls at desktop and compact widths.
    fn harness(width: f32) -> Harness<'static, PcbApp> {
        let mut app = PcbApp::default();
        app.start_new_project_in(crate::Workspace::Board);
        let board = app.session.project().unwrap().board_target().unwrap().board;
        app.run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"board":board,"copper_layers":2,"expected_revision":app.session.revision()}));
        let mut h = Harness::builder().with_size(egui::vec2(width, 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        h
    }

    /// Replace a field with actual editing keys instead of mutating the draft.
    fn replace(h: &mut Harness<'_, PcbApp>, label: &str, text: &str) {
        h.get_by_role_and_label(egui::Role::TextInput, label).scroll_to_me();
        h.run();
        h.get_by_role_and_label(egui::Role::TextInput, label).click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.key_press(egui::Key::Backspace);
        h.get_by_role_and_label(egui::Role::TextInput, label).type_text(text);
        h.run();
    }

    /// Applying one draft shares engine state with DRC and routing; cancellation/Undo leave no
    /// partial preferences, and unchanged mil displays retain the exact nanometres on reopen.
    #[test]
    fn board_rules_real_form_apply_cancel_history_and_units() {
        for width in [1200., 700., 400.] {
            let mut h = harness(width);
            h.state_mut().preferences.units = units::Unit::Mil;
            let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            click(&mut h, "Analyze");
            click(&mut h, "Edit net classes, assignments and board defaults.");
            click(&mut h, "Board defaults…");
            click(&mut h, "Cancel");
            assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
            h.state_mut().begin_board_rules();
            h.run();
            let diagram = h.get_by_role_and_label(egui::Role::Image, "Copper clearance diagram: Space between copper edges on different nets.");
            let field = h.get_by_role_and_label(egui::Role::TextInput, "Copper clearance");
            assert!(!diagram.accesskit_node().is_disabled(), "geometry stays legible with built-in values");
            assert!(field.accesskit_node().is_disabled());
            assert!(field.rect().width() <= 140.1 && field.rect().left() >= diagram.rect().right());
            click(&mut h, "Use built-in defaults");
            replace(&mut h, "Copper clearance", "0.35 mm");
            replace(&mut h, "Preferred track width", "0.400001 mm");
            let revision = h.state().session.revision();
            click(&mut h, "Apply");
            assert!(matches!(h.state().dialog, Dialog::None));
            assert_eq!(h.state().session.revision(), revision + 1);
            let board = h.state().session.project().unwrap().boards().next().unwrap();
            assert_eq!(board.effective_rules().values.clearance_nm, 350_000);
            assert_eq!(h.state().views.active.board.tracks.width_nm, 400_001);
            let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            h.state_mut().begin_board_rules();
            h.run();
            click(&mut h, "Apply");
            assert_eq!(saved, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
            assert_eq!(h.state().session.revision(), revision + 1);
            h.state_mut().run("edit.undo", &json!({}));
            h.run();
            assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
            h.state_mut().run("edit.redo", &json!({}));
            h.run();
            assert_eq!(saved, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
            h.state_mut().begin_board_rules();
            h.run();
            h.get_by_label("Use built-in defaults").scroll_to_me();
            h.run();
            click(&mut h, "Use built-in defaults");
            let Dialog::BoardRules(edit) = &h.state().dialog else { panic!("rule draft") };
            assert_eq!(edit.values().unwrap(), Values::default(), "reset preview uses exact defaults under mil display");
            click(&mut h, "Apply");
            assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
        }
    }

    /// Contradictory dimensions cannot apply, and a concurrent authored edit invalidates a draft.
    #[test]
    fn board_rules_invalid_and_stale_drafts_do_not_publish() {
        let mut h = harness(1000.);
        h.state_mut().begin_board_rules();
        h.run();
        click(&mut h, "Use built-in defaults");
        let before = h.state().session.inspect();
        replace(&mut h, "Minimum track width", "1 mm");
        assert!(h.get_by_role_and_label(egui::Role::Button, "Apply").accesskit_node().is_disabled());
        assert_eq!(before, h.state().session.inspect());
        replace(&mut h, "Preferred track width", "1.1 mm");
        assert!(!h.get_by_role_and_label(egui::Role::Button, "Apply").accesskit_node().is_disabled());
        h.state_mut().run("project.rename", &json!({"name":"Concurrent edit"}));
        h.run();
        assert!(h.get_by_role_and_label(egui::Role::Button, "Apply").accesskit_node().is_disabled());
        click(&mut h, "Cancel");
        assert!(h.state().session.project().unwrap().boards().next().unwrap().authored_rules().is_none());
    }

    /// Incidental whitespace edits must retain nanometres even at via/drill bounds, both when
    /// opening saved values and after resetting/re-enabling the exact built-in draft.
    #[test]
    fn board_rules_spacing_edits_round_trip_exact_mil_values() {
        let mut h = harness(1000.);
        h.state_mut().preferences.units = units::Unit::Mil;
        for reset in [false, true] {
            h.state_mut().begin_board_rules();
            h.run();
            if reset {
                click(&mut h, "Use built-in defaults");
            }
            click(&mut h, "Use built-in defaults");
            for label in ["Minimum via diameter", "Maximum via drill"] {
                let value = h.get_by_role_and_label(egui::Role::TextInput, label).value().unwrap();
                replace(&mut h, label, &format!(" {value} "));
            }
            click(&mut h, "Apply");
            assert!(matches!(h.state().dialog, Dialog::None));
            assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().authored_rules().unwrap().values, Values::default());
        }
    }

    /// Live MCP uses the actual editor refusal policy and the shared revision/history path;
    /// an open Rules draft blocks remote mutation, and a saved rule edit stales a DRC result.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn board_rules_live_mcp_and_gui_have_one_authority() {
        let mut h = harness(1200.);
        h.state_mut().run_drc();
        for _ in 0..100 {
            h.step();
            if !h.state().views.active.board.drc.pending() {
                break;
            }
        }
        assert!(!h.state().views.active.board.drc.pending());
        let before = h.state().session.revision();
        let board = h.state().session.project().unwrap().board_target().unwrap().board;
        let request = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::SET_RULES,"params":{"board":board,"expected_revision":before,"rules":Values{clearance_nm:400_000,..Default::default()}}}});
        let ctx = egui::Context::default();
        h.state_mut().begin_board_rules();
        {
            let mut editor = crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx };
            assert!(pcbcraft_mcp::live::call_editor(&mut editor, &request).is_err());
        }
        assert_eq!(h.state().session.revision(), before);
        h.state_mut().dialog = Dialog::None;
        {
            let mut editor = crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx };
            let result = pcbcraft_mcp::live::call_editor(&mut editor, &request).unwrap();
            assert_eq!(result["project"]["boards"][0]["rules"]["effective"]["values"]["clearance_nm"], 400_000);
        }
        h.run();
        assert!(h.query_by_label_contains("out of date").is_some());
        h.state_mut().begin_board_rules();
        h.run();
        assert!(matches!(h.state().dialog, Dialog::BoardRules(_)));
    }
}
