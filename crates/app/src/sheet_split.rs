//! "Move to New Sheet…" captures the schematic selection with its originating view and
//! revision, shows the engine's split preview for the typed name, and applies the same plan
//! as one undo step. The engine owns every check; this form only presents its answers.

use crate::{Dialog, PcbApp, ViewInput};
use pcbcraft_chrome::shell;
use pcbcraft_engine::{
    SheetTarget,
    hierarchy::Direction,
    selection::Selection,
    sheet_split::{PREVIEW, SPLIT},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// The parts of the engine's preview the form shows. Unknown fields are ignored.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Plan {
    symbols: Vec<Value>,
    wires: Vec<Value>,
    junctions: Vec<Value>,
    net_labels: Vec<Value>,
    ports: Vec<PortRow>,
    global_nets: Vec<GlobalRow>,
    split_part_instances: Vec<SplitRow>,
}

/// One proposed hierarchical port.
#[derive(Debug, Deserialize)]
struct PortRow {
    name: String,
    generated: bool,
    direction: Direction,
}

/// A boundary net that keeps connecting by a global or power name.
#[derive(Debug, Deserialize)]
struct GlobalRow {
    name: String,
}

/// A part whose units end up on both sheets.
#[derive(Debug, Deserialize)]
struct SplitRow {
    reference: String,
}

#[derive(Debug)]
pub(super) struct Edit {
    pub(super) source: ViewInput,
    sheet: SheetTarget,
    items: Selection,
    name: String,
    /// The engine's answer for `previewed`: its plan, or why it refuses.
    preview: Result<Plan, String>,
    /// The name and revision the preview was made for; a change of either previews again.
    previewed: Option<(String, u64)>,
    error: Option<String>,
    focused: bool,
}

impl Edit {
    /// Moving is offered only for a plan the engine accepted for the name now in the field.
    fn can_move(&self) -> bool {
        self.preview.is_ok() && self.previewed.as_ref().is_some_and(|(name, _)| *name == self.name) && !self.name.trim().is_empty()
    }
}

/// "1 symbol", "3 wires": counts in the summary line.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl PcbApp {
    /// Open the form for the active view's current selection. Nothing changes until Move to Sheet.
    pub(super) fn begin_split_sheet(&mut self) {
        if !matches!(self.dialog, Dialog::None) || self.views.active.schematic.tool.is_some() {
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else {
            return;
        };
        let Some(project) = self.views.active.snapshot(&self.session) else {
            return;
        };
        let items = self.views.active.schematic.selection.clone();
        if items.symbols.is_empty() && items.wires.is_empty() {
            return;
        }
        let mut number = project.sheets().len() + 1;
        while project.sheets().any(|sheet| sheet.name.eq_ignore_ascii_case(&format!("Sheet {number}"))) {
            number += 1;
        }
        self.dialog = Dialog::SplitSheet(Box::new(Edit {
            source,
            sheet: project.sheet_target(),
            items,
            name: format!("Sheet {number}"),
            preview: Ok(Plan::default()),
            previewed: None,
            error: None,
            focused: false,
        }));
    }

    /// Ask the engine what the split would do for the typed name, at the current revision.
    fn refresh_split_preview(&mut self, edit: &mut Edit) {
        let key = (edit.name.clone(), self.session.revision());
        if edit.previewed.as_ref() == Some(&key) {
            return;
        }
        if edit.previewed.as_ref().is_some_and(|(name, _)| *name != edit.name) {
            edit.error = None;
        }
        edit.preview = if edit.name.trim().is_empty() {
            Err(String::new())
        } else {
            let params = json!({"items":edit.items,"name":edit.name,"sheet":edit.sheet.id()});
            self.session
                .execute(PREVIEW, &params)
                .map_err(|error| error.to_string())
                .and_then(|result| serde_json::from_value(result["split_preview"].clone()).map_err(|error| error.to_string()))
        };
        edit.previewed = Some(key);
    }

    /// Render the form. Cancel and closing are inert; a stale form closes without edits;
    /// engine refusals stay on screen and the name stays editable.
    pub(super) fn split_sheet_dialog(&mut self, ctx: &egui::Context, mut edit: Box<Edit>) {
        self.refresh_split_preview(&mut edit);
        let (mut submit, mut cancel) = (false, false);
        let closing = shell::dialog(ctx, "split_sheet_dialog", "Move to New Sheet", |ui| {
            let label = ui.label("Name");
            let field = ui.add(egui::TextEdit::singleline(&mut edit.name).desired_width(f32::INFINITY)).labelled_by(label.id);
            if !edit.focused {
                field.request_focus();
                edit.focused = true;
            }
            submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            match &edit.preview {
                Err(reason) if reason.is_empty() => {
                    ui.label("Type a name for the new sheet.");
                }
                Err(reason) => {
                    ui.label(shell::error(reason.as_str()));
                }
                Ok(plan) => {
                    let moved: Vec<_> = [
                        (plan.symbols.len(), "symbol", "symbols"),
                        (plan.wires.len(), "wire", "wires"),
                        (plan.junctions.len(), "junction", "junctions"),
                        (plan.net_labels.len(), "net label", "net labels"),
                    ]
                    .into_iter()
                    .filter(|(n, _, _)| *n > 0)
                    .map(|(n, one, many)| count(n, one, many))
                    .collect();
                    ui.label(format!("Moves {} to a new sheet placed here as a sheet symbol.", moved.join(", ")));
                    if plan.ports.is_empty() {
                        ui.label("No net crosses the new sheet's edge, so it needs no ports.");
                    } else {
                        ui.strong(format!("Ports ({})", plan.ports.len()));
                        for port in &plan.ports {
                            let note = if port.generated { " (generated name)" } else { "" };
                            ui.label(format!("{} · {}{note}", port.name, crate::hierarchy::direction_name(port.direction)));
                        }
                    }
                    if !plan.global_nets.is_empty() {
                        ui.strong("Connected by name, no port needed");
                        for net in &plan.global_nets {
                            ui.label(net.name.as_str());
                        }
                    }
                    for part in &plan.split_part_instances {
                        ui.label(format!("{} keeps other units on this sheet; its units will be on two sheets.", part.reference));
                    }
                }
            }
            if let Some(error) = &edit.error {
                ui.label(shell::error(error.as_str()));
            }
            ui.horizontal(|ui| {
                submit |= ui.add_enabled_ui(edit.can_move(), |ui| pcbcraft_chrome::controls::button(ui, "Move to Sheet")).inner.clicked();
                cancel = pcbcraft_chrome::controls::button(ui, "Cancel").clicked();
            });
        });
        // A changed name previews again on the next frame.
        if edit.previewed.as_ref().is_some_and(|(name, _)| *name != edit.name) {
            ctx.request_repaint();
        }
        if submit && edit.can_move() {
            if !self.views.view(edit.source.view()).is_some_and(|view| view.control_input(&self.session) == Some(edit.source)) {
                self.message = Some("View or project changed; choose Move to New Sheet again to continue.".into());
                return;
            }
            let params = json!({"items":edit.items,"name":edit.name,"sheet":edit.sheet.id(),"expected_revision":edit.source.revision});
            match self.session.apply(SPLIT, &params) {
                Ok(_) => {
                    self.views.sync(&self.session);
                    self.select(Selection::default());
                    self.message = None;
                    self.file_notice = None;
                    self.repaint_editors(ctx);
                }
                Err(error) => {
                    edit.error = Some(error.to_string());
                    self.dialog = Dialog::SplitSheet(edit);
                }
            }
        } else if !(closing || cancel) {
            self.dialog = Dialog::SplitSheet(edit);
        }
    }
}
