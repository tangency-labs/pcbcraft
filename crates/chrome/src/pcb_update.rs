//! Update options and a filtered change report over the shared Craft dialog shell.
use crate::{Tokens, catalog};
use egui::{RichText, Sense, Ui, UiBuilder, vec2};
use solidcraft_ui_kit::{self as kit, Container, Surface};

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

/// Keep the header, Close and primary action fixed while the bounded body scrolls. Lifecycle-only
/// prompts use a shorter body; normal review has room for options, messages and filter controls.
pub fn form(ctx: &egui::Context, primary: &str, enabled: bool, report: bool, add: impl FnOnce(&mut Ui)) -> catalog::FormAction {
    let t = Tokens::get();
    let id = egui::Id::unique("board_update");
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let mut closed = false;
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        let width = 800.0_f32.min((ctx.content_rect().width() - 2.0 * t.space_16).max(0.0));
        ui.set_width(width);
        let field_style = ui.style().clone();
        kit::scope(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let (header, close) = catalog::form_header(ui, "Update PCB from Schematic");
            closed = close;
            let rows = if report { 20.0 } else { 8.0 };
            let height =
                (ctx.content_rect().height() - header.height() - t.form_footer_height - t.space_24 * 2.0).clamp(0.0, t.control_height * rows);
            let (body, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let inner = body.shrink2(vec2(t.space_12, t.space_8));
            let mut content = ui.new_child(UiBuilder::new().id_salt("update_body").max_rect(inner));
            content.set_clip_rect(ui.clip_rect().intersect(inner));
            content.set_style(field_style);
            egui::ScrollArea::vertical().id_salt("update_body_scroll").max_height(inner.height()).auto_shrink([false, false]).show(
                &mut content,
                |ui| {
                    crate::workspace::describe_scroll_viewport(ui);
                    add(ui);
                },
            );
            if ui.ctx().memory(|memory| memory.focused().is_none()) {
                ui.ctx().input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            }
            kit::footer(ui, primary, "Close", enabled, kit::Footer::default())
        })
    });
    let cancelled = modal.inner == kit::Action::Cancel && !ctx.input(|input| input.key_pressed(egui::Key::Escape));
    catalog::FormAction { close: closed || cancelled || modal.should_close(), primary: modal.inner == kit::Action::Ok, ..Default::default() }
}

/// Expose the supported post-update placement choice without changing the reviewed plan.
pub fn options(ui: &mut Ui, place: &mut bool) {
    ui.strong("Options");
    kit::checkbox(ui, "Place new footprints after updating", place);
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
    kit::report_list(ui, "pcb_update_report", height, |ui| {
        ui.set_min_height(height);
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
                        && kit::button(ui, label, kit::Button::Normal).clicked()
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
    ui.label(format!("Total warnings: {warnings}, errors: {errors}."));
    ui.horizontal_wrapped(|ui| {
        ui.label("Show:");
        let mut all = filters.errors && filters.warnings && filters.actions && filters.infos;
        let mixed = !all && (filters.errors || filters.warnings || filters.actions || filters.infos);
        if kit::checkbox_indeterminate(ui, "All", &mut all, mixed).changed() {
            *filters = Filters { errors: all, warnings: all, actions: all, infos: all };
        }
        kit::checkbox(ui, &format!("Errors ({errors})"), &mut filters.errors);
        kit::checkbox(ui, &format!("Warnings ({warnings})"), &mut filters.warnings);
        kit::checkbox(ui, "Actions", &mut filters.actions);
        kit::checkbox(ui, "Infos", &mut filters.infos);
        action.copy = kit::button(ui, "Copy report", kit::Button::Normal).clicked();
    });
    action
}
