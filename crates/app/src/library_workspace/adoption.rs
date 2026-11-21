//! The Parts Library review uses the same transactions and results as MCP.
use crate::PcbApp;
use pcbcraft_chrome::controls;
use pcbcraft_engine::{
    Project,
    library::sources::{APPLY_ADOPTION, AVAILABLE_REVISIONS, PREVIEW_ADOPTION},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Weak},
};

/// A pane-local list and its current review. A new list never retains an old confirmation.
#[derive(Clone, Debug)]
pub(super) struct Review {
    baseline: Weak<Project>,
    revision: u64,
    generation: u64,
    list: Value,
    selected: BTreeSet<String>,
    preview: Option<Value>,
    result: Option<Value>,
}

/// Open a fresh list from the live project. This is a query, not an authored edit.
pub(super) fn discover(app: &mut PcbApp) -> Result<Review, String> {
    let project = app.session.snapshot().ok_or("No project is open.")?;
    let list = app.session.execute(AVAILABLE_REVISIONS, &Value::Null).map_err(|error| error.to_string())?;
    Ok(Review {
        baseline: Arc::downgrade(&project),
        revision: app.session.revision(),
        generation: app.session.libraries().generation(),
        list,
        selected: BTreeSet::new(),
        preview: None,
        result: None,
    })
}

/// Display the selectable instances, exact review and confirmation inside the library pane.
/// Returns true when review owns the remaining pane instead of the browser.
pub(super) fn panel(ui: &mut egui::Ui, app: &mut PcbApp, state: &mut Option<Review>) -> bool {
    let project = app.session.snapshot();
    if state.as_ref().is_some_and(|review| {
        review.revision != app.session.revision()
            || review.generation != app.session.libraries().generation()
            || !project.as_ref().is_some_and(|project| review.baseline.ptr_eq(&Arc::downgrade(project)))
    }) {
        *state = None;
    }
    if let Some(reason) = app.library_sources.command_refusal(AVAILABLE_REVISIONS) {
        if project.is_some() {
            ui.label(reason);
        }
        return state.is_some();
    }
    if state.is_none() {
        if controls::enabled_button(ui, project.is_some(), "Review library updates").clicked() {
            match discover(app) {
                Ok(review) => {
                    *state = Some(review);
                    app.message = None;
                }
                Err(error) => app.message = Some(error),
            }
        }
        return false;
    }
    let mut close = false;
    let mut refresh = false;
    ui.horizontal(|ui| {
        close = controls::button(ui, "Back to Parts Library").clicked();
        refresh = controls::button(ui, "Reload available revisions").clicked();
    });
    if close {
        *state = None;
        app.message = None;
        return false;
    }
    if refresh {
        match discover(app) {
            Ok(review) => {
                *state = Some(review);
                app.message = None;
            }
            Err(error) => app.message = Some(error),
        }
    }
    let Some(review) = state else { return false };
    egui::ScrollArea::vertical().id_salt("library-adoption-review").show(ui, |ui| review.show(ui, app));
    true
}

impl Review {
    /// Accumulate the chosen instance IDs, then require explicit review before Apply.
    fn show(&mut self, ui: &mut egui::Ui, app: &mut PcbApp) {
        if let Some(result) = &self.result {
            ui.heading("Library revisions adopted");
            show_changes(ui, result);
            if controls::enabled_button(ui, app.session.available("edit.undo"), "Undo adoption").clicked() {
                // Only shown while this edit is still the current project revision.
                app.run("edit.undo", &Value::Null);
                app.repaint_editors(ui.ctx());
            }
            return;
        }
        ui.heading("Available library revisions");
        ui.label("Choose instances to review their non-geometric updates.");
        let mut count = 0;
        for entry in self.list["instances"].as_array().into_iter().flatten() {
            let supported = entry["items"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|item| item["status"] == "Update available" && matches!(item["pinned"]["kind"].as_str(), Some("symbol" | "part")));
            if !supported {
                continue;
            }
            let Some(id) = entry["part_instance"].as_str() else { continue };
            count += 1;
            let mut checked = self.selected.contains(id);
            let reference = entry["reference"].as_str().unwrap_or(id);
            if controls::checkbox(ui, &mut checked, format!("{reference} ({id})")).changed() {
                if checked {
                    self.selected.insert(id.to_owned());
                } else {
                    self.selected.remove(id);
                }
                self.preview = None;
            }
        }
        if count == 0 {
            ui.label("No supported updates from loaded sources.");
        }
        if controls::enabled_button(ui, !self.selected.is_empty(), "Preview selected updates").clicked() {
            let params = json!({"instances":self.selected,"expected_revision":self.list["revision"]});
            match app.session.execute(PREVIEW_ADOPTION, &params) {
                Ok(preview) => {
                    self.preview = Some(preview);
                    app.message = None;
                }
                Err(error) => {
                    self.preview = None;
                    app.message = Some(error.to_string());
                }
            }
        }
        let Some(preview) = &self.preview else { return };
        ui.separator();
        ui.heading("Review selected updates");
        show_changes(ui, preview);
        if controls::button(ui, "Apply reviewed updates").clicked() {
            let params = json!({"instances":self.selected,"expected_revision":preview["revision"],"preview":preview["preview"]});
            // Keep the canonical engine history and refresh every project pane as for manual edits.
            match app.session.execute(APPLY_ADOPTION, &params) {
                Ok(result) => {
                    if let Some(project) = app.session.snapshot() {
                        self.baseline = Arc::downgrade(&project);
                    }
                    self.revision = app.session.revision();
                    self.result = Some(result);
                    app.file_notice = None;
                    app.message = None;
                    app.views.sync(&app.session);
                    app.repaint_editors(ui.ctx());
                }
                Err(error) => app.message = Some(error.to_string()),
            }
            self.preview = None;
        }
    }
}

/// Render exact pin changes and readable before/after fields from the command result.
fn show_changes(ui: &mut egui::Ui, report: &Value) {
    for row in report["changes"].as_array().into_iter().flatten() {
        let text = |key: &str| row[key].as_str().unwrap_or_default();
        let pin = &row["to"];
        ui.label(format!(
            "{}: {}/{} r{} → r{}",
            text("reference"),
            pin["library"].as_str().unwrap_or_default(),
            pin["id"].as_str().unwrap_or_default(),
            row["from"]["revision"],
            pin["revision"]
        ));
        for field in row["fields"].as_array().into_iter().flatten() {
            let text = |key: &str| field[key].as_str().unwrap_or_default();
            ui.label(format!("{}: {} → {}", text("field"), text("before"), text("after")));
        }
    }
}
