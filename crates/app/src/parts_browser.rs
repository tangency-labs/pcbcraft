//! Library-aware part selection in the shared Craft catalog form. Browsing is transient;
//! placement retains the originating view/revision and uses the shared engine commands.
use crate::{editor_view::ViewInput, schematic_canvas::paint_preview};
use egui::{Rect, Ui};
use pcbcraft_chrome::{
    catalog as chrome, properties, shell,
    workspace::catalog::{self, View},
};
use pcbcraft_engine::{
    library,
    net_labels::{Label, Style},
    schematic::{Point, SymbolDefinition},
};
use std::sync::Arc;

/// Collections are presentation filters, not electrical identities or source registrations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Collection {
    #[default]
    All,
    Library(String),
    Power,
}
/// Exact item keys prevent display-name collisions and stale source revisions from aliasing.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Symbol(String, String, u32),
    Power(&'static str),
}
/// Display entries retain qualified symbol definitions; built-in power markers use their own commands.
#[derive(Clone, Debug)]
enum Entry {
    Symbol(Arc<SymbolDefinition>),
    Power(&'static str, Style),
}
impl Entry {
    /// A catalog row's identity remains independent of its visible name.
    fn key(&self) -> Key {
        match self {
            Self::Symbol(s) => Key::Symbol(s.library.clone(), s.id.clone(), s.revision()),
            Self::Power(name, _) => Key::Power(name),
        }
    }
    /// Search/display text comes from the source definition or the original power preset.
    fn name(&self) -> &str {
        match self {
            Self::Symbol(s) => &s.name,
            Self::Power(name, _) => name,
        }
    }
    /// Keep power markers separate from component symbol libraries and footprint mappings.
    fn in_collection(&self, collection: &Collection) -> bool {
        match (collection, self) {
            (Collection::All, _) => true,
            (Collection::Power, Self::Power(..)) => true,
            (Collection::Library(id), Self::Symbol(s)) => *id == s.library,
            _ => false,
        }
    }
}
/// The host performs the selected action only after the catalog has closed.
pub(crate) enum Pick {
    Symbol(Arc<SymbolDefinition>),
    Power(Label),
    Libraries,
    Stale,
}

#[derive(Debug)]
pub(crate) struct PartsBrowser {
    pub(crate) open: bool,
    query: String,
    view: View,
    collection: Collection,
    selected: Option<Key>,
    power_name: String,
    source: Option<ViewInput>,
    focus: bool,
    /// Old panel placement remains readable in saved arrangements, without restoring a dock.
    legacy_panel: catalog::PanelState,
}
impl Default for PartsBrowser {
    /// Catalogs open explicitly and start with readable list rows and a separate large preview.
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            view: View::List,
            collection: Collection::All,
            selected: None,
            power_name: String::new(),
            source: None,
            focus: false,
            legacy_panel: Default::default(),
        }
    }
}
/// Retain the old arrangement vocabulary without persisting an open modal or pointer gesture.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PartsState {
    open: bool,
    view: View,
    panel: catalog::PanelState,
}
impl PartsState {
    /// Previously saved dock coordinates remain validated compatibility data.
    pub(super) fn valid(&self) -> bool {
        self.panel.valid()
    }
}
impl PartsBrowser {
    /// Save presentation; an open chooser is transient and does not reopen on project restore.
    pub(super) fn saved_state(&self) -> PartsState {
        PartsState { open: false, view: self.view, panel: self.legacy_panel.clone() }
    }
    /// Restore display density and compatible legacy data, keeping the modal initially closed.
    pub(super) fn from_state(state: &PartsState) -> Self {
        Self { view: state.view, legacy_panel: state.panel.clone(), ..Default::default() }
    }
    /// Release only this catalog's keyboard ownership when another native host takes focus.
    pub(super) fn release_focus(&mut self, ctx: &egui::Context) {
        self.focus = false;
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
    /// A reopens the current library, retaining ordinary search text between placements.
    /// After power placement it returns to all libraries and clears the power search.
    pub(crate) fn focus_search(&mut self) {
        if self.collection == Collection::Power {
            self.query.clear();
            self.collection = Collection::All;
            self.selected = None;
        }
        self.open = true;
        self.focus = true;
        self.source = None;
    }
    /// P opens the same catalog with the power collection selected, without a property form.
    pub(crate) fn focus_power(&mut self) {
        self.collection = Collection::Power;
        self.query.clear();
        self.selected = None;
        self.open = true;
        self.focus = true;
        self.source = None;
    }
    /// Closing a chooser discards its interaction authority, not its retained browsing choices.
    pub(crate) fn close(&mut self, ctx: &egui::Context) {
        self.open = false;
        self.source = None;
        self.focus = false;
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }
    /// Escape dismisses the complete form before any canvas tool can consume the key.
    pub(crate) fn take_escape(&mut self, ctx: &egui::Context) -> bool {
        if self.open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.close(ctx);
            true
        } else {
            false
        }
    }
    /// Catalog forms do not consume canvas geometry or participate in camera fitting.
    pub(crate) fn rect(&self) -> Rect {
        Rect::NOTHING
    }
    /// Browser layout remains independent of an open modal catalog.
    pub(crate) fn browser_area(&self, canvas: Rect) -> Rect {
        canvas
    }

    /// Present libraries, filtered results and selected geometry using the merged source-manager
    /// form rhythm. Confirm/double-click/Enter starts placement; a stale view refuses the choice.
    pub(crate) fn show(&mut self, ui: &Ui, source: ViewInput, libraries: &library::sources::Catalog) -> Option<Pick> {
        if !self.open {
            return None;
        }
        let owner = *self.source.get_or_insert(source);
        let all = entries("", libraries);
        let all_libraries = libraries.inspect();
        let query = self.query.trim().to_lowercase();
        let entries: Vec<_> = entries(&query, libraries).into_iter().filter(|e| e.in_collection(&self.collection)).collect();
        if self.selected.is_none() {
            self.selected = entries.first().map(Entry::key);
            self.power_name = entries.first().filter(|e| matches!(e, Entry::Power(..))).map_or(String::new(), |e| e.name().into());
        }
        let mut confirm = false;
        let mut dragging = false;
        let enabled = entries.iter().any(|e| Some(e.key()) == self.selected) && valid_power_name(&self.power_name, self.selected.as_ref());
        let action = chrome::form(ui.ctx(), "parts_catalog", "Parts Catalog", 1108., "Place", enabled, true, |ui| {
            chrome::toolbar(ui, if ui.available_width() < 900. { 2 } else { 1 }, |ui| {
                let search = ui
                    .add_sized(
                        [ui.available_width(), pcbcraft_chrome::Tokens::get().control_height],
                        egui::TextEdit::singleline(&mut self.query).hint_text("Search parts"),
                    )
                    .on_hover_text("Search parts");
                search.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search parts"));
                if search.changed() {
                    self.selected = None;
                }
                if std::mem::take(&mut self.focus) {
                    search.request_focus();
                }
                if ui.available_width() < 900. {
                    let mut choices: Vec<_> =
                        [(Collection::All, "All libraries".to_owned()), (Collection::Power, "Power".to_owned())]
                            .into_iter()
                            .chain(all_libraries.as_array().into_iter().flatten().filter_map(|source| {
                                Some((Collection::Library(source["library"].as_str()?.into()), source["name"].as_str()?.into()))
                            }))
                            .collect();
                    if let Collection::Library(id) = &self.collection
                        && !choices.iter().any(|(collection, _)| collection == &self.collection)
                    {
                        choices.push((self.collection.clone(), format!("{id} (unavailable)")));
                    }
                    let keys: Vec<_> = choices
                        .iter()
                        .map(|(choice, _)| match choice {
                            Collection::All => "all".to_owned(),
                            Collection::Power => "power".to_owned(),
                            Collection::Library(id) => format!("library:{id}"),
                        })
                        .collect();
                    let mut value =
                        choices.iter().position(|(choice, _)| *choice == self.collection).map(|i| keys[i].clone()).unwrap_or_else(|| "all".into());
                    let options: Vec<_> = keys.iter().zip(&choices).map(|(key, (_, name))| (key.as_str(), name.clone())).collect();
                    properties::choice(ui, "Library", ui.id().with("catalog_library"), &mut value, &options);
                    if let Some(index) = keys.iter().position(|key| *key == value)
                        && self.collection != choices[index].0
                    {
                        self.collection = choices[index].0.clone();
                        self.selected = None;
                    }
                }
            });
            chrome::panes(ui, |ui, region| match region {
                chrome::Region::Libraries => {
                    pcbcraft_chrome::library_sources::heading(ui, "Libraries", all_libraries.as_array().map_or(1, |s| s.len() + 1));
                    pcbcraft_chrome::library_sources::list(ui, "catalog_libraries", |ui| self.library_rows(ui, &all, &all_libraries));
                }
                chrome::Region::Results => {
                    pcbcraft_chrome::library_sources::heading(ui, "Parts", entries.len());
                    let view = catalog::view_picker(ui, &mut self.view, false);
                    if entries.is_empty() {
                        ui.label(shell::dim("No matching parts"));
                    }
                    catalog::grid(ui, view, &entries, |ui, entry, width| {
                        let identity = match entry {
                            Entry::Symbol(s) if s.library != "starter" => Some(format!("{}/{} r{}", s.library, s.id, s.revision())),
                            _ => None,
                        };
                        let value = match entry {
                            Entry::Symbol(s) => s.default_value(),
                            Entry::Power(_, Style::Ground) => "Ground",
                            Entry::Power(..) => "Power",
                        };
                        let response = catalog::tile(
                            ui,
                            view,
                            width,
                            catalog::Item { name: entry.name(), value, identity: identity.as_deref(), selected: self.selected == Some(entry.key()) },
                            |p, r, ink| paint_entry(p, r, entry, ink),
                        );
                        if let Entry::Symbol(definition) = entry {
                            response.dnd_set_drag_payload(crate::schematic_canvas::PartDrag { definition: definition.clone(), source: owner });
                            dragging |= response.drag_started();
                        }
                        if response.clicked() {
                            if self.selected != Some(entry.key()) {
                                self.power_name = entry.name().into();
                            }
                            self.selected = Some(entry.key());
                        }
                        confirm |= response.double_clicked();
                    });
                }
                chrome::Region::Preview => {
                    if let Some(entry) = entries.iter().find(|e| Some(e.key()) == self.selected) {
                        chrome::title(ui, entry.name());
                        chrome::preview(ui, |p, r| paint_entry(p, r, entry, pcbcraft_chrome::Tokens::get().canvas_ink));
                        match entry {
                            Entry::Symbol(s) => {
                                properties::section(ui, "Symbol");
                                properties::field(ui, "Value", |ui| {
                                    ui.label(s.default_value());
                                });
                                properties::field(ui, "Pins", |ui| {
                                    ui.label(s.pins().len().to_string());
                                });
                                properties::section(ui, "Source");
                                ui.label(format!("{} · r{}", collection_name(&Collection::Library(s.library.clone()), &all_libraries), s.revision()));
                            }
                            Entry::Power(..) => {
                                properties::section(ui, "Power symbol");
                                properties::text(ui, "Net name", &mut self.power_name);
                                properties::field(ui, "Scope", |ui| {
                                    ui.label("Global");
                                });
                            }
                        }
                    }
                }
            });
            confirm |= ui.input(|i| i.key_pressed(egui::Key::Enter)) && !egui::Popup::is_any_open(ui.ctx());
        });
        if dragging {
            self.open = false;
            self.source = None;
            self.focus = false;
            return None;
        }
        if action.close {
            self.close(ui.ctx());
            return None;
        }
        if action.libraries {
            self.close(ui.ctx());
            return Some(Pick::Libraries);
        }
        if (action.primary || confirm) && enabled && valid_power_name(&self.power_name, self.selected.as_ref()) {
            if owner != source {
                self.close(ui.ctx());
                return Some(Pick::Stale);
            }
            let picked = entries.iter().find(|e| Some(e.key()) == self.selected).map(|entry| match entry {
                Entry::Symbol(symbol) => Pick::Symbol(symbol.clone()),
                Entry::Power(_, style) => {
                    Pick::Power(Label { id: 0, name: self.power_name.clone(), style: *style, at: Point { x_nm: 0, y_nm: 0 }, rotation: 0 })
                }
            });
            self.close(ui.ctx());
            return picked;
        }
        None
    }
    /// Render actual loaded library identities independently of search terms and display names.
    fn library_rows(&mut self, ui: &mut Ui, entries: &[Entry], sources: &serde_json::Value) {
        for (collection, name) in [(Collection::All, "All libraries".to_owned()), (Collection::Power, "Power".to_owned())].into_iter().chain(
            sources
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| Some((Collection::Library(s["library"].as_str()?.into()), s["name"].as_str()?.into()))),
        ) {
            let count = entries.iter().filter(|entry| entry.in_collection(&collection)).count();
            let response = ui.selectable_label(self.collection == collection, format!("{name}  {count}"));
            response
                .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, self.collection == collection, name.as_str()));
            if response.clicked() {
                self.collection = collection;
                self.selected = None;
            }
        }
    }
}
/// Preset drawings are original app markers; source symbols retain their licensed definitions.
fn paint_entry(p: &egui::Painter, r: Rect, entry: &Entry, ink: egui::Color32) {
    match entry {
        Entry::Symbol(s) => paint_preview(p, s, r, ink),
        Entry::Power(_, style) => {
            let ground = *style == Style::Ground;
            let glyph = if ground { pcbcraft_chrome::net_label::Style::Ground } else { pcbcraft_chrome::net_label::Style::Power };
            let step = (r.height() / 4.).clamp(6., 22.);
            let offset = if ground { -11. } else { 10. } * step / 10.;
            pcbcraft_chrome::net_label::paint(p, r.center() + egui::vec2(0., offset), "", glyph, 0, step, ink);
        }
    }
}
/// Read a library display name while retaining its stable identity as the filter key.
fn collection_name(collection: &Collection, sources: &serde_json::Value) -> String {
    match collection {
        Collection::All => "All libraries".into(),
        Collection::Power => "Power".into(),
        Collection::Library(id) => {
            sources.as_array().into_iter().flatten().find(|s| s["library"].as_str() == Some(id)).and_then(|s| s["name"].as_str()).unwrap_or(id).into()
        }
    }
}
/// Disable Place for invalid custom power names before a canvas preview is created.
fn valid_power_name(name: &str, key: Option<&Key>) -> bool {
    !matches!(key, Some(Key::Power(_)))
        || (!name.is_empty() && name.trim() == name && name.chars().count() <= 200 && !name.chars().any(char::is_control))
}
/// Combine current unambiguous native revisions with the original built-in power collection.
fn entries(query: &str, catalog: &library::sources::Catalog) -> Vec<Entry> {
    let mut entries: Vec<_> = matches_sources(query, catalog).into_iter().map(Entry::Symbol).collect();
    for (name, style) in [
        ("GND", Style::Ground),
        ("VCC", Style::Power),
        ("+3V3", Style::Power),
        ("+5V", Style::Power),
        ("+12V", Style::Power),
        ("VDD", Style::Power),
        ("VSS", Style::Power),
        ("GNDA", Style::Ground),
    ] {
        let text = format!("{name} {}", if style == Style::Ground { "ground" } else { "power supply" }).to_lowercase();
        if text.contains(query) {
            entries.push(Entry::Power(name, style));
        }
    }
    entries
}
/// Search definition names, IDs and prefixes without inspecting external implementation sources.
fn matches(query: &str) -> Vec<Arc<SymbolDefinition>> {
    let query = query.trim().to_lowercase();
    let mut parts: Vec<_> = library::starter()
        .symbols()
        .iter()
        .filter(|d| {
            query.is_empty() || d.name.to_lowercase().contains(&query) || d.id.contains(&query) || d.reference_prefix().eq_ignore_ascii_case(&query)
        })
        .cloned()
        .collect();
    let order = ["resistor", "capacitor", "led", "connector"];
    parts.sort_by_key(|part| order.iter().position(|id| *id == part.id).unwrap_or(order.len()));
    parts
}
/// Source search preserves qualified library identity, revision and provenance for placement.
fn matches_sources(query: &str, catalog: &library::sources::Catalog) -> Vec<Arc<SymbolDefinition>> {
    let mut parts = matches(query);
    parts.extend(catalog.search(query, Some(library::sources::Kind::Symbol)).into_iter().filter_map(|(_, item)| item.symbol()));
    parts
}

#[cfg(test)]
mod tests;
