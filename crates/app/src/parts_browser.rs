//! Library-aware selection shared by the large Craft chooser and compact dock. Browsing
//! leaves the project untouched; placement uses captured view input and shared engine commands.
use crate::{editor_view::ViewInput, schematic_canvas::paint_preview};
use egui::{Rect, Ui};
use pcbcraft_chrome::{
    catalog as chrome, properties,
    workspace::catalog::{self, View},
};
use pcbcraft_engine::{library, schematic::SymbolDefinition};
use std::sync::Arc;

/// Collections are presentation filters, not electrical identities or source registrations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Collection {
    #[default]
    All,
    Library(String),
}
/// Exact item keys prevent display-name collisions and stale source revisions from aliasing.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Symbol(String, String, u32),
}

/// Every catalog row retains the same validated native definition, independent of delivery.
#[derive(Clone, Debug)]
struct Entry(Arc<SymbolDefinition>);
impl Entry {
    /// Qualified item identity remains independent of display names and source paths.
    fn key(&self) -> Key {
        Key::Symbol(self.0.library.clone(), self.0.id.clone(), self.0.revision())
    }
    /// Display names come from native definitions, never from collection membership.
    fn name(&self) -> &str {
        &self.0.name
    }
    /// The editable template starts with the same Value as direct engine placement.
    fn default_value(&self) -> &str {
        self.0.default_value()
    }
    /// Libraries are presentation filters; electrical roles do not define a second item kind.
    fn in_collection(&self, collection: &Collection) -> bool {
        match collection {
            Collection::All => true,
            Collection::Library(id) => *id == self.0.library,
        }
    }
}
/// The host owns the placement tool or source manager; compact catalogs can remain visible.
pub(crate) enum Pick {
    Place(Placement),
    Libraries,
    Stale,
}

/// One instance template shared by native library selection, drag/drop and canvas tools.
#[derive(Clone, Debug)]
pub(crate) struct Placement {
    pub definition: Arc<SymbolDefinition>,
    pub value: String,
}
impl Placement {
    /// Copy definition defaults without altering the pinned library item.
    pub fn new(definition: Arc<SymbolDefinition>) -> Self {
        Self { value: definition.default_value().to_owned(), definition }
    }
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
    value: String,
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
            value: String::new(),
            source: None,
            focus: false,
            focus_ids: Vec::new(),
            reveal_selection: false,
            mode: Mode::Expanded,
            panel: catalog::Panel::at_position(catalog::Position::Left),
            rect: Rect::NOTHING,
            repeat: true,
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
    #[serde(default = "repeat_default")]
    repeat: bool,
}
/// New and pre-repeat catalog settings keep placement armed; an explicit false stays one-shot.
fn repeat_default() -> bool {
    true
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
        if self.collection == Collection::Library("power".into()) {
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
        self.collection = Collection::Library("power".into());
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
            self.value = entries.first().map(Entry::default_value).unwrap_or_default().into();
        }
        self.focus_ids.clear();
        let enabled = entries.iter().any(|e| Some(e.key()) == self.selected) && valid_value(&self.value);
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
                        pcbcraft_chrome::library_sources::heading(ui, "Libraries", collections(&sources).len());
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
                let field = properties::text(ui, "Value", &mut self.value);
                self.focus_ids.push(field.id);
                choices = self.results(ui, &entries, owner, short);
                choices.confirm |= ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && (choices.focused || self.focused(ui.ctx()))
                    && !egui::Popup::is_any_open(ui.ctx());
                ui.horizontal(|ui| {
                    let response = ui.button("Libraries…");
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Manage libraries…"));
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
        if action.primary && enabled && valid_value(&self.value) {
            if owner != source {
                self.close(ui.ctx());
                return Some(Pick::Stale);
            }
            let picked = entries.iter().find(|e| Some(e.key()) == self.selected).and_then(|entry| placement(entry, &self.value)).map(Pick::Place);
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
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, ui.is_enabled(), "Search parts"));
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
        let mut choices = collections(sources);
        if let Collection::Library(id) = &self.collection
            && !choices.iter().any(|(collection, _)| collection == &self.collection)
        {
            choices.push((self.collection.clone(), format!("{id} (unavailable)")));
        }
        let keys: Vec<_> = choices
            .iter()
            .map(|(choice, _)| match choice {
                Collection::All => "all".to_owned(),
                Collection::Library(id) => format!("library:{id}"),
            })
            .collect();
        let mut value = choices.iter().position(|(choice, _)| *choice == self.collection).map(|i| keys[i].clone()).unwrap_or_else(|| "all".into());
        let options: Vec<_> = keys.iter().zip(&choices).map(|(key, (_, name))| (key.as_str(), name.clone())).collect();
        properties::choice(ui, "Library", ui.scope_id().with("catalog_library"), &mut value, &options);
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
            let symbol = &entry.0;
            let identity = (entries.iter().filter(|other| other.name() == entry.name()).count() > 1)
                .then(|| format!("{}/{} r{}", symbol.library, symbol.id, symbol.revision()));
            let value = symbol.default_value();
            let response = catalog::tile(
                ui,
                view,
                width,
                catalog::Item { name: entry.name(), value, identity: identity.as_deref(), selected: self.selected == Some(entry.key()) },
                |p, r, ink| paint_entry(p, r, entry, ink),
            );
            let name = if self.selected == Some(entry.key()) { &self.value } else { entry.default_value() };
            if let Some(placement) = placement(entry, name) {
                response.dnd_set_drag_payload(crate::schematic_canvas::PartDrag { placement, source: owner });
                choices.dragging |= response.drag_started();
            }
            if response.clicked() || response.drag_started() {
                if self.selected != Some(entry.key()) {
                    self.value = entry.default_value().into();
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
        let symbol = &entry.0;
        properties::section(ui, "Symbol");
        let field = properties::text(ui, "Value", &mut self.value);
        self.focus_ids.push(field.id);
        properties::field(ui, "Pins", |ui| {
            ui.label(symbol.pins().len().to_string());
        });
        properties::section(ui, "Source");
        ui.label(format!("{} · r{}", collection_name(&Collection::Library(symbol.library.clone()), sources), symbol.revision()));
    }

    /// Render actual loaded library identities independently of search terms and display names.
    fn library_rows(&mut self, ui: &mut Ui, entries: &[Entry], sources: &serde_json::Value) {
        for (collection, name) in collections(sources) {
            let count = entries.iter().filter(|entry| entry.in_collection(&collection)).count();
            let response = ui.selectable_label(self.collection == collection, format!("{name}  {count}"));
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, self.collection == collection, name.as_str()));
            if response.clicked() {
                self.collection = collection;
                self.selected = None;
                self.reveal_selection = true;
            }
        }
    }
}

/// Click and drag capture the same editable Value and exact native definition.
fn placement(entry: &Entry, value: &str) -> Option<Placement> {
    valid_value(value).then(|| Placement { definition: entry.0.clone(), value: value.to_owned() })
}

/// Native drawings use one painter for bundled, folder and portable-bundle sources.
fn paint_entry(p: &egui::Painter, r: Rect, entry: &Entry, ink: egui::Color32) {
    paint_preview(p, &entry.0, r, ink);
}
/// Equivalent registrations share a library filter, while the source manager keeps each
/// registration inspectable. Display ordering uses library names, never source delivery kind.
fn collections(sources: &serde_json::Value) -> Vec<(Collection, String)> {
    let mut libraries = std::collections::BTreeMap::new();
    for source in sources.as_array().into_iter().flatten() {
        if let (Some(id), Some(name)) = (source["library"].as_str(), source["name"].as_str()) {
            libraries.entry(id.to_owned()).or_insert_with(|| name.to_owned());
        }
    }
    let mut choices: Vec<_> = libraries.into_iter().map(|(id, name)| (Collection::Library(id), name)).collect();
    choices.sort_by(|(_, a), (_, b)| a.cmp(b));
    choices.insert(0, (Collection::All, "All libraries".into()));
    choices
}

/// Resolve a library display name without changing its stable filter key.
fn collection_name(collection: &Collection, sources: &serde_json::Value) -> String {
    match collection {
        Collection::All => "All libraries".into(),
        Collection::Library(id) => {
            sources.as_array().into_iter().flatten().find(|s| s["library"].as_str() == Some(id)).and_then(|s| s["name"].as_str()).unwrap_or(id).into()
        }
    }
}
/// Placement values share the component validator's bounds before creating any gesture.
fn valid_value(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && value.chars().count() <= 64 && !value.chars().any(char::is_control)
}
/// Search current unambiguous source revisions without separately injecting bundled items.
fn entries(query: &str, catalog: &library::sources::Catalog) -> Vec<Entry> {
    matches_sources(query, catalog).into_iter().map(Entry).collect()
}
/// Qualified source identities, revisions and attribution survive into placement.
fn matches_sources(query: &str, catalog: &library::sources::Catalog) -> Vec<Arc<SymbolDefinition>> {
    let mut parts: Vec<_> = catalog.search(query, Some(library::sources::Kind::Symbol)).into_iter().filter_map(|(_, item)| item.symbol()).collect();
    let sources = catalog.inspect();
    parts.sort_by_cached_key(|part| collection_name(&Collection::Library(part.library.clone()), &sources));
    parts
}

#[cfg(test)]
mod tests;
