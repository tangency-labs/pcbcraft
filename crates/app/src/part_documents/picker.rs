//! Searchable reference selection over the same native catalog used by browsing and placement.
use super::*;
use crate::editor_layout::HostId;
use pcbcraft_chrome::{library_workspace::TableEntry, part_form as chrome};

/// Bound each rendered result page independently of the number of installed native definitions.
const PAGE_SIZE: usize = 50;
/// Only small result metadata is retained; native drawings stay shared in the catalog.
#[derive(Debug)]
struct Choice {
    key: Key,
    name: String,
}
/// A cache key invalidates result membership on every supported source/query/filter change.
#[derive(Debug, PartialEq, Eq)]
struct Query {
    generation: u64,
    text: String,
    library: String,
    older: bool,
}
/// A chooser captures its own document authority; browsing alone never edits a draft or project.
#[derive(Debug)]
pub(crate) struct State {
    document: u64,
    revision: u64,
    field: ReferenceField,
    host: HostId,
    query: String,
    library: String,
    older: bool,
    selected: Option<Key>,
    cache: Option<Query>,
    rows: Vec<Choice>,
    libraries: Vec<(String, String)>,
    page: usize,
    compact: usize,
    focus: bool,
    reveal: bool,
    error: Option<String>,
    symbol_camera: crate::library_preview::Camera,
    footprint_camera: crate::library_preview::Camera,
}
impl State {
    /// Retain the original exact revision as a selection, including an explicitly older revision.
    fn new(document: u64, revision: u64, field: ReferenceField, host: HostId, current: Option<Reference>, catalog: &Catalog) -> Self {
        let kind = field.kind();
        let older = current.as_ref().is_some_and(|r| catalog.highest_revision(&r.library, kind, &r.id) > r.revision);
        let selected = current.filter(|r| r.library != "user").map(|r| Key { library: r.library, id: r.id, revision: r.revision, kind });
        let mut state = Self {
            document,
            revision,
            field,
            host,
            query: String::new(),
            library: String::new(),
            older,
            selected,
            cache: None,
            rows: Vec::new(),
            libraries: Vec::new(),
            page: 0,
            compact: 0,
            focus: true,
            reveal: true,
            error: None,
            symbol_camera: Default::default(),
            footprint_camera: Default::default(),
        };
        state.refresh(catalog);
        state
    }
    /// The captured host owns presentation even when the originating part is parked elsewhere.
    pub(crate) fn host(&self) -> HostId {
        self.host
    }
    /// Rebuild small result metadata only after a query, source generation or filter changes.
    /// Selection does not survive changed membership; initial exact selection stays explicit.
    fn refresh(&mut self, catalog: &Catalog) {
        let key =
            Query { generation: catalog.generation(), text: self.query.trim().to_lowercase(), library: self.library.clone(), older: self.older };
        if self.cache.as_ref() == Some(&key) {
            return;
        }
        if self.cache.is_some() {
            self.selected = None;
            self.error = None;
            self.symbol_camera = Default::default();
            self.footprint_camera = Default::default();
        }
        if self.cache.as_ref().is_none_or(|old| old.generation != key.generation) {
            let loaded = catalog.inspect();
            let mut libraries = BTreeMap::new();
            for source in loaded.as_array().into_iter().flatten().filter(|source| source["available"] == true) {
                if let Some(id) = source["library"].as_str() {
                    libraries.entry(id.to_owned()).or_insert_with(|| source["name"].as_str().unwrap_or(id).to_owned());
                }
            }
            self.libraries = libraries.into_iter().collect();
        }
        let matches = if self.older {
            catalog.search_revisions(&self.query, Some(self.field.kind()))
        } else {
            catalog.search(&self.query, Some(self.field.kind()))
        };
        self.rows = matches
            .into_iter()
            .filter(|(_, item)| self.library.is_empty() || item.key.library == self.library)
            .map(|(_, item)| Choice { key: item.key.clone(), name: item.name.clone() })
            .collect();
        self.rows.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.key.library.cmp(&b.key.library))
                .then_with(|| a.key.id.cmp(&b.key.id))
                .then_with(|| b.key.revision.cmp(&a.key.revision))
        });
        if self.selected.as_ref().is_some_and(|key| !self.rows.iter().any(|row| row.key == *key)) {
            self.selected = None;
        }
        self.page = self.selected.as_ref().and_then(|key| self.rows.iter().position(|row| row.key == *key)).unwrap_or(0) / PAGE_SIZE;
        self.cache = Some(key);
    }
    /// Inspect at most one page without copying native geometry or constructing invisible rows.
    fn page(&self) -> &[Choice] {
        let start = (self.page * PAGE_SIZE).min(self.rows.len());
        &self.rows[start..(start + PAGE_SIZE).min(self.rows.len())]
    }
    /// A selected exact candidate must still be available, conflict-free and semantically usable.
    fn candidate(&self, catalog: &Catalog) -> Result<Reference, String> {
        let key = self.selected.as_ref().ok_or("Select a library item.")?;
        catalog.item(key)?;
        let reference = Reference { library: key.library.clone(), id: key.id.clone(), revision: key.revision };
        match key.kind {
            Kind::Category => {
                catalog.category(&reference)?;
            }
            Kind::Part => {
                catalog.resolve_part(&reference)?;
            }
            _ => {}
        }
        Ok(reference)
    }
    /// Selection resets preview navigation without modifying a draft or changing query membership.
    fn select(&mut self, index: usize) {
        if let Some(row) = self.rows.get(index) {
            self.selected = Some(row.key.clone());
            self.reveal = true;
            self.page = index / PAGE_SIZE;
            self.error = None;
            self.symbol_camera = Default::default();
            self.footprint_camera = Default::default();
        }
    }
    /// Arrow keys traverse the full matching set, revealing only its corresponding bounded page.
    fn step(&mut self, forward: bool) {
        if self.rows.is_empty() {
            return;
        }
        let current = self.selected.as_ref().and_then(|key| self.rows.iter().position(|row| row.key == *key));
        let index = match (current, forward) {
            (Some(i), true) => (i + 1).min(self.rows.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
            (None, true) => 0,
            (None, false) => self.rows.len() - 1,
        };
        self.select(index);
    }
}
impl PcbApp {
    /// Open reference browsing after capturing a stable package/field identity and draft revision.
    pub(crate) fn begin_part_reference(&mut self, ctx: &egui::Context, document: u64, field: ReferenceField) -> Result<(), String> {
        let doc = self.library_editors.parts.get(&document).ok_or("Unknown part document.")?;
        if !doc.pending.is_empty() {
            return Err("Apply or reset typed fields before choosing another definition.".into());
        }
        let current = doc.session.record().reference_field(&field)?;
        self.dialog =
            Dialog::PartReference(Box::new(State::new(document, doc.session.revision(), field, self.views.host, current, self.session.libraries())));
        ctx.request_repaint();
        Ok(())
    }
    /// GUI and MCP apply an exact reference through one revision-checked engine operation.
    pub(crate) fn choose_part_reference(
        &mut self,
        document: u64,
        revision: u64,
        field: &ReferenceField,
        reference: Option<Reference>,
    ) -> Result<(), String> {
        let doc = self.library_editors.parts.get_mut(&document).ok_or("The part document closed.")?;
        if !doc.pending.is_empty() {
            return Err("Apply or reset typed fields before choosing another definition.".into());
        }
        let candidate = doc.session.record().select_reference(field, reference, self.session.libraries())?;
        let changed = &candidate != doc.session.record();
        doc.session.replace(revision, candidate)?;
        if changed {
            if let ReferenceField::NewPackage { package } = field {
                doc.package = Some(package.clone());
            }
            doc.selected_pad = None;
            doc.symbol_camera = Default::default();
            doc.footprint_camera = Default::default();
        }
        Ok(())
    }
    /// Search/filter/preview state is disposable. Cancel, stale authority and disappearing sources
    /// never change an association; only Choose submits the captured engine edit.
    pub(crate) fn part_reference_dialog(&mut self, ctx: &egui::Context, mut state: State) {
        let Some(doc) = self.library_editors.parts.get(&state.document) else { return };
        let stale = doc.session.revision() != state.revision;
        let catalog = self.session.libraries();
        state.refresh(catalog);
        let candidate = state.candidate(catalog);
        let mut enter = false;
        let title = match state.field.kind() {
            Kind::Symbol => "Choose symbol",
            Kind::Footprint => "Choose footprint",
            Kind::Category => "Choose category",
            _ => "Choose generic part",
        };
        let action = chrome::chooser(ctx, title, !stale && candidate.is_ok(), |ui, height| {
            let search = chrome::search(
                ui,
                &mut state.query,
                match state.field.kind() {
                    Kind::Symbol => "Search symbols",
                    Kind::Footprint => "Search footprints",
                    Kind::Category => "Search categories",
                    _ => "Search generic parts",
                },
            );
            if state.focus {
                search.request_focus();
                state.focus = false;
            }
            let mut options = vec![("", "All libraries".into())];
            options.extend(state.libraries.iter().map(|(id, name)| (id.as_str(), name.clone())));
            properties::choice(ui, "Library", ui.scope_id().with("reference_library"), &mut state.library, &options);
            controls::checkbox(ui, &mut state.older, "Include older revisions");
            state.refresh(catalog);
            if !egui::Popup::is_any_open(ctx) {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
                    state.step(true);
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                    state.step(false);
                }
                enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            }
            ui.horizontal(|ui| {
                let start = (state.page * PAGE_SIZE).min(state.rows.len());
                let end = (start + PAGE_SIZE).min(state.rows.len());
                ui.label(if state.rows.is_empty() { "0 items".into() } else { format!("{}–{} of {}", start + 1, end, state.rows.len()) });
                if controls::enabled_button(ui, state.page > 0, "Previous page").clicked() {
                    state.page = state.page.saturating_sub(1);
                }
                if controls::enabled_button(ui, end < state.rows.len(), "Next page").clicked() {
                    state.page += 1;
                }
            });
            let mut compact = state.compact;
            chrome::chooser_body(ui, height, &mut compact, |ui, preview| {
                if preview {
                    preview_item(ui, &mut state, catalog);
                } else {
                    result_items(ui, &mut state);
                }
            });
            state.compact = compact;
            if stale {
                ui.colored_label(pcbcraft_chrome::Tokens::get().error, "The part changed. Cancel and reopen this chooser.");
            }
            if let Some(error) = &state.error {
                ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
            }
        });
        if action.close {
            return;
        }
        if action.primary || (enter && !stale) {
            match state
                .candidate(self.session.libraries())
                .and_then(|reference| self.choose_part_reference(state.document, state.revision, &state.field, Some(reference)))
            {
                Ok(()) => {
                    self.repaint_editors(ctx);
                    return;
                }
                Err(error) => state.error = Some(error),
            }
        }
        self.dialog = Dialog::PartReference(Box::new(state));
    }
}
/// Only the current page creates table cells. Exact identity/revision labels disambiguate names.
fn result_items(ui: &mut egui::Ui, state: &mut State) {
    let entries: Vec<_> = state
        .page()
        .iter()
        .map(|row| TableEntry {
            key: format!("Choose item {}/{} r{}", row.key.library, row.key.id, row.key.revision),
            cells: vec![row.name.clone(), row.key.library.clone(), format!("r{}", row.key.revision)],
            selected: state.selected.as_ref() == Some(&row.key),
            branch: None,
            nested: false,
        })
        .collect();
    let mut chosen = None;
    ui.push_id(state.page, |ui| {
        chosen = chrome::reference_results(ui, &entries, std::mem::take(&mut state.reveal));
    });
    if let Some(index) = chosen {
        state.select(state.page * PAGE_SIZE + index);
    }
}
/// Native previews inspect one exact candidate; metadata and missing dependencies stay explicit.
fn preview_item(ui: &mut egui::Ui, state: &mut State, catalog: &Catalog) {
    let Some(key) = &state.selected else {
        ui.label("No selection");
        return;
    };
    let item = match catalog.item(key) {
        Ok(item) => item,
        Err(error) => {
            ui.label(error);
            return;
        }
    };
    properties::section(ui, &item.name);
    let library = state.libraries.iter().find(|(id, _)| id == &key.library).map(|(_, name)| name.clone()).unwrap_or_else(|| key.library.clone());
    for (label, value) in [("Library", library), ("Revision", key.revision.to_string()), ("License", item.license.clone())] {
        properties::field(ui, label, |ui| {
            ui.add(egui::Label::new(&value).truncate()).on_hover_text(value);
        });
    }
    controls::collapsing(ui, "reference_details", "Details", false, None, |ui| {
        for (label, value) in [("Library ID", &key.library), ("Item ID", &key.id), ("Record", &item.file)] {
            properties::field(ui, label, |ui| {
                ui.add(egui::Label::new(value).truncate()).on_hover_text(value);
            });
        }
    });
    if let Some(symbol) = item.symbol() {
        crate::library_preview::workspace_symbol(ui, &symbol, &mut state.symbol_camera);
    }
    if let Some(footprint) = item.footprint() {
        crate::library_preview::workspace_footprint(ui, &footprint, &mut state.footprint_camera);
    }
    if let Some(category) = item.category() {
        for (id, field) in &category.fields {
            properties::field(ui, &field.name, |ui| {
                ui.label(id);
            });
        }
    }
    if let Some(part) = item.part() {
        match catalog.resolve_part(&part.reference()) {
            Ok(resolved) => {
                if let Some(symbol) = &resolved.symbol {
                    crate::library_preview::workspace_symbol(ui, symbol, &mut state.symbol_camera);
                }
                if let Some(footprint) = resolved.footprints.values().next() {
                    crate::library_preview::workspace_footprint(ui, footprint, &mut state.footprint_camera);
                }
                ui.label(format!("{} packages", part.packages.len()));
            }
            Err(error) => {
                ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
            }
        }
    }
    if let Err(error) = state.candidate(catalog) {
        ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
    }
}

#[cfg(test)]
mod tests;
