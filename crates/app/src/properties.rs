//! Compact selection properties and an expanded form edit the same revision-bound draft.
use crate::{Dialog, PcbApp, ViewInput};
use egui::{Rect, Ui};
use pcbcraft_chrome::{
    properties as chrome, shell,
    units::{self, Unit},
};
use pcbcraft_engine::{
    SET_SYMBOL_FIELDS,
    schematic::{Point, SymbolDefinition},
};
use serde_json::json;
use std::sync::Arc;
pub(crate) mod fields;

#[derive(Debug)]
pub(super) struct State {
    draft: Option<Edit>,
    pub(super) collapsed: bool,
    pub(super) net_draft: Option<crate::net_labels::Edit>,
    pub(super) rect: Rect,
}

impl Default for State {
    fn default() -> Self {
        Self { draft: None, collapsed: false, net_draft: None, rect: Rect::NOTHING }
    }
}

/// A form's object and revision are independent of the window presenting it.
#[derive(Clone, Debug)]
pub(super) struct Edit {
    pub(super) source: ViewInput,
    symbol: String,
    component: String,
    definition: Arc<SymbolDefinition>,
    unit: String,
    available_units: Vec<String>,
    reference: String,
    value: String,
    metadata: pcbcraft_engine::schematic::fields::Metadata,
    original_metadata: pcbcraft_engine::schematic::fields::Metadata,
    original_layout: pcbcraft_engine::schematic::fields::Layouts,
    appearance: std::collections::BTreeMap<String, fields::Appearance>,
    appearance_field: String,
    rotation: u16,
    x: String,
    y: String,
    units: Unit,
    original_position: Point,
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

/// Definition actions leave the property draft only after its source and pending edits are checked.
#[derive(Clone, Copy)]
enum LibraryAction {
    EditCopy,
    Sources,
}

impl Edit {
    /// Keep pending changes distinguishable from a view-only inspection.
    fn changed(&self) -> bool {
        (&self.reference, &self.value, self.rotation) != (&self.original.0, &self.original.1, self.original.2)
            || self.position() != Ok(self.original_position)
            || self.metadata != self.original_metadata
            || self.layouts().as_ref() != Ok(&self.original_layout)
    }

    /// Parse both axes before submitting any field, naming invalid coordinates in the error.
    /// The command validates the narrower schematic bounds before committing the whole draft.
    fn position(&self) -> Result<Point, String> {
        Ok(Point {
            x_nm: units::parse_length(&self.x, self.units).map_err(|error| format!("X: {error}"))?,
            y_nm: units::parse_length(&self.y, self.units).map_err(|error| format!("Y: {error}"))?,
        })
    }

    /// Reformat valid draft coordinates in the new display unit without applying them. Invalid
    /// text stays visible for correction, with bare values subsequently read in the new unit.
    fn set_units(&mut self, unit: Unit) {
        if self.units != unit {
            for text in [&mut self.x, &mut self.y] {
                if let Ok(nm) = units::parse_length(text, self.units) {
                    *text = units::edit_label(nm, unit);
                }
            }
            for draft in self.appearance.values_mut() {
                draft.set_units(self.units, unit);
            }
            self.units = unit;
        }
    }

    /// Both forms edit the same placement draft. Enter in a coordinate field requests Apply.
    fn position_fields(ui: &mut Ui, x: &mut String, y: &mut String) -> bool {
        let x = chrome::text(ui, "X", x);
        let y = chrome::text(ui, "Y", y);
        (x.lost_focus() || y.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter))
    }

    /// The compact and expanded forms share reference/value/orientation and coordinate drafts.
    fn fields(&mut self, ui: &mut Ui) -> bool {
        chrome::text(ui, "Reference", &mut self.reference);
        chrome::text(ui, "Value", &mut self.value);
        chrome::rotation(ui, &mut self.rotation);
        Self::position_fields(ui, &mut self.x, &mut self.y)
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
        let units = self.preferences.units;
        Some(Edit {
            source,
            symbol: id.to_owned(),
            component: component.id(),
            definition: symbol.definition.clone(),
            unit: symbol.unit_id().to_owned(),
            available_units: project.available_units(&component.id()).iter().map(|unit| unit.unit_id().to_owned()).collect(),
            original: (reference.clone(), value.clone(), rotation),
            metadata: component.metadata.clone(),
            original_metadata: component.metadata.clone(),
            original_layout: symbol.field_layout.clone(),
            appearance: fields::appearances(&component.metadata, &symbol.field_layout, units),
            reference,
            value,
            rotation,
            x: units::edit_label(symbol.position.x_nm, units),
            y: units::edit_label(symbol.position.y_nm, units),
            units,
            original_position: symbol.position,
            tab: Tab::General,
            appearance_field: "reference".into(),
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
        if self.views.active.schematic.selection.single_net_label().is_some() {
            self.views.active.schematic.properties.draft = None;
            if let Some(rect) = self.net_label_inspector(ui, canvas, area) {
                return rect;
            }
        } else {
            self.views.active.schematic.properties.net_draft = None;
        }
        let width = pcbcraft_chrome::workspace::panels::card_width(canvas, true);
        let Some(current) = self.read_property_edit() else {
            if self.views.active.schematic.properties.draft.take().is_some_and(|edit| edit.changed()) {
                self.message = Some("Selection changed; unapplied property edits were discarded.".into());
            }
            let count = self.views.active.schematic.selection.len();
            let (rect, collapse) = self.views.active.panels[crate::workspace_panels::PROPERTIES].show(
                ui,
                egui::Id::unique(("selection_properties", self.views.host)),
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
        let units = self.preferences.units;
        let footprint = self.package_label(&current.component);
        let state = &mut self.views.active.schematic.properties;
        let mut edit = match state.draft.take() {
            Some(edit) if edit.source == current.source && edit.symbol == current.symbol => edit,
            Some(edit) => {
                if edit.changed() {
                    self.message = Some("Selection or revision changed; unapplied property edits were discarded.".into());
                }
                current.clone()
            }
            None => current.clone(),
        };
        let mut apply = false;
        let mut more = false;
        let mut place_unit = None;
        let mut choose = false;
        let can_choose = !edit.changed();
        edit.set_units(units);
        let (rect, collapse) = self.views.active.panels[crate::workspace_panels::PROPERTIES].show(
            ui,
            egui::Id::unique(("selection_properties", self.views.host)),
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
                apply |= edit.fields(ui);
                chrome::field(ui, "Footprint", |ui| {
                    choose = pcbcraft_chrome::controls::bounded_button(ui, "choose_footprint", &footprint, ui.available_width(), can_choose)
                        .on_hover_text("Choose footprint")
                        .clicked();
                });
                if let Some(error) = &edit.error {
                    ui.label(shell::error(error));
                }
                if edit.changed() {
                    apply |= pcbcraft_chrome::controls::button(ui, "Apply changes").clicked();
                    if pcbcraft_chrome::controls::button(ui, "Reset changes").clicked() {
                        edit = current.clone();
                    }
                }
                ui.separator();
                ui.label(shell::dim(format!("Unit {} · {} pins", edit.unit, edit.definition.unit(&edit.unit).map_or(0, |unit| unit.pins().len()))));
                for unit in &edit.available_units {
                    if pcbcraft_chrome::controls::enabled_button(ui, !edit.changed(), format!("Place unit {unit}")).clicked() {
                        place_unit = Some(unit.clone());
                    }
                }
                more = pcbcraft_chrome::controls::button(ui, "More properties ↗").clicked();
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
            self.dialog = Dialog::EditFields(Box::new(edit.clone()));
        }
        if choose {
            self.begin_footprint_assignment(Some(edit.component.clone()));
        }
        if let Some(unit) = place_unit {
            self.views.active.schematic.tool = Some(crate::schematic_canvas::Tool::PlaceSymbol {
                definition: edit.definition.clone(),
                unit,
                component: Some(edit.component.clone()),
                value: edit.value.clone(),
                revision: self.session.revision(),
                turns: 0,
            });
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
        if self.views.active.schematic.selection.single_symbol() != Some(edit.symbol.as_str()) {
            return Err("Selection changed; reopen properties to continue.".into());
        }
        let position = edit.position()?;
        let layouts = edit.layouts()?;
        let mut params = json!({"symbol":edit.symbol,"reference":edit.reference,"value":edit.value,"rotation":edit.rotation,
            "metadata":edit.metadata,"field_layout":layouts,
            "x_nm":position.x_nm,"y_nm":position.y_nm,"auto_heal":self.preferences.auto_heal_wires,"expected_revision":edit.source.revision});
        if let Some(crate::editor_view::DocumentTarget::Sheet(sheet)) = self.views.active.target() {
            params["sheet"] = json!(sheet.id());
        }
        self.session.apply(SET_SYMBOL_FIELDS, &params).map_err(|e| e.to_string())?;
        self.views.active.schematic.properties.draft = None;
        self.views.sync(&self.session);
        self.message = None;
        Ok(())
    }

    /// Present component, placement and library actions using the shared Preferences form
    /// chrome. Definition authoring and package choice have explicit actions rather than raw fields.
    pub(super) fn symbol_properties_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        edit.set_units(self.preferences.units);
        let mut apply = false;
        let mut choose = false;
        let mut library_action = None;
        let can_choose = !edit.changed();
        let title = format!("{} · Symbol properties", edit.original.0);
        let footprint = self.package_label(&edit.component);
        let action = pcbcraft_chrome::catalog::property_form(ctx, "edit_fields", &title, 760.0, "OK", |ui| {
            ui.label(shell::dim(&edit.definition.name));
            chrome::tabs(ui, &mut edit.tab, &[(Tab::General, "General"), (Tab::Terminals, "Pin functions"), (Tab::Library, "Library")]);
            ui.add_space(pcbcraft_chrome::Tokens::get().space_8);
            match edit.tab {
                Tab::General => {
                    chrome::group(ui, "Part fields", |ui| {
                        apply |= edit.metadata_fields(ui);
                        choose = chrome::field_action(ui, "Footprint", &footprint, can_choose).clicked();
                        pcbcraft_chrome::controls::collapsing(ui, "text_appearance", "Text appearance", false, None, |ui| edit.field_appearance(ui));
                    });
                    ui.add_space(pcbcraft_chrome::Tokens::get().space_12);
                    let mut position_apply = false;
                    chrome::split(ui, |ui, column| {
                        if column == 0 {
                            chrome::group(ui, "Placement", |ui| {
                                chrome::field(ui, "Unit", |ui| {
                                    ui.label(edit.unit.as_str());
                                });
                                chrome::rotation(ui, &mut edit.rotation);
                                position_apply = Edit::position_fields(ui, &mut edit.x, &mut edit.y);
                            });
                        } else {
                            chrome::group(ui, "Attributes", |ui| edit.attributes(ui));
                        }
                    });
                    apply |= position_apply;
                }
                Tab::Terminals => {
                    egui::Grid::new("property_terminals").num_columns(3).show(ui, |ui| {
                        ui.strong("Terminal");
                        ui.strong("Name");
                        ui.strong("Electrical type");
                        ui.end_row();
                        for pin in edit.definition.unit(&edit.unit).into_iter().flat_map(|unit| unit.pins()) {
                            ui.label(&pin.number);
                            ui.label(pin.name.as_deref().unwrap_or("—"));
                            ui.label(format!("{:?}", pin.electrical_type));
                            ui.end_row();
                        }
                    });
                }
                Tab::Library => {
                    chrome::group(ui, "Pinned symbol", |ui| {
                        chrome::field(ui, "Library", |ui| {
                            ui.label(&edit.definition.library);
                        });
                        chrome::field(ui, "Symbol", |ui| {
                            ui.label(&edit.definition.id);
                        });
                        chrome::field(ui, "Revision", |ui| {
                            ui.label(edit.definition.revision().to_string());
                        });
                    });
                    chrome::group(ui, "Library actions", |ui| {
                        let pending = "Apply or cancel pending property changes before opening library actions.";
                        if pcbcraft_chrome::controls::enabled_button(ui, can_choose, "Edit symbol copy…").on_disabled_hover_text(pending).clicked()
                        {
                            library_action = Some(LibraryAction::EditCopy);
                        }
                        if pcbcraft_chrome::controls::enabled_button(ui, can_choose, "Manage libraries…").on_disabled_hover_text(pending).clicked()
                        {
                            library_action = Some(LibraryAction::Sources);
                        }
                        ui.label(shell::dim(
                            "Definition edits open as an independent copy. Updating or replacing the placed symbol is not implemented yet.",
                        ));
                    });
                }
            }
            if let Some(error) = &edit.error {
                ui.label(shell::error(error));
            }
        });
        apply |= action.primary;
        if apply {
            if !self.views.active.accepts(edit.source, &self.session) {
                self.message = Some("View or project changed; reopen the symbol fields to continue editing.".into());
                return;
            }
            if let Err(error) = self.apply_property_edit(&edit) {
                edit.error = Some(error);
                self.dialog = Dialog::EditFields(Box::new(edit));
            }
        } else if choose {
            self.begin_footprint_assignment(Some(edit.component));
        } else if let Some(library_action) = library_action {
            if !self.views.active.accepts(edit.source, &self.session) || edit.changed() {
                edit.error = Some("View or project changed; reopen symbol properties to continue.".into());
                self.dialog = Dialog::EditFields(Box::new(edit));
            } else {
                match library_action {
                    LibraryAction::EditCopy => {
                        self.select(pcbcraft_engine::selection::Selection::one_symbol(edit.symbol.clone()));
                        if let Err(error) = self.fork_library_selection(pcbcraft_engine::authoring::Kind::Symbol) {
                            edit.error = Some(error);
                            self.dialog = Dialog::EditFields(Box::new(edit));
                        }
                    }
                    LibraryAction::Sources => self.dialog = Dialog::LibrarySources,
                }
            }
        } else if action.close {
            if let Some(view) = self.views.view_mut(edit.source.view()) {
                view.schematic.properties.draft = None;
            }
        } else {
            self.dialog = Dialog::EditFields(Box::new(edit));
        }
    }
}

#[cfg(test)]
mod coordinate_tests;
#[cfg(test)]
mod field_tests;

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
        h.get_by_label("Rotation").scroll_to_me();
        h.run();
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
            for label in ["Reference", "Value", "X", "Y"] {
                let rect = h.get_by_label(label).rect();
                assert!(rect.left() >= 0.0 && rect.right() <= width, "{label}: {rect:?}");
            }
            if width < 600.0 {
                assert!(h.state().views.active.schematic.properties.rect.top() > h.state().browser.rect().bottom());
            }
        }
    }
}
