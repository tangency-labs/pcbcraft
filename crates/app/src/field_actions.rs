//! Field rotation and web links use the owning view; only appearance commits enter history.
use crate::PcbApp;
use egui::Ui;
use pcbcraft_engine::{Project, SET_SYMBOL_FIELDS, schematic::fields::Layouts};
use serde_json::json;

impl PcbApp {
    /// Commit one field layout through the same revision-checked command used by Properties and
    /// MCP. Content, symbol placement and all other fields are retained from the current snapshot.
    pub(super) fn set_field_layout(&mut self, symbol: &str, layouts: Layouts, revision: u64) {
        let params = self.views.active.snapshot(&self.session).and_then(|project| {
            let placement = project.symbols().iter().find(|s| s.id() == symbol)?;
            let component = project.component(placement)?;
            Some(json!({"symbol":symbol,"reference":component.reference(),"value":component.value(),
                "field_layout":layouts,"expected_revision":revision}))
        });
        if let Some(params) = params {
            self.run(SET_SYMBOL_FIELDS, &params);
        }
    }

    /// D opens the hovered body's/field's owner, falling back to a single selected symbol.
    /// This host action does not change selection, project revision, saved data or undo history.
    pub(super) fn open_datasheet(&mut self, ui: &Ui, project: &Project) {
        let rect = ui.available_rect_before_wrap();
        let hovered = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)).and_then(|pointer| {
            crate::schematic_canvas::fields::at(ui, project, &self.views.active.camera, rect, pointer)
                .map(|field| field.symbol)
                .or_else(|| crate::schematic_canvas::symbol_at(project, &self.views.active.camera, rect, pointer).map(|(symbol, _)| symbol.id()))
        });
        let target = hovered.or_else(|| self.views.active.schematic.selection.single_symbol().map(str::to_owned));
        let Some(symbol) = target.and_then(|id| project.symbols().iter().find(|s| s.id() == id)) else { return };
        let url = project
            .component(symbol)
            .and_then(|c| c.metadata.fields.get("datasheet"))
            .and_then(|field| pcbcraft_chrome::properties::web_link(&field.value));
        if let Some(url) = url {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            self.message = None;
        } else {
            self.message = Some("No HTTP(S) datasheet link is set for this component.".into());
        }
    }
}
