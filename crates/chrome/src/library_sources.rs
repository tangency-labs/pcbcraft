//! Native library management presentation, independent of source I/O and electrical models.
use crate::{Tokens, shell};

/// Keep source content scrollable at compact heights while Close remains outside the scroll area.
/// Escape and backdrop clicks close the modal through the shared shell contract.
pub fn dialog(ctx: &egui::Context, add: impl FnOnce(&mut egui::Ui)) -> bool {
    let t = Tokens::get();
    let mut close = false;
    let closing = shell::dialog_width(ctx, "library_sources", "Library sources", t.form_card_width * 2., |ui| {
        egui::ScrollArea::vertical()
            .id_salt("library_source_body")
            .max_height((ctx.content_rect().height() - t.space_24 * 6.).max(t.space_24 * 2.))
            .auto_shrink([false, true])
            .show(ui, add);
        close = ui.button("Close").clicked();
    });
    closing || close
}

/// A bounded list keeps both registrations and search results visible in the manager.
pub fn list(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().id_salt(id).max_height(Tokens::get().space_24 * 7.5).auto_shrink([false, true]).show(ui, add);
}

/// Short visible actions keep their full source-qualified names in the accessibility tree.
pub fn action(ui: &mut egui::Ui, label: &str, source_name: &str, enabled: bool) -> bool {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, format!("{label} {source_name}")));
    response.clicked()
}

/// Full-width source search avoids a fixed-width text field expanding compact modals.
pub fn search(ui: &mut egui::Ui, query: &mut String) {
    let label = ui.label("Find");
    ui.add_sized([ui.available_width(), Tokens::get().space_24], egui::TextEdit::singleline(query).hint_text("Search libraries…"))
        .labelled_by(label.id);
}
