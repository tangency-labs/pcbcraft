//! Update options and a filtered change report over the shared Craft dialog shell.
use crate::{Tokens, catalog};
use egui::{Frame, RichText, Stroke, Ui, UiBuilder, vec2};

/// Message classes describe the report rather than granting permission to apply a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Error,
    Warning,
    Action,
    Info,
}

/// Display-only report text; callers retain the associated component and update capability.
#[derive(Clone, Debug)]
pub struct Message {
    /// Severity/category used by filters and totals.
    pub kind: Kind,
    /// Human-readable action, diagnostic or connection description.
    pub text: String,
    /// Optional corrective action, for example assigning a missing package.
    pub action: Option<String>,
}

/// Presentation filters never change which engine operations an update will accept.
#[derive(Clone, Debug)]
pub struct Filters {
    errors: bool,
    warnings: bool,
    actions: bool,
    infos: bool,
}

impl Default for Filters {
    /// Start a fresh review with every report category visible.
    fn default() -> Self {
        Self { errors: true, warnings: true, actions: true, infos: true }
    }
}

impl Filters {
    /// Preserve report order while selecting only visible message classes.
    fn includes(&self, kind: Kind) -> bool {
        match kind {
            Kind::Error => self.errors,
            Kind::Warning => self.warnings,
            Kind::Action => self.actions,
            Kind::Info => self.infos,
        }
    }
}

/// UI results address report rows; domain IDs and mutation remain with the owning editor.
#[derive(Default)]
pub struct ReportAction {
    /// Index of the corrective action activated in the unfiltered message list.
    pub row: Option<usize>,
    /// Request a clipboard copy of the full report, independently of filters.
    pub copy: bool,
}

/// Keep Close and the primary action fixed while the inner report scrolls. Lifecycle-only
/// prompts use a shorter body; normal review has room for options, messages and filter controls.
pub fn form(ctx: &egui::Context, primary: &str, enabled: bool, report: bool, add: impl FnOnce(&mut Ui)) -> catalog::FormAction {
    catalog::form_body(
        ctx,
        "board_update",
        "Update PCB from Schematic",
        800.,
        primary,
        enabled,
        false,
        true,
        if report { 20. } else { 8. },
        None,
        "Close",
        |ui| {
            let t = Tokens::get();
            let inner = ui.available_rect_before_wrap().shrink2(vec2(t.space_12, t.space_8));
            ui.scope_builder(UiBuilder::new().max_rect(inner), add);
        },
    )
}

/// Expose the supported post-update placement choice without changing the reviewed plan.
pub fn options(ui: &mut Ui, place: &mut bool) {
    ui.strong("Options");
    ui.checkbox(place, "Place new footprints after updating");
}

/// Draw one scrollable log, unfiltered warning/error totals and independently selectable message
/// categories. Hiding an error never hides its count or makes a blocked update applicable.
pub fn report(ui: &mut Ui, messages: &[&Message], filters: &mut Filters, trailing_actions: bool) -> ReportAction {
    let t = Tokens::get();
    let mut action = ReportAction::default();
    let errors = messages.iter().filter(|m| m.kind == Kind::Error).count();
    let warnings = messages.iter().filter(|m| m.kind == Kind::Warning).count();
    ui.strong("Changes to Be Applied");
    let height = (ui.available_height() - t.control_height * (if trailing_actions { 6. } else { 3. }) - t.space_8 * 4.).max(t.control_height * 2.);
    Frame::new().fill(t.field).stroke(Stroke::new(1., t.border)).inner_margin(t.space_8).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt("pcb_update_report").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
            let mut visible = false;
            for (index, message) in messages.iter().enumerate().filter(|(_, m)| filters.includes(m.kind)) {
                visible = true;
                ui.push_id(index, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let color = match message.kind {
                            Kind::Error => t.error,
                            Kind::Warning => ui.visuals().warn_fg_color,
                            _ => t.text,
                        };
                        ui.add(egui::Label::new(RichText::new(&message.text).color(color)).wrap());
                        if let Some(label) = &message.action
                            && ui.add(catalog::button(label, false)).clicked()
                        {
                            action.row = Some(index);
                        }
                    });
                });
            }
            if !visible {
                ui.label("No messages match the selected filters.");
            }
        });
    });
    ui.label(format!("Total warnings: {warnings}, errors: {errors}."));
    ui.horizontal_wrapped(|ui| {
        ui.label("Show:");
        let mut all = filters.errors && filters.warnings && filters.actions && filters.infos;
        if ui.checkbox(&mut all, "All").changed() {
            *filters = Filters { errors: all, warnings: all, actions: all, infos: all };
        }
        ui.checkbox(&mut filters.errors, format!("Errors ({errors})"));
        ui.checkbox(&mut filters.warnings, format!("Warnings ({warnings})"));
        ui.checkbox(&mut filters.actions, "Actions");
        ui.checkbox(&mut filters.infos, "Infos");
        action.copy = ui.add(catalog::button("Copy report", false)).clicked();
    });
    action
}
