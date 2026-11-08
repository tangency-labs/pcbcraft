//! Compact selection properties and an expanded form edit the same revision-bound draft.
use crate::{Dialog, PcbApp, ViewInput};
use egui::{Rect, Ui};
use pcbcraft_chrome::{properties as chrome, shell};
use pcbcraft_engine::{SET_SYMBOL_FIELDS, schematic::SymbolDefinition};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct State {
    draft: Option<Edit>,
    collapsed: bool,
    pub(super) rect: Rect,
}

impl Default for State {
    fn default() -> Self {
        Self { draft: None, collapsed: false, rect: Rect::NOTHING }
    }
}

/// A form's object and revision are independent of the window presenting it.
#[derive(Clone, Debug)]
pub(super) struct Edit {
    pub(super) source: ViewInput,
    symbol: String,
    component: String,
    definition: Arc<SymbolDefinition>,
    reference: String,
    value: String,
    rotation: u16,
    original: (String, String, u16),
    tab: Tab,
    error: Option<String>,
    focused: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    General,
    Terminals,
    Library,
}

impl Edit {
    /// Keep pending changes distinguishable from a view-only inspection.
    fn changed(&self) -> bool {
        (&self.reference, &self.value, self.rotation) != (&self.original.0, &self.original.1, self.original.2)
    }

    /// The compact and expanded forms share reference/value/orientation controls.
    fn fields(&mut self, ui: &mut Ui) {
        chrome::text(ui, "Reference", &mut self.reference);
        chrome::text(ui, "Value", &mut self.value);
        chrome::rotation(ui, &mut self.rotation);
    }
}

impl PcbApp {
    /// Capture the focused sheet's actual selection, never a cross-probe highlight or another view.
    pub(super) fn read_property_edit(&self) -> Option<Edit> {
        let source = self.views.active.input(&self.session)?;
        let id = self.views.active.schematic.selection.single_symbol()?;
        let project = self.views.active.snapshot(&self.session)?;
        let symbol = project.symbols().iter().find(|symbol| symbol.id() == id)?;
        let component = project.component(symbol)?;
        let reference = component.reference().to_owned();
        let value = component.value().to_owned();
        let rotation = u16::from(symbol.turns()) * 90;
        Some(Edit {
            source,
            symbol: id.to_owned(),
            component: component.id(),
            definition: symbol.definition.clone(),
            original: (reference.clone(), value.clone(), rotation),
            reference,
            value,
            rotation,
            tab: Tab::General,
            error: None,
            focused: false,
        })
    }

    /// Keep the inspector's geometry stable across selection changes. A modal replaces its
    /// controls with a read-only summary, preserving clearance without competing field editors.
    pub(super) fn selection_properties(&mut self, ui: &mut Ui, canvas: Rect, area: Rect) -> Rect {
        if !self.properties_open || (area.width() < 600.0 && self.parts.open) {
            self.views.active.schematic.properties.rect = Rect::NOTHING;
            return Rect::NOTHING;
        }
        let width = pcbcraft_chrome::workspace::panels::card_width(canvas, true);
        let Some(current) = self.read_property_edit() else {
            if self.views.active.schematic.properties.draft.take().is_some_and(|edit| edit.changed()) {
                self.message = Some("Selection changed; unapplied property edits were discarded.".into());
            }
            let count = self.views.active.schematic.selection.len();
            let (rect, collapse) = self.views.active.panels[crate::workspace_panels::PROPERTIES].show(
                ui,
                egui::Id::new(("selection_properties", self.views.host)),
                "Properties",
                canvas,
                area,
                self.views.active.schematic.properties.collapsed,
                width,
                |ui| {
                    chrome::inspector_body(ui);
                    ui.label(if count == 0 { "Nothing selected".to_owned() } else { format!("{count} items selected") });
                },
            );
            if collapse {
                self.views.active.schematic.properties.collapsed = !self.views.active.schematic.properties.collapsed;
            }
            self.views.active.schematic.properties.rect = rect;
            return rect;
        };
        let enabled = matches!(self.dialog, Dialog::None);
        let footprint = self.package_label(&current.component);
        let state = &mut self.views.active.schematic.properties;
        let mut edit = match state.draft.take() {
            Some(edit) if edit.source == current.source && edit.symbol == current.symbol => edit,
            Some(edit) => {
                if edit.changed() {
                    self.message = Some("Selection or revision changed; unapplied property edits were discarded.".into());
                }
                current
            }
            None => current,
        };
        let mut apply = false;
        let mut more = false;
        let mut choose = false;
        let can_choose = !edit.changed();
        let (rect, collapse) = self.views.active.panels[crate::workspace_panels::PROPERTIES].show(
            ui,
            egui::Id::new(("selection_properties", self.views.host)),
            "Properties",
            canvas,
            area,
            state.collapsed,
            width,
            |ui| {
                chrome::inspector_body(ui);
                chrome::identity(ui, &edit.reference, &edit.value);
                if !enabled {
                    return;
                }
                edit.fields(ui);
                chrome::field(ui, "Footprint", |ui| {
                    choose = ui.add_enabled(can_choose, egui::Button::new(&footprint).truncate()).on_hover_text("Choose footprint").clicked();
                });
                if let Some(error) = &edit.error {
                    ui.label(shell::error(error));
                }
                if edit.changed() {
                    apply = ui.button("Apply changes").clicked();
                }
                ui.separator();
                ui.label(shell::dim(format!("{} pins", edit.definition.pins().len())));
                more = ui.button("More properties ↗").clicked();
            },
        );
        if collapse {
            self.views.active.schematic.properties.collapsed = !self.views.active.schematic.properties.collapsed;
        }
        if apply {
            match self.apply_property_edit(&edit) {
                Ok(()) => {
                    if let Some(current) = self.read_property_edit() {
                        edit = current;
                    }
                }
                Err(error) => edit.error = Some(error),
            }
        }
        if more {
            self.dialog = Dialog::EditFields(edit.clone());
        }
        if choose {
            self.begin_footprint_assignment(Some(edit.component.clone()));
        }
        self.views.active.schematic.properties.draft = Some(edit);
        self.views.active.schematic.properties.rect = rect;
        rect
    }

    /// Describe the exact assignment without resolving a newer library revision.
    fn package_label(&self, component: &str) -> String {
        self.session
            .project()
            .and_then(|p| p.footprint_assignments().into_iter().find(|a| a["component"] == component))
            .and_then(|a| a["definition"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Choose footprint…".into())
    }

    /// Commit supported properties in one command and undo step. A stale view cannot redirect the edit.
    fn apply_property_edit(&mut self, edit: &Edit) -> Result<(), String> {
        if !self.views.active.accepts(edit.source, &self.session) {
            return Err("View or project changed; reopen properties to continue.".into());
        }
        let mut params = json!({"symbol":edit.symbol,"reference":edit.reference,"value":edit.value,"rotation":edit.rotation,"expected_revision":edit.source.revision});
        if let Some(crate::editor_view::DocumentTarget::Sheet(sheet)) = self.views.active.target() {
            params["sheet"] = json!(sheet.id());
        }
        self.session.apply(SET_SYMBOL_FIELDS, &params).map_err(|e| e.to_string())?;
        self.views.active.schematic.properties.draft = None;
        self.views.sync(&self.session);
        self.message = None;
        Ok(())
    }

    /// Present the expanded form using Next's component/placement grouping and existing Craft
    /// chrome. Definition authoring and package choice have explicit actions rather than raw fields.
    pub(super) fn symbol_properties_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        let mut apply = false;
        let mut cancel = false;
        let mut choose = false;
        let can_choose = !edit.changed();
        let title = format!("{} · Symbol properties", edit.original.0);
        let footprint = self.package_label(&edit.component);
        let closing = shell::scrolling_dialog_width(ctx, "edit_fields", &title, 760.0, |ui| {
            ui.label(shell::dim(&edit.definition.name));
            ui.horizontal_wrapped(|ui| {
                for (tab, label) in [(Tab::General, "General"), (Tab::Terminals, "Pin functions"), (Tab::Library, "Library")] {
                    ui.selectable_value(&mut edit.tab, tab, label);
                }
            });
            ui.separator();
            match edit.tab {
                Tab::General => {
                    chrome::columns(
                        ui,
                        |ui| {
                            chrome::section(ui, "Component");
                            let field = chrome::text(ui, "Reference", &mut edit.reference);
                            if !edit.focused {
                                field.request_focus();
                                edit.focused = true;
                            }
                            let value = chrome::text(ui, "Value", &mut edit.value);
                            apply |= (field.lost_focus() || value.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            chrome::field(ui, "Footprint", |ui| {
                                choose = ui.add_enabled(can_choose, egui::Button::new(&footprint).truncate()).clicked();
                            });
                        },
                        |ui| {
                            chrome::section(ui, "Placement");
                            chrome::field(ui, "Unit", |ui| {
                                ui.label(edit.definition.unit_id());
                            });
                            chrome::rotation(ui, &mut edit.rotation);
                        },
                    );
                }
                Tab::Terminals => {
                    egui::Grid::new("property_terminals").num_columns(3).show(ui, |ui| {
                        ui.strong("Terminal");
                        ui.strong("Name");
                        ui.strong("Electrical type");
                        ui.end_row();
                        for pin in edit.definition.pins() {
                            ui.label(&pin.number);
                            ui.label(pin.name.as_deref().unwrap_or("—"));
                            ui.label(format!("{:?}", pin.electrical_type));
                            ui.end_row();
                        }
                    });
                }
                Tab::Library => {
                    chrome::section(ui, "Pinned symbol");
                    ui.label(format!("{} / {}", edit.definition.library, edit.definition.id));
                    ui.label(format!("Revision {}", edit.definition.revision()));
                    ui.label("Project copy");
                }
            }
            if let Some(error) = &edit.error {
                ui.label(shell::error(error));
            }
            ui.separator();
            ui.horizontal(|ui| {
                apply |= ui.button("OK").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if apply {
            if !self.views.active.accepts(edit.source, &self.session) {
                self.message = Some("View or project changed; reopen the symbol fields to continue editing.".into());
                return;
            }
            if let Err(error) = self.apply_property_edit(&edit) {
                edit.error = Some(error);
                self.dialog = Dialog::EditFields(edit);
            }
        } else if choose {
            self.begin_footprint_assignment(Some(edit.component));
        } else if !(closing || cancel) {
            self.dialog = Dialog::EditFields(edit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{click, harness, resistor_at_origin};
    use egui_kittest::kittest::Queryable;

    /// Compact and expanded views retain one draft and publish one transaction, while Cancel
    /// and external revisions never apply pending text to a different object.
    #[test]
    fn compact_and_expanded_properties_share_atomic_history() {
        let mut h = harness();
        resistor_at_origin(&mut h);
        h.state_mut().properties_open = true;
        h.state_mut().select(pcbcraft_engine::selection::Selection::one_symbol("symbol-2".into()));
        h.run();
        let before = h.state().session.snapshot().unwrap();
        h.get_by_label("Value").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Value").type_text("4.7k");
        h.run();
        click(&mut h, "More properties ↗");
        assert_eq!(h.get_by_label("Value").value().as_deref(), Some("4.7k"));
        click(&mut h, "Rotation");
        click(&mut h, "90°");
        let revision = h.state().session.revision();
        click(&mut h, "OK");
        assert_eq!(h.state().session.revision(), revision + 1);
        assert_eq!(h.state().session.inspect()["project"]["schematic"]["symbols"][0]["rotation"], 90);
        assert_eq!(h.state().session.inspect()["project"]["schematic"]["symbols"][0]["value"], "4.7k");
        h.state_mut().session.apply("edit.undo", &serde_json::Value::Null).unwrap();
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click(&mut h, "More properties ↗");
        click(&mut h, "Cancel");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }

    /// The first real constructor enables Properties; headless composition retains explicit
    /// panel choices. Hiding the panel changes no authored document data.
    #[test]
    fn properties_toggle_is_a_window_action() {
        let mut h = crate::tests::startup_harness();
        assert!(h.state().properties_open);
        crate::tests::new_project_tile(&h).click();
        h.run();
        let before = h.state().session.inspect();
        click(&mut h, "View");
        click(&mut h, "Toggle Properties");
        assert!(!h.state().properties_open);
        assert_eq!(h.state().session.inspect(), before);
    }

    /// A narrow host stacks Browser and Properties without clipping editable controls.
    #[test]
    fn compact_host_stacks_properties_with_bounded_fields() {
        let mut h = harness();
        resistor_at_origin(&mut h);
        h.state_mut().properties_open = true;
        h.state_mut().select(pcbcraft_engine::selection::Selection::one_symbol("symbol-2".into()));
        for width in [1200.0, 420.0] {
            h.set_size(egui::vec2(width, 900.0));
            h.run();
            for label in ["Reference", "Value"] {
                let rect = h.get_by_label(label).rect();
                assert!(rect.left() >= 0.0 && rect.right() <= width, "{label}: {rect:?}");
            }
            if width < 600.0 {
                assert!(h.state().views.active.schematic.properties.rect.top() > h.state().browser.rect().bottom());
            }
        }
    }
}
