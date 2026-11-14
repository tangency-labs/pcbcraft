//! Generic and orderable part documents live inside the existing Parts Library workspace.
mod form;
pub(crate) mod picker;
mod publishing;
#[cfg(test)]
mod tests;
use crate::{Dialog, PcbApp};
use pcbcraft_chrome::{controls, properties};
use pcbcraft_engine::{
    library::{
        parts::Reference,
        sources::{Catalog, Key, Kind},
    },
    part_authoring::{Record, ReferenceField, Session},
};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::Value;
use std::collections::BTreeMap;

/// Authored records and field input survive pane changes until publication or explicit discard.
#[derive(Debug)]
pub(crate) struct Document {
    pub(crate) session: Session,
    pub(crate) source: Option<String>,
    pub(crate) pending: BTreeMap<String, String>,
    package: Option<String>,
    selected_pad: Option<String>,
    error: Option<String>,
    symbol_camera: crate::library_preview::Camera,
    footprint_camera: crate::library_preview::Camera,
}
impl Document {
    /// Admit a bounded session document while keeping navigation and invalid field input disposable.
    pub(crate) fn new(record: Record, published: bool, source: Option<String>) -> Result<Self, String> {
        let package = match &record {
            Record::Part(p) => p.packages.keys().next().cloned(),
            Record::Orderable(p) => Some(p.package.clone()),
        };
        Ok(Self {
            session: Session::new(record, published)?,
            source,
            pending: BTreeMap::new(),
            package,
            selected_pad: None,
            error: None,
            symbol_camera: Default::default(),
            footprint_camera: Default::default(),
        })
    }
    /// Invalid typed input participates in close/deletion protection even before it is applied.
    pub(crate) fn dirty(&self) -> bool {
        self.session.dirty() || !self.pending.is_empty()
    }
}
impl PcbApp {
    /// Enter the existing workspace without replacing another retained part draft.
    pub(crate) fn place_part_document(&mut self, ctx: &egui::Context, id: u64) -> Result<(), String> {
        if !self.library_editors.parts.contains_key(&id) {
            return Err("Unknown part document.".into());
        }
        self.place_library_workspace(ctx, pcbcraft_chrome::workspace::Placement::Here)?;
        self.views.active.library.as_mut().ok_or("Parts Library pane is unavailable.")?.part_document = Some(id);
        Ok(())
    }
    /// Create a reusable definition without requiring a schematic or physical association.
    pub(crate) fn new_part_document(&mut self, ctx: &egui::Context, symbol: Option<Reference>) -> Result<u64, String> {
        if let Some(symbol) = &symbol
            && self.session.libraries().symbol(&symbol.library, &symbol.id, symbol.revision).is_none()
        {
            return Err("Published symbol is unavailable or conflicting.".into());
        }
        let record = Record::new_part(symbol)?;
        let source = self.preferred_library_source();
        let id = self.library_editors.open_part(record, false, source)?;
        self.place_part_document(ctx, id)?;
        Ok(id)
    }
    /// Reopen exact source records or copy a project pin; no project revision is adopted here.
    pub(crate) fn edit_part_selection(&mut self, ctx: &egui::Context, copy: bool) -> Result<(), String> {
        let state = self.views.active.library.as_ref().ok_or("Choose a Parts Library pane.")?;
        let key = state.selected_key().ok_or("Select a part.")?.clone();
        if state.project_id().is_some_and(|id| self.session.project().map(|p| p.id()) != Some(id)) {
            return Err("Project selection is no longer current.".into());
        }
        if !copy && !state.source_id().is_some_and(|source| self.writable_libraries().iter().any(|(id, _)| id == source)) {
            return Err("This library is read-only; use Edit copy.".into());
        }
        let record = if state.project_id().is_some() {
            let inventory = self.project_library_inventory()?;
            let item = inventory.item(&key).ok_or("Pinned part is unavailable.")?;
            if let Some(p) = item.part() {
                Record::Part((*p).clone())
            } else if let Some(p) = item.manufacturer_part() {
                Record::Orderable((*p).clone())
            } else {
                return Err("Select a generic or orderable part.".into());
            }
        } else {
            if self.session.libraries().conflicting(&key) {
                return Err("Resolve the conflicting revision before editing.".into());
            }
            catalog_record(self.session.libraries(), &key)?
        };
        let record = if copy { record.copy()? } else { record };
        let source = self.preferred_library_source();
        let id = self.library_editors.open_part(record, !copy, source)?;
        self.place_part_document(ctx, id)
    }
    /// Open an orderable form only after its exact owning part resolves from published sources.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new_orderable_document(&mut self, ctx: &egui::Context, part: Reference) -> Result<u64, String> {
        let record = Record::new_orderable(self.session.libraries(), part)?;
        let source = self.preferred_library_source();
        let id = self.library_editors.open_part(record, false, source)?;
        self.place_part_document(ctx, id)?;
        Ok(id)
    }
    /// Browse and open part documents share one workspace; closing a tab asks before discarding.
    pub(crate) fn part_document_tabs(&mut self, ui: &mut egui::Ui, state: &mut crate::library_workspace::State) {
        if state.part_document.is_some_and(|id| !self.library_editors.parts.contains_key(&id)) {
            state.part_document = None;
        }
        let mut current = state.part_document.map(|id| id.to_string()).unwrap_or_default();
        let mut choices = vec![(String::new(), "Browse".to_owned())];
        choices.extend(
            self.library_editors
                .parts
                .iter()
                .map(|(id, doc)| (id.to_string(), format!("{}{}", doc.session.record().name(), if doc.dirty() { " *" } else { "" }))),
        );
        if choices.len() == 1 {
            return;
        }
        ui.horizontal(|ui| {
            let options: Vec<_> = choices.iter().map(|(id, label)| (id.as_str(), label.clone())).collect();
            let width = (ui.available_width() - 150.).clamp(120., 600.);
            ui.allocate_ui_with_layout(
                egui::vec2(width, pcbcraft_chrome::Tokens::get().control_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    properties::choice(ui, "Document", ui.scope_id().with("part_document"), &mut current, &options);
                },
            );
            state.part_document = current.parse().ok();
            if let Some(id) = state.part_document
                && controls::button(ui, "Close document").clicked()
            {
                self.dialog = Dialog::LibraryClose(Some(id));
            }
        });
    }
    /// GUI and MCP replace the same bounded engine draft at its captured document revision.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn replace_part_document(&mut self, id: u64, revision: u64, record: Record) -> Result<(), String> {
        let doc = self.library_editors.parts.get_mut(&id).ok_or("Unknown part document.")?;
        if !doc.pending.is_empty() {
            return Err("Apply or reset typed fields before replacing the part draft.".into());
        }
        doc.session.replace(revision, record)
    }
}
/// Exact selection never silently falls back to the latest record or another source.
fn catalog_record(catalog: &Catalog, key: &Key) -> Result<Record, String> {
    if catalog.conflicting(key) {
        return Err("Resolve this part revision conflict before editing.".into());
    }
    let item = catalog.item(key)?;
    if let Some(part) = item.part() {
        return Ok(Record::Part((*part).clone()));
    }
    if let Some(part) = item.manufacturer_part() {
        return Ok(Record::Orderable((*part).clone()));
    }
    Err("Select a generic or orderable part.".into())
}
/// Display only the selected native association; catalog enumeration belongs to its chooser.
fn reference_name(catalog: &Catalog, reference: Option<&Reference>, kind: Kind) -> String {
    let Some(reference) = reference else { return "None".into() };
    let key = Key { library: reference.library.clone(), id: reference.id.clone(), revision: reference.revision, kind };
    catalog.item(&key).map(|item| format!("{} · {} · r{}", item.name, key.library, key.revision)).unwrap_or_else(|_| {
        if reference.library == "user" { "None selected".into() } else { format!("Unavailable · {} · r{}", reference.id, reference.revision) }
    })
}

#[cfg(not(target_arch = "wasm32"))]
impl PcbApp {
    /// Live part operations address explicit documents; project session history is untouched.
    pub(crate) fn part_document_command(&mut self, ctx: &egui::Context, id: &str, params: &Value) -> Result<(), String> {
        let document = || params["document"].as_u64().ok_or("A part document ID is required.");
        let revision = || params["expected_revision"].as_u64().ok_or("A part document revision is required.");
        match id {
            "app.part_new" => {
                let symbol = params.get("symbol").map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string())).transpose()?;
                self.new_part_document(ctx, symbol)?;
            }
            "app.part_orderable" => {
                let part = serde_json::from_value(params["part"].clone()).map_err(|e| e.to_string())?;
                self.new_orderable_document(ctx, part)?;
            }
            "app.part_edit" => {
                let record = serde_json::from_value(params["record"].clone()).map_err(|e| e.to_string())?;
                self.replace_part_document(document()?, revision()?, record)?;
            }
            "app.part_reference" => {
                let field: ReferenceField = serde_json::from_value(params["field"].clone()).map_err(|e| e.to_string())?;
                let reference = params.get("reference").map(|value| serde_json::from_value(value.clone()).map_err(|e| e.to_string())).transpose()?;
                self.choose_part_reference(document()?, revision()?, &field, reference)?;
            }
            "app.part_history" => {
                let doc = self.library_editors.parts.get_mut(&document()?).ok_or("Unknown part document.")?;
                if !doc.pending.is_empty() {
                    return Err("Apply or reset typed fields before changing history.".into());
                }
                doc.session.history(revision()?, params["redo"] == true)?;
            }
            "app.part_focus" => self.place_part_document(ctx, document()?)?,
            _ => return Err("Unknown part document command.".into()),
        }
        self.repaint_editors(ctx);
        Ok(())
    }
}

/// A publication offers exact dependency updates without adopting any project revision.
#[derive(Debug, serde::Serialize)]
pub(crate) struct DependencyUpdate {
    kind: Kind,
    old: Reference,
    new: Reference,
    part: Reference,
    name: String,
}
impl PcbApp {
    /// Capture latest loaded generic parts still referencing the published drawing's prior revision.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn queue_part_dependency_updates(&mut self, prepared: &pcbcraft_engine::authoring::publishing::Prepared) {
        use pcbcraft_engine::authoring::publishing::Publication;
        let old = prepared.previous();
        let new = prepared.reference();
        let kind = prepared.kind();
        if old.library != new.library || old.id != new.id || old.revision >= new.revision {
            return;
        }
        for (_, item) in self.session.libraries().search("", Some(Kind::Part)) {
            let Some(part) = item.part() else { continue };
            if Record::Part((*part).clone()).advance_dependency(kind, &old, &new).is_ok() {
                self.part_updates.push(DependencyUpdate {
                    kind,
                    old: old.clone(),
                    new: new.clone(),
                    part: part.reference(),
                    name: part.name.clone(),
                });
            }
        }
    }
    /// Open a source-preserving part revision draft with the chosen dependency advanced; invalid
    /// maps stay visible for correction and cannot publish until the ordinary validator succeeds.
    pub(crate) fn open_part_dependency_update(&mut self, ctx: &egui::Context, index: usize) -> Result<(), String> {
        let update = self.part_updates.get(index).ok_or("Part update is no longer available.")?;
        let key = Key { library: update.part.library.clone(), id: update.part.id.clone(), revision: update.part.revision, kind: Kind::Part };
        let record = catalog_record(self.session.libraries(), &key)?;
        let candidate = record.advance_dependency(update.kind, &update.old, &update.new)?;
        let source = self.preferred_library_source();
        let mut session = Session::new(record.clone(), true)?;
        session.replace(session.revision(), candidate.clone())?;
        let id = self.library_editors.open_part(record, true, source)?;
        self.library_editors.parts.get_mut(&id).ok_or("Part document closed.")?.session = session;
        self.part_updates.remove(index);
        self.place_part_document(ctx, id)
    }
    /// A compact ordinary disclosure keeps the update offer present until reviewed or dismissed.
    pub(crate) fn part_update_actions(&mut self, ui: &mut egui::Ui) {
        if self.part_updates.is_empty() {
            return;
        }
        let mut selected = None;
        let mut dismiss = false;
        controls::collapsing(
            ui,
            "part_updates",
            &format!("{} part definitions reference earlier drawings", self.part_updates.len()),
            true,
            None,
            |ui| {
                for (index, update) in self.part_updates.iter().enumerate() {
                    if controls::button(ui, format!("Review {} · r{} → r{}", update.name, update.old.revision, update.new.revision)).clicked() {
                        selected = Some(index);
                    }
                }
                dismiss = controls::button(ui, "Dismiss updates").clicked();
            },
        );
        if dismiss {
            self.part_updates.clear();
        } else if let Some(index) = selected {
            self.message = self.open_part_dependency_update(ui.ctx(), index).err();
            if self.message.is_none() {
                self.pending_part_navigation = self.views.active.library.as_ref().and_then(|state| state.part_document);
            }
        }
    }
    /// Geometry editors expose computed users and the same part document creation path.
    pub(crate) fn part_drawing_actions(&mut self, ui: &mut egui::Ui, document: u64) {
        let Some((kind, address)) = self.library_editors.published_drawing(document) else { return };
        let used: Vec<_> = self
            .session
            .libraries()
            .search("", Some(Kind::Part))
            .into_iter()
            .filter_map(|(_, item)| item.part())
            .filter(|part| match kind {
                Kind::Symbol => part.symbol.as_ref() == Some(&address),
                Kind::Footprint => part.packages.values().any(|package| package.footprint == address),
                _ => false,
            })
            .map(|part| part.name.clone())
            .collect();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Used by: {}", if used.is_empty() { "No loaded parts".into() } else { used.join(", ") }));
            if kind == Kind::Symbol && controls::button(ui, "Create part from this symbol").clicked() {
                self.message = self.new_part_document(ui.ctx(), Some(address.clone())).map(|_| ()).err();
            }
        });
        self.part_update_actions(ui);
    }
}
