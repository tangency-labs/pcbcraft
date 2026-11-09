//! Library-aware selection shared by the large Craft chooser and compact dock. Browsing
//! leaves the project untouched; placement uses captured view input and shared engine commands.
use crate::{editor_view::ViewInput, schematic_canvas::paint_preview};
use egui::{Rect, Ui};
use pcbcraft_chrome::{
    catalog as chrome, properties,
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

/// A built-in marker's stable catalog identity and display name are independent of its default
/// electrical net name. Different glyph choices can therefore join the same supply net.
#[derive(Debug)]
struct PowerPreset {
    id: &'static str,
    name: &'static str,
    net_name: &'static str,
    style: Style,
}

static POWER_PRESETS: [PowerPreset; 9] = [
    PowerPreset { id: "gnd", name: "GND", net_name: "GND", style: Style::Ground },
    PowerPreset { id: "vcc", name: "VCC", net_name: "VCC", style: Style::Power },
    PowerPreset { id: "vcc-bar", name: "VCC (bar)", net_name: "VCC", style: Style::PowerBar },
    PowerPreset { id: "3v3", name: "+3V3", net_name: "+3V3", style: Style::Power },
    PowerPreset { id: "5v", name: "+5V", net_name: "+5V", style: Style::Power },
    PowerPreset { id: "12v", name: "+12V", net_name: "+12V", style: Style::Power },
    PowerPreset { id: "vdd", name: "VDD", net_name: "VDD", style: Style::Power },
    PowerPreset { id: "vss", name: "VSS", net_name: "VSS", style: Style::Power },
    PowerPreset { id: "gnda", name: "GNDA", net_name: "GNDA", style: Style::Ground },
];

/// Display entries retain qualified symbol definitions; built-in power markers use their own commands.
#[derive(Clone, Debug)]
enum Entry {
    Symbol(Arc<SymbolDefinition>),
    Power(&'static PowerPreset),
}
impl Entry {
    /// A catalog row's identity remains independent of its visible name.
    fn key(&self) -> Key {
        match self {
            Self::Symbol(s) => Key::Symbol(s.library.clone(), s.id.clone(), s.revision()),
            Self::Power(preset) => Key::Power(preset.id),
        }
    }
    /// Search/display text comes from the source definition or the original power preset.
    fn name(&self) -> &str {
        match self {
            Self::Symbol(s) => &s.name,
            Self::Power(preset) => preset.name,
        }
    }
    /// A glyph's descriptive catalog name never becomes its electrical name by accident.
    fn default_net_name(&self) -> Option<&'static str> {
        match self {
            Self::Power(preset) => Some(preset.net_name),
            Self::Symbol(_) => None,
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
/// The host owns the placement tool or source manager; compact catalogs can remain visible.
pub(crate) enum Pick {
    Symbol(Arc<SymbolDefinition>),
    Power(Label),
    Libraries,
    Stale,
}

/// The full chooser is the normal entry point; the compact catalog retains its own dock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Mode {
    #[default]
    Expanded,
    Compact,
}

/// Result widgets report input ownership as well as explicit placement and drag gestures.
#[derive(Default)]
struct Choices {
    confirm: bool,
    dragging: bool,
    focused: bool,
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
    focus_ids: Vec<egui::Id>,
    reveal_selection: bool,
    mode: Mode,
    panel: catalog::Panel,
    rect: Rect,
    repeat: bool,
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
            focus_ids: Vec::new(),
            reveal_selection: false,
            mode: Mode::Expanded,
            panel: catalog::Panel::at_position(catalog::Position::Left),
            rect: Rect::NOTHING,
            repeat: false,
        }
    }
}
/// Save compact presentation while keeping open modal forms and pointer gestures transient.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PartsState {
    open: bool,
    view: View,
    panel: catalog::PanelState,
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    repeat: bool,
}
impl Default for PartsState {
    /// New compact catalogs start docked beneath Browser; the full chooser starts closed.
    fn default() -> Self {
        PartsBrowser::default().saved_state()
    }
}
impl PartsState {
    /// Previously saved dock coordinates remain validated compatibility data.
    pub(super) fn valid(&self) -> bool {
        self.panel.valid()
    }
}
impl PartsBrowser {
    /// Persist the compact dock's placement and visibility, never an open modal or live input.
    pub(super) fn saved_state(&self) -> PartsState {
        PartsState {
            open: self.open && self.mode == Mode::Compact,
            view: self.view,
            panel: self.panel.saved_state(),
            mode: self.mode,
            repeat: self.repeat,
        }
    }
    /// Restore a compact dock and density; older states have no mode and keep the chooser closed.
    pub(super) fn from_state(state: &PartsState) -> Self {
        Self {
            open: state.open && state.mode == Mode::Compact,
            mode: state.mode,
            repeat: state.repeat,
            view: state.view,
            panel: catalog::Panel::from_state(&state.panel),
            ..Default::default()
        }
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
        self.panel.expand();
        self.focus = true;
        self.source = None;
    }
    /// P opens the same catalog with the power collection selected, without a property form.
    pub(crate) fn focus_power(&mut self) {
        self.collection = Collection::Power;
        self.query.clear();
        self.selected = None;
        self.reveal_selection = true;
        self.open = true;
        self.panel.expand();
        self.focus = true;
        self.source = None;
    }
    /// Closing a chooser discards its interaction authority, not its retained browsing choices.
    pub(crate) fn close(&mut self, ctx: &egui::Context) {
        self.open = false;
        self.source = None;
        self.focus = false;
        self.rect = Rect::NOTHING;
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }
    /// Escape first cancels a panel drag. Modal Escape closes the chooser; compact search
    /// yields keyboard focus before the canvas can cancel its active placement tool.
    pub(crate) fn take_escape(&mut self, ctx: &egui::Context) -> bool {
        if !self.open {
            return false;
        }
        let pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if self.mode == Mode::Compact {
            if pressed && self.panel.cancel_drag() {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                return true;
            }
            if pressed && self.focused(ctx) {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                self.release_focus(ctx);
                return true;
            }
            return false;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.close(ctx);
            true
        } else {
            false
        }
    }
    /// Only an open compact catalog covers the canvas and participates in camera fitting.
    pub(crate) fn rect(&self) -> Rect {
        if self.open && self.mode == Mode::Compact { self.rect } else { Rect::NOTHING }
    }
    /// Keep a left-docked compact catalog beneath Browser without resizing the canvas.
    pub(crate) fn browser_area(&self, canvas: Rect) -> Rect {
        if self.open && self.mode == Mode::Compact { self.panel.browser_area(canvas) } else { canvas }
    }

    /// The expanded catalog owns modal input; compact catalogs leave ordinary canvas tools usable.
    pub(crate) fn is_modal(&self) -> bool {
        self.open && self.mode == Mode::Expanded
    }

    /// Repetition is a per-window placement preference shared by parts and named anchors.
    pub(super) fn repeats(&self) -> bool {
        self.repeat
    }

    /// GUI and live controls update the same preference without changing project data.
    pub(super) fn set_repeat(&mut self, repeat: bool) {
        self.repeat = repeat;
    }

    /// Expose the same placement choice in either catalog presentation.
    fn placement_options(&mut self, ui: &mut Ui) {
        let mut repeat = self.repeat;
        if chrome::repeat_placement(ui, &mut repeat).changed() {
            self.set_repeat(repeat);
        }
    }

    /// Name the window's current catalog presentation for editor inspection.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn presentation(&self) -> &'static str {
        if self.mode == Mode::Compact { "compact" } else { "expanded" }
    }

    /// GUI and live window commands switch the same catalog state and keep it visible.
    pub(super) fn set_compact(&mut self, ctx: &egui::Context, compact: bool) {
        self.open = true;
        self.set_mode(if compact { Mode::Compact } else { Mode::Expanded }, ctx);
    }

    /// Only fields belonging to this catalog may consume compact-mode Enter or Escape.
    fn focused(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.focused().is_some_and(|id| self.focus_ids.contains(&id)))
    }

    /// Change presentation without losing browsing state. A new presentation captures fresh
    /// view authority; unfinished panel drags and drag payloads cannot cross that boundary.
    fn set_mode(&mut self, mode: Mode, ctx: &egui::Context) {
        self.panel.cancel_drag();
        self.panel.expand();
        self.mode = mode;
        self.source = None;
        self.rect = Rect::NOTHING;
        self.focus_ids.clear();
        self.focus = true;
        self.reveal_selection = true;
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }

    /// Share catalog state across the expanded chooser and compact dock. Modal choices retain
    /// their opening view/revision; nonmodal placement gestures capture the current view.
    pub(crate) fn show(&mut self, ui: &Ui, canvas: Rect, browser: Rect, source: ViewInput, libraries: &library::sources::Catalog) -> Option<Pick> {
        if !self.open {
            return None;
        }
        let expanded = self.mode == Mode::Expanded;
        let owner = if expanded { *self.source.get_or_insert(source) } else { source };
        let all = entries("", libraries);
        let sources = libraries.inspect();
        let query = self.query.trim().to_lowercase();
        let entries: Vec<_> = entries(&query, libraries).into_iter().filter(|e| e.in_collection(&self.collection)).collect();
        if self.selected.is_none() {
            self.selected = entries.first().map(Entry::key);
            self.power_name = entries.first().and_then(Entry::default_net_name).unwrap_or_default().into();
        }
        self.focus_ids.clear();
        let enabled = entries.iter().any(|e| Some(e.key()) == self.selected) && valid_power_name(&self.power_name, self.selected.as_ref());
        let mut choices = Choices::default();
        let mut action = if expanded {
            chrome::form(ui.ctx(), "parts_catalog", "Parts Catalog", 1108., "Place", enabled, true, ui.is_enabled(), |ui| {
                chrome::toolbar(ui, if ui.available_width() < 900. { 3 } else { 2 }, |ui| {
                    self.search(ui);
                    if ui.available_width() < 900. {
                        self.library_selector(ui, &sources);
                    }
                    self.placement_options(ui);
                });
                chrome::panes(ui, |ui, region| match region {
                    chrome::Region::Libraries => {
                        pcbcraft_chrome::library_sources::heading(ui, "Libraries", sources.as_array().map_or(1, |s| s.len() + 1));
                        pcbcraft_chrome::library_sources::list(ui, "catalog_libraries", |ui| self.library_rows(ui, &all, &sources));
                    }
                    chrome::Region::Results => {
                        pcbcraft_chrome::library_sources::heading(ui, "Parts", entries.len());
                        choices = self.results(ui, &entries, owner, false);
                    }
                    chrome::Region::Preview => self.preview(ui, &entries, &sources),
                });
                choices.confirm |= ui.is_enabled() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !egui::Popup::is_any_open(ui.ctx());
            })
        } else {
            let mut action = chrome::FormAction::default();
            let mut panel = std::mem::take(&mut self.panel);
            let (rect, header) = panel.show(ui, canvas, browser, |ui, short| {
                self.search(ui);
                self.library_selector(ui, &sources);
                self.placement_options(ui);
                if matches!(self.selected, Some(Key::Power(_))) {
                    let field = properties::text(ui, "Net name", &mut self.power_name);
                    self.focus_ids.push(field.id);
                }
                choices = self.results(ui, &entries, owner, short);
                choices.confirm |= ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && (choices.focused || self.focused(ui.ctx()))
                    && !egui::Popup::is_any_open(ui.ctx());
                ui.horizontal(|ui| {
                    let response = ui.button("Libraries…");
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Manage libraries…"));
                    action.libraries = response.clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        action.primary = ui.add_enabled(enabled, egui::Button::new("Place")).clicked();
                    });
                });
            });
            self.panel = panel;
            self.rect = rect;
            if header.expand {
                self.set_compact(ui.ctx(), false);
                return None;
            }
            action.close = header.close;
            action
        };
        if !ui.is_enabled() {
            return None;
        }
        if action.compact {
            self.set_compact(ui.ctx(), true);
            return None;
        }
        if choices.dragging {
            if expanded {
                self.open = false;
                self.source = None;
            }
            self.release_focus(ui.ctx());
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
        action.primary |= choices.confirm;
        if action.primary && enabled && valid_power_name(&self.power_name, self.selected.as_ref()) {
            if owner != source {
                self.close(ui.ctx());
                return Some(Pick::Stale);
            }
            let picked = entries.iter().find(|e| Some(e.key()) == self.selected).map(|entry| match entry {
                Entry::Symbol(symbol) => Pick::Symbol(symbol.clone()),
                Entry::Power(preset) => Pick::Power(Label {
                    id: 0,
                    name: self.power_name.clone(),
                    style: preset.style,
                    at: Point { x_nm: 0, y_nm: 0 },
                    rotation: 0,
                    text: None,
                }),
            });
            if expanded {
                self.close(ui.ctx());
            } else {
                self.release_focus(ui.ctx());
            }
            return picked;
        }
        None
    }

    /// Both catalog presentations edit the same query and explicitly own their search focus.
    fn search(&mut self, ui: &mut Ui) {
        let response = ui.add_sized(
            [ui.available_width(), pcbcraft_chrome::Tokens::get().control_height],
            egui::TextEdit::singleline(&mut self.query).hint_text("Search parts"),
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, ui.is_enabled(), "Search parts"));
        self.focus_ids.push(response.id);
        if response.changed() {
            self.selected = None;
            self.reveal_selection = true;
        }
        if std::mem::take(&mut self.focus) {
            response.request_focus();
        }
    }

    /// Library filters use stable source IDs and retain unavailable selections until an explicit
    /// choice, so removing a source cannot silently substitute a remaining part.
    fn library_selector(&mut self, ui: &mut Ui, sources: &serde_json::Value) {
        let mut choices: Vec<_> = [(Collection::All, "All libraries".to_owned()), (Collection::Power, "Power".to_owned())]
            .into_iter()
            .chain(
                sources
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| Some((Collection::Library(s["library"].as_str()?.into()), s["name"].as_str()?.into()))),
            )
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
        let mut value = choices.iter().position(|(choice, _)| *choice == self.collection).map(|i| keys[i].clone()).unwrap_or_else(|| "all".into());
        let options: Vec<_> = keys.iter().zip(&choices).map(|(key, (_, name))| (key.as_str(), name.clone())).collect();
        properties::choice(ui, "Library", ui.id().with("catalog_library"), &mut value, &options);
        if let Some(index) = keys.iter().position(|key| *key == value)
            && self.collection != choices[index].0
        {
            self.collection = choices[index].0.clone();
            self.selected = None;
            self.reveal_selection = true;
        }
    }

    /// Render identical selected identities and drag payloads in either presentation. Compact
    /// rows keep selection separate from placement; double-click and focused Enter can confirm.
    fn results(&mut self, ui: &mut Ui, entries: &[Entry], owner: ViewInput, short: bool) -> Choices {
        let mut choices = Choices::default();
        let view = catalog::view_picker(ui, &mut self.view, short);
        let reveal = self.reveal_selection.then(|| entries.iter().position(|entry| Some(entry.key()) == self.selected)).flatten();
        if reveal.is_some() {
            self.reveal_selection = false;
        }
        catalog::grid(ui, view, entries, reveal, |ui, entry, width| {
            let identity = match entry {
                Entry::Symbol(s) if s.library != "starter" => Some(format!("{}/{} r{}", s.library, s.id, s.revision())),
                _ => None,
            };
            let value = match entry {
                Entry::Symbol(s) => s.default_value(),
                Entry::Power(preset) if preset.style == Style::Ground => "Ground",
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
                choices.dragging |= response.drag_started();
            }
            if response.clicked() {
                if self.selected != Some(entry.key()) {
                    self.power_name = entry.default_net_name().unwrap_or_default().into();
                }
                self.selected = Some(entry.key());
            }
            choices.confirm |= response.double_clicked();
            choices.focused |= response.has_focus();
        });
        choices
    }

    /// Keep definition metadata and custom rail names in the full preview pane; compact mode
    /// uses the same tile painter and name field without duplicating this large preview.
    fn preview(&mut self, ui: &mut Ui, entries: &[Entry], sources: &serde_json::Value) {
        let Some(entry) = entries.iter().find(|e| Some(e.key()) == self.selected) else {
            return;
        };
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
                ui.label(format!("{} · r{}", collection_name(&Collection::Library(s.library.clone()), sources), s.revision()));
            }
            Entry::Power(..) => {
                properties::section(ui, "Power symbol");
                let field = properties::text(ui, "Net name", &mut self.power_name);
                self.focus_ids.push(field.id);
                properties::field(ui, "Scope", |ui| {
                    ui.label("Global");
                });
            }
        }
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
                self.reveal_selection = true;
            }
        }
    }
}
/// Preset drawings are original app markers; source symbols retain their licensed definitions.
fn paint_entry(p: &egui::Painter, r: Rect, entry: &Entry, ink: egui::Color32) {
    match entry {
        Entry::Symbol(s) => paint_preview(p, s, r, ink),
        Entry::Power(preset) => {
            let ground = preset.style == Style::Ground;
            let glyph = crate::net_labels::visual(preset.style);
            let step = (r.height() / 4.).clamp(6., 22.);
            let offset = if ground { -11. } else { 10. } * step / 10.;
            pcbcraft_chrome::net_label::paint(p, r.center() + egui::vec2(0., offset), "", glyph, 0, step, ink, false);
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
    for preset in &POWER_PRESETS {
        let text =
            format!("{} {} {}", preset.name, preset.net_name, if preset.style == Style::Ground { "ground" } else { "power supply" }).to_lowercase();
        if text.contains(query) {
            entries.push(Entry::Power(preset));
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
