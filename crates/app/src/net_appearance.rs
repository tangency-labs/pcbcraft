//! Revision-bound net wire appearance form shared by schematic Properties and MCP.
//!
//! The form edits a sparse patch; it never writes project documents itself and closes on the
//! first successful shared command. Selection and highlight colors stay in control of the canvas.

use crate::{Dialog, PcbApp, ViewInput, Workspace};
use pcbcraft_chrome::{catalog, controls, rule_form as form, settings, shell};
use pcbcraft_engine::nets::{Appearance, ClassTarget, INSPECT_APPEARANCE, MAX_WIDTH_CP, MIN_WIDTH_CP, SET_APPEARANCE};

/// One captured view/revision and the current saved override of exactly one net.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    target: ClassTarget,
    net_name: String,
    use_color: bool,
    use_width: bool,
    color: [u8; 3],
    width_cp: u16,
    error: Option<String>,
}

impl Edit {
    /// Build an editable draft from the inspection result, defaulting to the theme appearance.
    fn new(source: ViewInput, target: ClassTarget, net_name: String, inspected: Option<Appearance>) -> Self {
        let theme_color = pcbcraft_chrome::theme::Tokens::get().schematic_wire;
        let color = inspected.and_then(|appearance| appearance.color).unwrap_or([theme_color.r(), theme_color.g(), theme_color.b()]);
        let width_cp = inspected.and_then(|appearance| appearance.width_cp).unwrap_or(pcbcraft_engine::nets::DEFAULT_WIDTH_CP);
        let use_color = inspected.is_some_and(|appearance| appearance.color.is_some());
        let use_width = inspected.is_some_and(|appearance| appearance.width_cp.is_some());
        Self { source, target, net_name, use_color, use_width, color, width_cp, error: None }
    }

    /// Render the form and return the user's action without touching the session.
    fn show(&mut self, ui: &mut egui::Ui) -> Action {
        let mut action = Action::default();
        ui.label(format!("Appearance for {}", self.net_name));
        ui.label("All wires on this net use the same drawn color and width across sheets.");
        ui.add_space(ui.spacing().item_spacing.y * 2.0);
        ui.horizontal(|ui| {
            controls::checkbox(ui, &mut self.use_color, "Set wire color");
            ui.add_enabled_ui(self.use_color, |ui| {
                settings::color(ui, "Wire color", &mut self.color);
            });
        });
        ui.horizontal(|ui| {
            controls::checkbox(ui, &mut self.use_width, "Set drawn width");
            ui.add_enabled_ui(self.use_width, |ui| {
                ui.add(egui::DragValue::new(&mut self.width_cp).range(MIN_WIDTH_CP..=MAX_WIDTH_CP).speed(1))
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, ui.is_enabled(), "Wire stroke width"));
                ui.label(format!("= {:.2} pt", f32::from(self.width_cp) / 100.0));
            });
        });
        ui.label("Width is a screen-space stroke, so it stays readable at every zoom level.");
        if controls::button(ui, "Reset to theme default").clicked() {
            action.reset = true;
        }
        if let Some(error) = &self.error {
            ui.label(shell::error(error));
        }
        action
    }

    /// Build the sparse patch sent through the shared command. An empty patch resets all fields.
    fn patch(&self) -> serde_json::Value {
        if !self.use_color && !self.use_width {
            return serde_json::Value::Null;
        }
        let mut patch = serde_json::Map::new();
        if self.use_color {
            patch.insert("color".into(), serde_json::json!(self.color));
        }
        if self.use_width {
            patch.insert("width_cp".into(), serde_json::json!(self.width_cp));
        }
        serde_json::Value::Object(patch)
    }
}

#[derive(Default)]
struct Action {
    reset: bool,
}

impl PcbApp {
    /// Open the appearance form for the current schematic wire selection, if it names one net.
    pub(super) fn begin_net_appearance(&mut self) {
        if !matches!(self.dialog, Dialog::None)
            || self.files.busy()
            || self.project_action_pending()
            || self.views.active.workspace != Workspace::Schematic
        {
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(view) = self.views.view(source.view()) else { return };
        if !view.accepts(source, &self.session) {
            return;
        }
        let Some(project) = self.session.snapshot() else { return };
        let selected = crate::net_class_editor::selected_target(&project, view.target(), &view.schematic.selection, &view.board.tracks.selected);
        let target = match selected {
            Ok(Some(target)) => target,
            Ok(None) => {
                self.message = Some("Select a wire on a named net to edit its appearance.".into());
                return;
            }
            Err(error) => {
                self.message = Some(error);
                return;
            }
        };
        let target_value = match serde_json::to_value(&target) {
            Ok(value) => value,
            Err(error) => {
                self.message = Some(error.to_string());
                return;
            }
        };
        let inspected = self.session.execute(INSPECT_APPEARANCE, &serde_json::json!({"target":target_value}));
        let appearance = inspected.ok().and_then(|report| serde_json::from_value::<Option<Appearance>>(report["appearance"].clone()).ok()).flatten();
        let net_name = project
            .inspect_net_registry()
            .ok()
            .and_then(|registry| {
                registry["nets"].as_array()?.iter().find_map(|row| {
                    let anchor: pcbcraft_engine::nets::Anchor = serde_json::from_value(row["anchor"].clone()).ok()?;
                    (pcbcraft_engine::nets::ClassTarget::Derived { anchor } == target).then(|| row["name"].as_str().unwrap_or("Net").to_owned())
                })
            })
            .unwrap_or_else(|| "net".to_owned());
        self.dialog = Dialog::NetAppearance(Box::new(Edit::new(source, target, net_name, appearance)));
    }

    /// Host the standalone form and submit through the ordinary shared command path.
    pub(super) fn net_appearance_dialog(&mut self, ctx: &egui::Context, mut edit: Box<Edit>) {
        let current = self.views.view(edit.source.view()).is_some_and(|view| view.accepts(edit.source, &self.session));
        let valid = self.session.project().is_some();
        let mut action = Action::default();
        let form_action = catalog::settings_form_width(ctx, "Net appearance", 420., "Apply", current && valid, |ui| {
            form::body(ui, |ui| {
                action = edit.show(ui);
            });
        });
        if form_action.close {
            return;
        }
        let mut submit = form_action.primary;
        if action.reset {
            submit = true;
        }
        if submit && current && valid {
            let appearance = if action.reset { serde_json::Value::Null } else { edit.patch() };
            let target = match serde_json::to_value(&edit.target) {
                Ok(value) => value,
                Err(error) => {
                    edit.error = Some(error.to_string());
                    self.dialog = Dialog::NetAppearance(edit);
                    return;
                }
            };
            self.run(
                SET_APPEARANCE,
                &serde_json::json!({
                    "target":target,
                    "appearance":appearance,
                    "expected_revision":edit.source.revision,
                }),
            );
            if self.message.is_none() {
                self.dialog = Dialog::None;
                return;
            }
            edit.error = self.message.clone();
        }
        self.dialog = Dialog::NetAppearance(edit);
    }
}
