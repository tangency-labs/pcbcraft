//! Sheet lifecycle forms capture their originating view and revision; engine commands own edits.

use crate::{Dialog, PcbApp, ViewId, ViewInput, editor_view::DocumentTarget};
use pcbcraft_chrome::shell;
use pcbcraft_engine::{
    SheetTarget,
    sheets::{CREATE_SHEET, DELETE_SHEET, RENAME_SHEET},
};
use serde_json::json;

#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    command: &'static str,
    sheet: Option<SheetTarget>,
    text: String,
    error: Option<String>,
    focused: bool,
}

impl PcbApp {
    /// Open a lifecycle form only for a live source and known target. The captured address
    /// prevents a delayed rename/delete from applying after an edit, reopen or view change.
    pub(super) fn begin_sheet_edit(&mut self, source: ViewId, command: &'static str, sheet: Option<SheetTarget>) {
        let Some(input) = self.views.view(source).and_then(|view| view.control_input(&self.session)) else {
            return;
        };
        let Some(project) = self.session.project() else {
            return;
        };
        let text = if let Some(target) = sheet {
            let Some(sheet) = project.sheets().find(|s| s.target == target) else {
                return;
            };
            sheet.name.to_owned()
        } else {
            let mut number = project.sheets().len() + 1;
            while project.sheets().any(|sheet| sheet.name.eq_ignore_ascii_case(&format!("Sheet {number}"))) {
                number += 1;
            }
            format!("Sheet {number}")
        };
        self.dialog = Dialog::Sheet(Edit { source: input, command, sheet, text, error: None, focused: false });
    }

    /// Render a captured create/rename form or deletion confirmation, keeping engine validation
    /// errors editable. Cancel is inert; stale forms close without changing project or history.
    pub(super) fn sheet_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        let deletion = edit.command == DELETE_SHEET;
        let title = match edit.command {
            CREATE_SHEET => "New Sheet",
            RENAME_SHEET => "Rename Sheet",
            _ => "Delete Sheet",
        };
        let (mut submit, mut cancel) = (false, false);
        let closing = shell::dialog(ctx, "sheet_dialog", title, |ui| {
            if deletion {
                ui.label(format!("Delete sheet “{}”?", edit.text));
                ui.label("Its symbols, wires, associated board footprints and connections to parent pins will be removed. Child sheets become top-level sheets. Undo restores them.");
            } else {
                let label = ui.label("Name");
                let field = ui.add(egui::TextEdit::singleline(&mut edit.text).desired_width(f32::INFINITY)).labelled_by(label.id);
                if !edit.focused {
                    field.request_focus();
                    edit.focused = true;
                }
                submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            }
            if let Some(error) = &edit.error {
                ui.label(shell::error(error.as_str()));
            }
            ui.horizontal(|ui| {
                submit |= ui.button(if deletion { "Delete Sheet" } else { "OK" }).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if submit {
            if !self.views.view(edit.source.view()).is_some_and(|view| view.control_input(&self.session) == Some(edit.source)) {
                self.message = Some("View or project changed; reopen the sheet action to continue.".into());
                return;
            }
            let before: Vec<_> = self.session.project().into_iter().flat_map(|p| p.sheets()).map(|s| s.target).collect();
            let mut params = json!({"expected_revision":edit.source.revision});
            if !deletion {
                params["name"] = json!(edit.text);
            }
            if let Some(sheet) = edit.sheet {
                params["sheet"] = json!(sheet.id());
            }
            match self.session.apply(edit.command, &params) {
                Ok(_) => {
                    self.views.sync(&self.session);
                    let destination = self.session.project().and_then(|project| {
                        if edit.command == CREATE_SHEET {
                            project.sheets().find(|s| !before.contains(&s.target)).map(|s| s.target)
                        } else if deletion && self.views.view(edit.source.view()).is_some_and(|view| view.unavailable()) {
                            project.sheets().next().map(|s| s.target)
                        } else {
                            None
                        }
                    });
                    if let Some(sheet) = destination {
                        self.views.open_document(edit.source.view(), DocumentTarget::Sheet(sheet), &self.session);
                    }
                    self.message = None;
                    self.file_notice = None;
                    self.repaint_editors(ctx);
                }
                Err(error) => {
                    edit.error = Some(error.to_string());
                    self.dialog = Dialog::Sheet(edit);
                }
            }
        } else if !(closing || cancel) {
            self.dialog = Dialog::Sheet(edit);
        }
    }
}
