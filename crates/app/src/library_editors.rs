//! Library definitions are document workspaces, separate from project instances and history.
mod canvas;
pub(super) mod editing;
mod form;
mod io;
pub(super) mod publishing;
#[cfg(test)]
mod tests;
mod views;
use crate::{Dialog, IN_BROWSER, Navigation, PcbApp};
use egui::{Context, Ui};
use pcbcraft_chrome::menu::CheckKind;
use pcbcraft_chrome::{
    authoring,
    menu::{self, MenuIcon},
    properties, shell,
};
use pcbcraft_engine::authoring::{Action, Item, Kind, Session};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A bounded set of explicit authoring documents survives returning to the project workspace.
#[derive(Debug, Default)]
pub(super) struct Editors {
    documents: BTreeMap<u64, Document>,
    pub(crate) parts: BTreeMap<u64, crate::part_documents::Document>,
    pub(super) active: Option<u64>,
    next_id: u64,
    last_focused: [Option<u64>; 2],
    last_active: Option<u64>,
    files: io::Files,
    pub(crate) publisher: publishing::Publisher,
    pub(crate) notice: Option<String>,
    pending_document: Option<u64>,
    open_target: Option<crate::ViewId>,
}
#[derive(Debug)]
struct Document {
    session: Session,
    publish_source: Option<String>,
    /// The library reference preview is derived once per authoring revision.
    reference_preview: Option<(u64, std::sync::Arc<[Vec<pcbcraft_engine::schematic::Point>]>)>,
    bound_view: Option<crate::ViewId>,
    presentations: BTreeMap<crate::ViewId, views::Presentation>,
    selected: Option<u64>,
    selection: std::collections::BTreeSet<u64>,
    prototype: Option<pcbcraft_engine::authoring::Primitive>,
    drawing_purpose: pcbcraft_engine::board::definition::GraphicPurpose,
    drawing_stroke_nm: i64,
    drawing_fill: pcbcraft_engine::library_graphics::Fill,
    array_requested: bool,
    details_requested: bool,
    draft: Option<form::Draft>,
    camera: crate::canvas_camera::Camera,
    collapsed: [bool; 2],
    tool: canvas::Tool,
    ribbon_tab: usize,
    error: Option<String>,
}
impl Editors {
    /// Published drawing identity is enough for used-by inspection and creating a part association.
    pub(crate) fn published_drawing(&self, id: u64) -> Option<(pcbcraft_engine::library::sources::Kind, pcbcraft_engine::library::parts::Reference)> {
        let doc = self.documents.get(&id)?;
        let address = doc.session.published()?;
        Some((
            if doc.session.snapshot().kind == Kind::Symbol {
                pcbcraft_engine::library::sources::Kind::Symbol
            } else {
                pcbcraft_engine::library::sources::Kind::Footprint
            },
            pcbcraft_engine::library::parts::Reference { library: address.library.clone(), id: address.id.clone(), revision: address.revision },
        ))
    }

    /// Part and drawing documents share a bounded runtime ID namespace.
    pub(crate) fn open_part(
        &mut self,
        record: pcbcraft_engine::part_authoring::Record,
        published: bool,
        source: Option<String>,
    ) -> Result<u64, String> {
        if self.parts.len() + self.documents.len() >= 8 {
            return Err("Close a library document before opening another (maximum eight).".into());
        }
        if self.parts.values().any(|doc| {
            doc.session.record().kind() == record.kind()
                && doc.session.record().reference().library == record.reference().library
                && doc.session.record().reference().id == record.reference().id
        }) {
            return Err("This part is already open; its current edits were kept.".into());
        }
        let id = self.next_id.checked_add(1).ok_or("Library document ID limit reached.")?;
        let doc = crate::part_documents::Document::new(record, published, source)?;
        self.parts.insert(id, doc);
        self.next_id = id;
        Ok(id)
    }
    /// Publication/creation completions retain their original document, across both kinds.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn document_revision(&self, id: u64) -> Option<u64> {
        self.parts.get(&id).map(|doc| doc.session.revision()).or_else(|| self.documents.get(&id).map(|doc| doc.session.revision()))
    }
    #[cfg(not(target_arch = "wasm32"))]
    /// Retain a host-confirmed destination suggestion on the owning part document.
    pub(crate) fn set_part_destination(&mut self, id: u64, source: String) {
        if let Some(doc) = self.parts.get_mut(&id) {
            doc.source = Some(source);
        }
    }
    /// Unpublished form input and uncertain writes require an explicit discard decision.
    pub(crate) fn close_part(&mut self, id: u64, discard: bool) -> Result<(), String> {
        if self.busy() {
            return Err("Finish the library operation before closing.".into());
        }
        let doc = self.parts.get(&id).ok_or("Unknown part document.")?;
        if (doc.dirty() || self.publisher.unconfirmed() == Some(id)) && !discard {
            return Err("Part document has unpublished changes.".into());
        }
        self.parts.remove(&id);
        self.publisher.discard(id);
        Ok(())
    }

    /// Destination suggestions are document-local and do not bind an identity before publication.
    pub(crate) fn suggest_destination(&mut self, id: u64, source: Option<String>) {
        if let Some(doc) = self.documents.get_mut(&id)
            && doc.publish_source.is_none()
            && doc.session.published().is_none()
        {
            doc.publish_source = source;
        }
    }
    /// Shared service notices remain visible when creation finishes in the Parts Library workspace.
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
    /// An uncertain publication needs a close decision even when its separate draft file was saved.
    fn document_unsaved(&self, id: u64) -> bool {
        self.publisher.unconfirmed() == Some(id)
            || self.parts.get(&id).is_some_and(crate::part_documents::Document::dirty)
            || self.documents.get(&id).is_some_and(Document::unsaved)
    }
    /// Unpublished geometry and unapplied fields both prevent deleting their origin library.
    pub(crate) fn unpublished_in(&self, library: &str) -> bool {
        self.publisher.unconfirmed_in(library)
            || self.parts.values().any(|doc| doc.session.record().reference().library == library && doc.dirty())
            || self.documents.values().any(|doc| {
                doc.session.snapshot().library == library && (doc.unsaved() || (doc.session.published().is_some() && doc.session.publication_dirty()))
            })
    }
    /// A bounded change stamp wakes all views when a definition edit or dirty-state transition occurs.
    pub(super) fn stamp(&self) -> Vec<(u64, u64, bool)> {
        self.documents
            .iter()
            .map(|(id, doc)| (*id, doc.session.revision(), self.document_unsaved(*id)))
            .chain(self.parts.iter().map(|(id, doc)| (*id, doc.session.revision(), self.document_unsaved(*id))))
            .collect()
    }
    /// The active document determines its workspace, independent of the parked project view.
    pub(super) fn active_kind(&self) -> Option<Kind> {
        self.active.and_then(|id| self.documents.get(&id)).map(|doc| doc.session.snapshot().kind)
    }
    /// Validate before focus changes, retain document-local drafts/cameras and cancel only
    /// an unfinished pointer gesture. Switching never changes a definition or project revision.
    pub(super) fn focus(&mut self, id: Option<u64>) -> Result<(), String> {
        if id.is_some_and(|id| !self.documents.contains_key(&id)) {
            return Err("Unknown library document ID.".into());
        }
        if let Some(old) = self.active.and_then(|id| self.documents.get_mut(&id)) {
            old.tool = canvas::Tool::Select;
        }
        if let Some(id) = id {
            let kind = self.documents[&id].session.snapshot().kind;
            self.last_focused[usize::from(kind == Kind::Footprint)] = Some(id);
            self.last_active = Some(id);
        }
        self.active = id;
        Ok(())
    }
    /// Enter a definition workspace by resuming its last document or creating its first one.
    /// Failed creation, including the document limit, retains the previous workspace.
    pub(super) fn enter(&mut self, kind: Kind) -> Result<(), String> {
        if self.active_kind() == Some(kind) {
            return Ok(());
        }
        let id = self.last_focused[usize::from(kind == Kind::Footprint)]
            .filter(|id| self.documents.contains_key(id))
            .or_else(|| self.documents.iter().rev().find(|(_, doc)| doc.session.snapshot().kind == kind).map(|(id, _)| *id));
        match id {
            Some(id) => self.focus(Some(id)),
            None => self.open(Item::new(kind)?, false).map(|_| ()),
        }
    }

    /// Return to retained documents without changing their selection or pending property input.
    pub(super) fn resume(&mut self) {
        let id = self.last_active.filter(|id| self.documents.contains_key(id)).or_else(|| self.documents.keys().next_back().copied());
        let _ = self.focus(id);
    }
    pub(super) fn has_documents(&self) -> bool {
        !self.documents.is_empty() || !self.parts.is_empty()
    }
    /// The browser picker starts during the triggering interaction; native pickers use a worker.
    pub(super) fn open_picker(&mut self, ctx: &Context) -> Result<(), String> {
        self.files.start(ctx, None)
    }
    /// Include unapplied field input in leave/close protection, independently of the project.
    pub(super) fn unsaved(&self) -> bool {
        self.publisher.unconfirmed().is_some()
            || self.documents.values().any(Document::unsaved)
            || self.parts.values().any(crate::part_documents::Document::dirty)
    }
    /// Only one bounded picker/publication owns resources at a time.
    pub(super) fn busy(&self) -> bool {
        self.files.busy() || self.publisher.busy()
    }
    /// Inspection identifies each document and its own revision, never the project revision.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inspect(&self) -> Value {
        json!({"part_documents":self.parts.iter().map(|(id, doc)|json!({"document":id,"state":doc.session.inspect(),"pending_fields":doc.pending,"source":doc.source})).collect::<Vec<_>>(),"active":self.active,"file_operation":self.busy(),"publication":self.publisher.inspect(),"notice":self.notice,"documents":self.documents.iter().map(|(id,doc)|json!({"document":id,"publish_source":doc.publish_source,"state":doc.session.inspect(),"selected":doc.selected,"selection":doc.selection,"pending_fields":doc.pending(),"view":{"ribbon_tab":(["draw","library","view"][doc.ribbon_tab]),"browser":!doc.collapsed[0],"properties":!doc.collapsed[1],"zoom_percent":(doc.camera.zoom()*100.0).round(),"center":doc.camera.center(),"grid_spacing_nm":doc.camera.snap_nm()}})).collect::<Vec<_>>()})
    }
    /// Admit a validated item with fresh runtime identity; duplicate portable IDs never replace
    /// unsaved open documents. Opening files and forks use this same ownership boundary.
    fn open(&mut self, item: Item, saved: bool) -> Result<u64, String> {
        if self.documents.len() >= 4 || self.documents.len() + self.parts.len() >= 8 {
            return Err("Close a library document before opening another (maximum four).".into());
        }
        if self.documents.values().any(|doc| {
            let old = doc.session.snapshot();
            old.kind == item.kind && old.library == item.library && old.id == item.id
        }) {
            return Err("This library item is already open; its current edits were kept.".into());
        }
        let id = self.next_id.checked_add(1).ok_or("Library document ID limit reached.")?;
        let kind = item.kind;
        let mut camera = crate::canvas_camera::Camera::default();
        camera.set_snap_nm(if kind == Kind::Symbol { 1_270_000 } else { 100_000 });
        camera.request_fit();
        self.documents.insert(
            id,
            Document {
                reference_preview: None,
                session: Session::new(item, saved)?,
                publish_source: None,
                bound_view: None,
                presentations: BTreeMap::new(),
                selected: None,
                selection: Default::default(),
                prototype: None,
                drawing_purpose: pcbcraft_engine::board::definition::GraphicPurpose::Fabrication,
                drawing_stroke_nm: 100_000,
                drawing_fill: Default::default(),
                array_requested: false,
                details_requested: false,
                draft: None,
                camera,
                collapsed: [false, false],
                tool: canvas::Tool::Select,
                ribbon_tab: 0,
                error: None,
            },
        );
        self.next_id = id;
        self.focus(Some(id))?;
        self.pending_document = Some(id);
        Ok(id)
    }
    /// GUI and MCP edits share this document/revision check and engine transaction path.
    fn edit(&mut self, id: u64, revision: u64, action: Action) -> Result<(), String> {
        self.documents.get_mut(&id).ok_or("Unknown library document ID.")?.execute(revision, action)
    }
    /// Close only an explicitly addressed document. Discard is a conscious user/command choice;
    /// in-flight file owners cannot be destroyed or silently rebound to another document.
    fn close(&mut self, id: u64, discard: bool) -> Result<(), String> {
        if self.parts.contains_key(&id) {
            return self.close_part(id, discard);
        }
        if self.busy() {
            return Err("Finish the library file operation before closing a document.".into());
        }
        let doc = self.documents.get(&id).ok_or("Unknown library document ID.")?;
        if (doc.unsaved() || self.publisher.unconfirmed() == Some(id)) && !discard {
            return Err("Library item has unsaved changes.".into());
        }
        self.documents.remove(&id);
        self.publisher.discard(id);
        if self.active == Some(id) {
            self.focus(self.documents.keys().next_back().copied())?;
        }
        Ok(())
    }
    /// File completion affects its captured document and snapshot only. Newer edits remain dirty.
    pub(super) fn poll(&mut self) {
        let Some(result) = self.files.poll() else { return };
        if !matches!(&result, Ok(io::Outcome::Opened(_))) {
            self.open_target = None;
        }
        let result = match result {
            Ok(io::Outcome::Opened(item)) => self.open(*item, true).map(|_| "Opened library item".into()),
            #[cfg(not(target_arch = "wasm32"))]
            Ok(io::Outcome::Saved(id, snapshot)) => {
                if let Some(doc) = self.documents.get_mut(&id) {
                    doc.session.mark_saved(snapshot);
                }
                Ok("Saved library copy".into())
            }
            Ok(io::Outcome::Exported) => Ok("Exported library item".into()),
            Ok(io::Outcome::Cancelled) => return,
            Err(error) => Err(error),
        };
        self.notice = Some(result.unwrap_or_else(|error| error));
    }
    /// Capture immutable authoring state; unapplied field input must be resolved before saving.
    fn save(&mut self, ctx: &Context, id: u64, export: bool) -> Result<(), String> {
        let doc = self.documents.get(&id).ok_or("Unknown library document ID.")?;
        if doc.pending_any() {
            return Err("Apply or reset the property fields before saving.".into());
        }
        self.files.start(ctx, Some((id, doc.session.snapshot(), export)))
    }
}
impl Document {
    /// Unapplied numeric text participates in close protection even when parsing failed.
    fn pending(&self) -> bool {
        self.draft.as_ref().is_some_and(form::Draft::dirty)
    }
    /// Both committed definition changes and retained form input require a close decision.
    fn unsaved(&self) -> bool {
        self.session.dirty() || self.pending_any()
    }
    /// Selection cannot discard unfinished fields. Multiple objects retain their stable IDs,
    /// while a single object remains the scalar inspector's target.
    fn select(&mut self, id: Option<u64>) -> Result<(), String> {
        self.select_many(id.into_iter().collect())
    }
    /// Validate a whole selection before changing its primary inspector target or cancelling a tool.
    fn select_many(&mut self, ids: std::collections::BTreeSet<u64>) -> Result<(), String> {
        if ids.iter().any(|id| !self.session.snapshot().objects.contains_key(id)) {
            return Err("Unknown library primitive ID.".into());
        }
        if ids != self.selection {
            if self.pending() {
                let error = "Apply or reset the property fields before changing selection.".to_owned();
                self.error = Some(error.clone());
                return Err(error);
            }
            self.selection = ids;
            self.selected = if self.selection.len() == 1 { self.selection.first().copied() } else { None };
            self.draft = None;
            self.tool = canvas::Tool::Select;
            self.error = None;
        }
        Ok(())
    }
    /// Validate a shared transaction and invalidate cached forms/tools only after success.
    /// Selecting a unit clears primitive selection rather than treating its existing objects as inserts.
    fn execute(&mut self, revision: u64, action: Action) -> Result<(), String> {
        if self.pending() {
            return Err("Apply or reset the property fields before editing the document.".into());
        }
        let selecting_unit = matches!(&action, Action::SelectUnit { .. } | Action::AddUnit { .. });
        let old = self.session.snapshot();
        self.session.execute(revision, action)?;
        let next = self.session.snapshot();
        let inserted: std::collections::BTreeSet<_> = next.objects.keys().filter(|id| !old.objects.contains_key(id)).copied().collect();
        self.selection.retain(|id| next.objects.contains_key(id));
        if selecting_unit {
            self.selection.clear();
        } else if !inserted.is_empty() {
            self.selection = inserted;
        }
        self.selected = if self.selection.len() == 1 { self.selection.first().copied() } else { None };
        self.draft = None;
        self.tool = canvas::Tool::Select;
        if self.selected.is_some_and(|id| !self.session.snapshot().objects.contains_key(&id)) {
            self.selected = None;
        }
        self.error = None;
        Ok(())
    }
    /// A form retains invalid input and its original revision until Apply or Reset.
    fn properties(&mut self, ui: &mut Ui, expanded: bool) {
        properties::inspector_body(ui);
        let item = self.session.snapshot();
        if item.kind == Kind::Symbol {
            let mut action = None;
            ui.horizontal(|ui| {
                let label = ui.label("Unit");
                egui::ComboBox::from_id_salt("symbol_unit")
                    .selected_text(&item.unit)
                    .show_ui(ui, |ui| {
                        for unit in std::iter::once(&item.unit).chain(item.other_units.keys()) {
                            if ui.selectable_label(unit == &item.unit, unit).clicked() && unit != &item.unit {
                                action = Some(Action::SelectUnit { unit: unit.clone() });
                            }
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if pcbcraft_chrome::controls::enabled_button(ui, item.other_units.len() < 31, "Add unit")
                    .on_hover_text("Add schematic unit")
                    .clicked()
                {
                    let id = (1..=32).map(|n| format!("unit-{n}")).find(|id| id != &item.unit && !item.other_units.contains_key(id));
                    if let Some(unit) = id {
                        action = Some(Action::AddUnit { unit });
                    }
                }
            });
            if let Some(action) = action {
                if self.pending() {
                    self.error = Some("Apply or reset the property fields before changing units.".into());
                } else if let Err(error) = self.execute(self.session.revision(), action) {
                    self.error = Some(error);
                }
            }
        }
        if self.selection.len() > 1 {
            properties::identity(ui, &format!("{} objects", self.selection.len()), &self.session.snapshot().name);
            return;
        }
        if self.draft.as_ref().is_some_and(|draft| draft.revision != self.session.revision()) {
            if self.pending() {
                self.error = Some("The library document changed; reset these fields before applying new edits.".into());
            } else {
                self.draft = None;
                self.error = Some("The library document changed; property fields were refreshed.".into());
            }
        }
        let snapshot = self.session.snapshot();
        let draft = self.draft.get_or_insert_with(|| form::Draft::new(&snapshot, self.selected, self.session.revision()));
        properties::identity(ui, &form::title(draft), &snapshot.name);
        if let Some(error) = &self.error {
            ui.label(shell::error(error));
        }
        draft.show(ui, expanded);
        if expanded
            && self.selected.is_none()
            && let Some(source) = &snapshot.original_source
        {
            ui.collapsing("Original source", |ui| {
                ui.label(&source.kind);
                ui.add(egui::Label::new(&source.reference).wrap());
                ui.add(egui::Label::new(&source.locator).wrap());
            });
        }
        let (mut apply, mut reset) = (false, false);
        ui.horizontal(|ui| {
            apply = pcbcraft_chrome::controls::enabled_button(ui, draft.dirty(), "Apply").clicked();
            reset = pcbcraft_chrome::controls::button(ui, "Reset").clicked();
        });
        if apply {
            let result = draft.action().and_then(|action| self.session.execute(draft.revision, action));
            match result {
                Ok(_) => {
                    self.draft = None;
                    self.tool = canvas::Tool::Select;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        } else if reset {
            self.draft = None;
            self.error = None;
        }
    }
    /// Match project Browser row metrics, selection and disclosure behavior. Primitive identities
    /// stay stable while their electrical numbers change; folding never changes the definition.
    fn browser(&mut self, ui: &mut Ui) {
        use pcbcraft_chrome::workspace::{TreeIcon, TreeRow, tree_row};
        let item = self.session.snapshot();
        let fold_id = ui.make_persistent_id("definition_browser");
        let open = ui.ctx().data(|data| data.get_temp::<bool>(fold_id).unwrap_or(true));
        let root = tree_row(
            ui,
            fold_id,
            &TreeRow {
                depth: 0,
                fold: Some(open),
                icon: Some(if item.kind == Kind::Symbol { TreeIcon::Sheet } else { TreeIcon::Board }),
                label: "Definition",
                accessibility_label: None,
                tip: "",
                selected: self.selection.is_empty(),
            },
        );
        if root.fold {
            ui.ctx().data_mut(|data| data.insert_temp(fold_id, !open));
        }
        if root.clicked {
            let _ = self.select(None);
        }
        if !open {
            return;
        }
        for (object_id, primitive) in &item.objects {
            let label = format!("{} · {}", canvas::name(primitive), object_id);
            let row = TreeRow {
                depth: 1,
                fold: None,
                icon: Some(TreeIcon::Tool(canvas::browser_icon(primitive))),
                label: &label,
                accessibility_label: None,
                tip: "",
                selected: self.selection.contains(object_id),
            };
            if tree_row(ui, ui.make_persistent_id(("definition_primitive", object_id)), &row).clicked {
                let mut selection = if ui.input(|i| i.modifiers.shift || i.modifiers.command) { self.selection.clone() } else { Default::default() };
                if !selection.insert(*object_id) {
                    selection.remove(object_id);
                }
                let _ = self.select_many(selection);
            }
        }
    }
    /// Compose cards and canvas from shared chrome while retaining a document-local camera.
    fn show(&mut self, ui: &mut Ui, navigation: Navigation, active: bool, panel: &mut pcbcraft_chrome::workspace::panels::Panel) -> bool {
        let mut expanded = false;
        self.collapsed = authoring::workspace(ui, self.collapsed, panel, |ui, region| match region {
            authoring::Region::Browser => self.browser(ui),
            authoring::Region::Properties => {
                self.properties(ui, false);
                expanded = pcbcraft_chrome::controls::button(ui, "More properties…").clicked();
            }
            authoring::Region::Canvas { area, covered } => self.canvas(ui, area, covered, navigation, active),
        });
        expanded || std::mem::take(&mut self.details_requested)
    }
}
impl PcbApp {
    /// Create a definition document without requiring or modifying a project.
    pub(super) fn new_library_item(&mut self, kind: Kind) {
        let preferred = self.preferred_library_source();
        self.message = Item::new(kind)
            .and_then(|item| self.library_editors.open(item, false))
            .inspect(|id| self.library_editors.suggest_destination(*id, preferred))
            .and_then(|id| self.place_library_document(&Context::default(), id, pcbcraft_chrome::workspace::Placement::Here))
            .err();
    }
    /// Fork the actual focused selection under a fresh identity; highlights grant no edit authority.
    pub(super) fn fork_library_selection(&mut self, kind: Kind) -> Result<(), String> {
        if self.views.active.definition.is_some()
            || !matches!(
                (kind, self.views.active.workspace),
                (Kind::Symbol, crate::Workspace::Schematic) | (Kind::Footprint, crate::Workspace::Board)
            )
        {
            return Err("Return to the matching project workspace and select an item before editing a copy.".into());
        }
        let project = self.views.active.snapshot(&self.session).ok_or("Open a project first.")?;
        let item = match kind {
            Kind::Symbol => {
                let selected = &self.views.active.schematic.selection.symbols;
                if selected.len() != 1 {
                    return Err("Select one symbol to edit a copy.".into());
                }
                let symbol = project.symbols().iter().find(|symbol| selected.contains(&symbol.id())).ok_or("Selected symbol no longer exists.")?;
                Item::from_symbol(&symbol.definition)?
            }
            Kind::Footprint => {
                let selected = &self.views.active.board.selection;
                if selected.len() != 1 {
                    return Err("Select one footprint to edit a copy.".into());
                }
                let board = project.board_target().and_then(|target| project.board_view(target)).ok_or("Open a physical board first.")?;
                let footprint =
                    board.footprints().find(|footprint| selected.contains(&footprint.id)).ok_or("Selected footprint no longer exists.")?;
                let assignment = project
                    .footprint_assignments()
                    .into_iter()
                    .find(|a| a["component"] == footprint.component)
                    .ok_or("This placement has no pinned physical definition.")?;
                let definition = project
                    .footprint_catalog()
                    .into_iter()
                    .find(|d| {
                        assignment["library"] == d.library && assignment["definition"] == d.id && assignment["definition_revision"] == d.revision
                    })
                    .ok_or("Pinned physical definition is unavailable.")?;
                Item::from_footprint(&definition)?
            }
        };
        let id = self.library_editors.open(item, false)?;
        self.place_library_document(&Context::default(), id, pcbcraft_chrome::workspace::Placement::Here)
    }
    /// Draw authoring chrome for the focused definition; its body shares the host's mixed
    /// pane layout while project services and modal ownership remain with the main host.
    pub(super) fn library_frame(&mut self, ui: &mut Ui, integrated: bool) {
        let ctx = ui.ctx().clone();
        self.sync_library_focus();
        let main = self.views.host == self.views.main;
        let enabled = matches!(self.dialog, Dialog::None)
            && !self.files.busy()
            && !self.project_action_pending()
            && !self.library_sources.mutating()
            && !self.library_sources.installing()
            && (main || !self.home);
        let active = self.library_editors.active;
        let available = active.and_then(|id| self.library_editors.documents.get(&id));
        let can_save = available.is_some_and(|doc| !doc.pending_any()) && !self.library_editors.busy();
        let can_undo = available.is_some_and(|doc| !doc.pending() && doc.session.can_undo());
        let can_redo = available.is_some_and(|doc| !doc.pending() && doc.session.can_redo());
        let title = active.and_then(|id| {
            self.library_editors.documents.get(&id).map(|doc| (doc.session.snapshot().name.clone(), self.library_editors.document_unsaved(id)))
        });
        let tabs: Vec<_> = self
            .library_editors
            .documents
            .iter()
            .filter(|(_, doc)| Some(doc.session.snapshot().kind) == self.library_editors.active_kind())
            .map(|(id, doc)| (*id, format!("{}{}", doc.session.snapshot().name, if self.library_editors.document_unsaved(*id) { " *" } else { "" })))
            .collect();
        enum Intent {
            Home,
            Open,
            Save(bool),
            Publish,
            Copy,
            Undo,
            Redo,
            Focus(u64),
            Close,
            Quit,
        }
        let intents = std::cell::RefCell::new(Vec::new());
        let mut preferences = false;
        let mut help = None;
        #[cfg(not(target_arch = "wasm32"))]
        let mut assistant_clicked = false;
        #[cfg(not(target_arch = "wasm32"))]
        let (assistant_open, assistant_ready) = self.assistant.toolbar_state();
        #[cfg(not(target_arch = "wasm32"))]
        let assistant_visible = self.assistant.toolbar_visible();
        shell::app_bar(
            ui,
            title.as_ref().map(|(name, dirty)| (name.as_str(), *dirty)),
            integrated,
            enabled,
            |ui| {
                if menu::quick_action(ui, MenuIcon::Home, "Home", "", true).clicked() {
                    intents.borrow_mut().push(Intent::Home);
                }
            },
            |ui| {
                menu::file(ui, |ui| {
                    if pcbcraft_chrome::MenuItem::new("Open Library Item…").enabled(!self.library_editors.busy()).show(ui) {
                        intents.borrow_mut().push(Intent::Open);
                    }
                    if pcbcraft_chrome::MenuItem::new(if IN_BROWSER { "Download Library Item…" } else { "Save Library Copy…" })
                        .enabled(can_save)
                        .show_response(ui)
                        .on_hover_text("Save an unfinished draft file; this does not publish it to the catalog.")
                        .clicked()
                    {
                        intents.borrow_mut().push(Intent::Save(false));
                    }
                    if pcbcraft_chrome::MenuItem::new("Export Native Definition…").enabled(can_save).show(ui) {
                        intents.borrow_mut().push(Intent::Save(true));
                    }
                    if pcbcraft_chrome::MenuItem::new("Close Library Item").show(ui) {
                        intents.borrow_mut().push(Intent::Close);
                    }
                    if !IN_BROWSER && pcbcraft_chrome::MenuItem::new("Quit").show(ui) {
                        intents.borrow_mut().push(Intent::Quit);
                    }
                });
                menu::library(ui, |ui| {
                    if pcbcraft_chrome::MenuItem::new("Publish…").enabled(can_save && !IN_BROWSER).show(ui) {
                        intents.borrow_mut().push(Intent::Publish);
                    }
                    if pcbcraft_chrome::MenuItem::new("Edit copy").enabled(can_save).show(ui) {
                        intents.borrow_mut().push(Intent::Copy);
                    }
                    if pcbcraft_chrome::MenuItem::new("Save Library Copy…")
                        .enabled(can_save)
                        .show_response(ui)
                        .on_hover_text("Save an unfinished draft file; this does not publish it to the catalog.")
                        .clicked()
                    {
                        intents.borrow_mut().push(Intent::Save(false));
                    }
                });
                if menu::quick_action(ui, MenuIcon::Save, if IN_BROWSER { "Download Library Item" } else { "Publish Library Item" }, "", can_save)
                    .clicked()
                {
                    intents.borrow_mut().push(if IN_BROWSER { Intent::Save(false) } else { Intent::Publish });
                }
                for (icon, label, intent, available) in
                    [(MenuIcon::Undo, "Undo Library Edit", Intent::Undo, can_undo), (MenuIcon::Redo, "Redo Library Edit", Intent::Redo, can_redo)]
                {
                    if menu::quick_action(ui, icon, label, "", available).clicked() {
                        intents.borrow_mut().push(intent);
                    }
                }
            },
            |ui| {
                if tabs.len() > 1 {
                    pcbcraft_chrome::controls::Choice::from_id_salt("library_documents").selected_text("Documents").show_ui(ui, |ui| {
                        for (id, name) in &tabs {
                            if ui.selectable_label(active == Some(*id), name).clicked() {
                                intents.borrow_mut().push(Intent::Focus(*id));
                            }
                        }
                    });
                }
            },
            |ui, width| {
                if width >= crate::COMPACT_APP_BAR {
                    help = menu::help(ui, crate::help_menu).flatten();
                }
                preferences = menu::quick_action(ui, MenuIcon::Settings, "Preferences", "", true).clicked();
                #[cfg(not(target_arch = "wasm32"))]
                if assistant_visible {
                    assistant_clicked = pcbcraft_chrome::assistant::toolbar(ui, assistant_open, assistant_ready, "");
                }
            },
        );
        if preferences {
            self.open_preferences();
        } else if let Some(help) = help {
            self.help_action(&ctx, help);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if assistant_clicked {
            self.assistant.toggle(&ctx);
        }
        if enabled && !ctx.egui_wants_keyboard_input() {
            if ctx.input_mut(|input| input.consume_shortcut(&crate::COMMAND_S)) {
                intents.borrow_mut().push(if IN_BROWSER { Intent::Save(false) } else { Intent::Publish });
            }
            if ctx.input_mut(|input| input.consume_shortcut(&crate::COMMAND_Z)) {
                intents.borrow_mut().push(Intent::Undo);
            }
            if ctx.input_mut(|input| input.consume_shortcut(&crate::COMMAND_Y)) {
                intents.borrow_mut().push(Intent::Redo);
            }
            if !IN_BROWSER && ctx.input_mut(|input| input.consume_shortcut(&crate::COMMAND_Q)) {
                self.begin_project_close(&ctx);
            }
        }
        for intent in intents.into_inner() {
            let result = match intent {
                Intent::Home => {
                    let _ = self.library_editors.focus(None);
                    self.show_home();
                    Ok(())
                }
                Intent::Open => self.open_library_picker(&ctx),
                Intent::Publish => active.ok_or("No active library item.".into()).and_then(|id| self.begin_library_publish(id)),
                Intent::Copy => active.ok_or("No active library item.".into()).and_then(|id| self.copy_library_document(&ctx, id)),
                Intent::Save(export) => active.ok_or("No active library item.".into()).and_then(|id| self.library_editors.save(&ctx, id, export)),
                history @ (Intent::Undo | Intent::Redo) => active.ok_or("No active library item.".into()).and_then(|id| {
                    let revision = self.library_editors.documents.get(&id).ok_or("Unknown library document ID.")?.session.revision();
                    let action = if matches!(history, Intent::Undo) { Action::Undo } else { Action::Redo };
                    self.library_editors.edit(id, revision, action)
                }),
                Intent::Focus(id) => self.place_library_document(&ctx, id, pcbcraft_chrome::workspace::Placement::Here),
                Intent::Close => {
                    if let Some(id) = active {
                        if self.library_editors.document_unsaved(id) {
                            self.dialog = Dialog::LibraryClose(Some(id));
                        } else {
                            if let Err(error) = self.library_editors.close(id, false) {
                                self.library_editors.notice = Some(error);
                            }
                        }
                    }
                    Ok(())
                }
                Intent::Quit => {
                    self.begin_project_close(&ctx);
                    Ok(())
                }
            };
            if let Err(error) = result {
                self.library_editors.notice = Some(error);
            }
        }
        let ribbon = ui.add_enabled_ui(enabled, |ui| self.library_ribbon(ui, can_save)).inner;
        if let Some(notice) = self.message.as_ref().or(self.library_editors.notice.as_ref()) {
            ui.label(notice);
        }
        ui.add_enabled_ui(enabled, |ui| {
            if let Some((library, state)) = active.and_then(|id| self.library_publication_status(id)) {
                shell::status_bar(
                    ui,
                    |ui| {
                        ui.label(library);
                    },
                    |ui| {
                        ui.label(state);
                    },
                );
            }
            let area = ui.available_rect_before_wrap();
            self.editor_workspace(ui, area);
        });
        self.paint_ribbon_background(ribbon);
        #[cfg(not(target_arch = "wasm32"))]
        if main {
            let area = self.library_editors.active.and_then(|id| self.library_editors.documents.get(&id)).and_then(|doc| doc.camera.schematic_rect());
            self.assistant.show(&ctx, area, &self.session, enabled && !self.library_editors.busy(), self.library_editors.active.is_some());
            self.serve_chat(&ctx);
            if let Some(request) = self.assistant.take_settings_request() {
                self.dialog = match request {
                    crate::assistant::SettingsRequest::Settings => {
                        Dialog::Preferences(crate::preferences::PreferencesWindow::at_ai(self.preferences))
                    }
                    crate::assistant::SettingsRequest::Add(form) => Dialog::Agent { form, return_to: None },
                };
            }
        }
        let dialog_host = self.arrangement_dialog_host().unwrap_or(self.views.main);
        if self.views.host == dialog_host {
            self.project_dialogs(&ctx);
        } else if !matches!(self.dialog, Dialog::None) {
            ctx.send_viewport_cmd_to(crate::editor_windows::viewport(dialog_host), egui::ViewportCommand::Focus);
        }
    }
    /// Authoring uses the shared workspace picker and ribbon while keeping drawing state,
    /// file operations and contextual panels owned by the active definition document.
    fn library_ribbon(&mut self, ui: &mut Ui, can_save: bool) -> Option<pcbcraft_chrome::workspace::RibbonBackground> {
        use pcbcraft_chrome::workspace::{self, RibbonTool, ToolIcon};
        let entries = self.workspace_entries();
        let split = self.views.is_split();
        let detached = self.views.host != self.views.main;
        let layout_tools = [
            (ToolIcon::SplitView, "Split", "Split library workspace", true, split),
            (
                ToolIcon::EditorWindow,
                if detached { "Dock" } else { "Detach" },
                if detached { "Dock library pane" } else { "Detach library pane" },
                !IN_BROWSER,
                false,
            ),
            (ToolIcon::EditorWindow, "New window", "New library window", !IN_BROWSER, false),
        ]
        .map(|(icon, label, description, enabled, active)| RibbonTool {
            icon,
            label,
            description,
            shortcut: "",
            enabled,
            active,
            menu_check: (label == "Split").then_some(CheckKind::Toggle),
        });
        let mut layout_action = None;
        let id = self.library_editors.active?;
        let doc = self.library_editors.documents.get_mut(&id)?;
        let kind = doc.session.snapshot().kind;
        let title = if kind == Kind::Symbol { "Symbol Editor" } else { "Footprint Editor" };
        let mut tab = doc.ribbon_tab;
        let mut file_action = None;
        let response = workspace::ribbon(
            ui,
            title,
            &["Draw", "Library", "View"],
            &mut tab,
            workspace::Workspaces { entries: &entries, ..crate::WORKSPACES },
            &[],
            |ui, shown| match shown {
                0 => doc.tools(ui),
                1 => {
                    let tools = [
                        (ToolIcon::Sheet, "New", "New definition", true),
                        (ToolIcon::Browser, "Open", "Open library item", !self.library_editors.files.busy()),
                        (ToolIcon::Shared("save"), "Publish", "Publish item to a native library", can_save && !IN_BROWSER),
                        (ToolIcon::Shared("copy"), "Edit copy", "Edit under a new item identity", can_save),
                        (ToolIcon::Sheet, "Save copy", "Save unfinished draft; does not publish to the catalog", can_save),
                        (ToolIcon::Sheet, "Export", "Export native definition", can_save),
                    ]
                    .map(|(icon, label, description, enabled)| RibbonTool {
                        icon,
                        label,
                        description,
                        shortcut: "",
                        enabled,
                        active: false,
                        menu_check: None,
                    });
                    file_action = workspace::ribbon_group(ui, "Definition", &tools);
                }
                _ => {
                    doc.view_tools(ui);
                    workspace::ribbon_separator(ui);
                    layout_action = workspace::ribbon_group(ui, "Layout", &layout_tools);
                }
            },
        );
        doc.ribbon_tab = tab;
        if let Some(index) = layout_action {
            match index {
                0 => self.split_editors(ui.ctx(), !split),
                1 => self.toggle_editor_window(ui.ctx()),
                _ => self.open_editor_window(ui.ctx()),
            }
        }
        if let Some(pick) = response.workspace {
            self.pick_workspace(ui.ctx(), pick);
        } else if let Some(action) = file_action {
            self.library_editors.notice = match action {
                0 => {
                    self.new_library_item(kind);
                    self.message.take()
                }
                1 => self.open_library_picker(ui.ctx()).err(),
                2 => self.begin_library_publish(id).err(),
                3 => self.copy_library_document(ui.ctx(), id).err(),
                _ => self.library_editors.save(ui.ctx(), id, action == 5).err(),
            };
        }
        response.background
    }

    /// Explicit close/quit decisions protect every authoring document, including unapplied fields.
    pub(super) fn library_close_dialog(&mut self, ctx: &Context, id: Option<u64>) {
        let busy = self.library_editors.busy();
        if !busy {
            if let Some(id) = id
                && !self.library_editors.document_unsaved(id)
            {
                self.message = self.library_editors.close(id, false).err();
                return;
            }
            if id.is_none() && !self.library_editors.unsaved() {
                self.begin_project_close(ctx);
                return;
            }
        }
        let (mut discard, mut review, mut cancel) = (false, false, false);
        let dismissed = shell::dialog(ctx, "library_close", if id.is_some() { "Close library item?" } else { "Unsaved library items" }, |ui| {
            ui.label(if busy {
                "Finish the library file operation before closing."
            } else if self.library_editors.publisher.unconfirmed().is_some() {
                "A publication is unconfirmed. Discard closes its recovery state and leaves any published revision on disk."
            } else {
                "Unsaved library edits will be lost."
            });
            ui.horizontal(|ui| {
                review = pcbcraft_chrome::controls::button(ui, "Review edits").clicked();
                discard = pcbcraft_chrome::controls::enabled_button(ui, !busy, "Discard library edits").clicked();
                cancel = pcbcraft_chrome::controls::button(ui, "Cancel").clicked();
            });
        });
        if discard {
            if let Some(id) = id {
                self.message = self.library_editors.close(id, true).err();
            } else {
                self.library_editors.parts.clear();
                self.library_editors.documents.clear();
                self.library_editors.publisher = Default::default();
                self.library_editors.active = None;
                self.begin_project_close(ctx);
            }
        } else if review {
            self.review_library_edits(ctx, id);
        } else if !(cancel || dismissed) {
            self.dialog = Dialog::LibraryClose(id);
        }
    }
    /// Expanded and compact properties operate on the same revision-bound field draft.
    pub(super) fn library_properties_dialog(&mut self, ctx: &Context, id: u64, view: crate::ViewId) {
        if self.bind_library_view(view) != Some(id) {
            return;
        }
        let Some(doc) = self.library_editors.documents.get_mut(&id) else { return };
        let mut close = false;
        let dismissed = shell::scrolling_dialog_width(ctx, &format!("library_details_{id}"), "Library Properties", 760.0, |ui| {
            doc.properties(ui, true);
            close = pcbcraft_chrome::controls::button(ui, "Close").clicked();
        });
        if !(close || dismissed) {
            self.dialog = Dialog::LibraryProperties(id, view);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PcbApp {
    /// Live commands validate parameters through window discovery, then use the exact document
    /// handlers called by the canvas, forms and File actions. Project revision is not authority.
    pub(super) fn library_command(&mut self, ctx: &Context, id: &str, params: &Value) -> Result<(), String> {
        let kind = || if params["kind"] == "symbol" { Kind::Symbol } else { Kind::Footprint };
        let document = || params["document"].as_u64().ok_or("A library document ID is required.");
        match id {
            "app.library_new" => {
                let preferred = self.preferred_library_source();
                let item = Item::new(kind())?;
                let document = self.library_editors.open(item, false)?;
                self.library_editors.suggest_destination(document, preferred);
                self.place_library_document(ctx, document, pcbcraft_chrome::workspace::Placement::Here)?;
            }
            "app.library_fork_selected" => {
                let view = crate::ViewId(params["view"].as_u64().ok_or("A source view ID is required.")?);
                let host = self.views.owner(view).ok_or("Unknown source view ID.")?;
                if !matches!(self.dialog, Dialog::None) {
                    return Err("Finish the current dialog before forking a definition.".into());
                }
                let previous = self.views.host;
                self.bind_editor_host(host);
                self.activate_editor(ctx, view);
                let result = self.fork_library_selection(kind());
                self.bind_editor_host(previous);
                result?;
            }
            "app.library_edit" => {
                let revision = params["expected_revision"].as_u64().ok_or("A library revision is required.")?;
                let action = serde_json::from_value(params["action"].clone()).map_err(|e| e.to_string())?;
                self.library_editors.edit(document()?, revision, action)?;
            }
            "app.library_focus" => {
                if let Some(id) = params["document"].as_u64() {
                    self.place_library_document(ctx, id, pcbcraft_chrome::workspace::Placement::Here)?;
                } else {
                    self.views.active.definition = None;
                    self.views.active.release_input();
                    self.views.active.sync(&self.session);
                    self.sync_library_focus();
                }
            }
            "app.library_view" => {
                if let Some(view) = params["view"].as_u64()
                    && self.views.view(crate::ViewId(view)).and_then(|view| view.definition) != Some(document()?)
                {
                    return Err("This view does not show the addressed definition.".into());
                }
                if let Some(view) = params["view"].as_u64() {
                    self.bind_library_view(crate::ViewId(view));
                }
                let doc = self.library_editors.documents.get_mut(&document()?).ok_or("Unknown library document ID.")?;
                if (params["definition"] == true && (params.get("selected").is_some() || params.get("selected_objects").is_some()))
                    || (params.get("selected").is_some() && params.get("selected_objects").is_some())
                {
                    return Err("Choose a primitive or definition metadata, not both.".into());
                }
                if params["fit"] == true && (params.get("zoom_percent").is_some() || params.get("center").is_some()) {
                    return Err("Fit cannot be combined with explicit camera coordinates.".into());
                }
                let grid = params["grid_spacing_nm"].as_i64();
                if grid.is_some_and(|grid| {
                    if doc.session.snapshot().kind == Kind::Symbol {
                        grid != 1_270_000
                    } else {
                        ![25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000].contains(&grid)
                    }
                }) {
                    return Err("Choose a grid spacing offered by this library workspace.".into());
                }
                if let Some(ids) = params["selected_objects"].as_array() {
                    doc.select_many(ids.iter().filter_map(Value::as_u64).collect())?;
                } else if params["definition"] == true {
                    doc.select(None)?;
                } else if let Some(id) = params["selected"].as_u64() {
                    doc.select(Some(id))?;
                }
                if let Some(grid) = grid {
                    doc.camera.set_snap_nm(grid);
                }
                if let Some(zoom) = params["zoom_percent"].as_i64() {
                    doc.camera.set_zoom(zoom as f32 / 100.0);
                }
                if let Some(center) = params.get("center") {
                    let center = serde_json::from_value(center.clone()).map_err(|e| e.to_string())?;
                    doc.camera.request_center(center);
                }
                if params["fit"] == true {
                    doc.camera.request_fit();
                }
                if let Some(tab) = params["ribbon_tab"].as_str() {
                    doc.ribbon_tab = match tab {
                        "draw" => 0,
                        "library" => 1,
                        _ => 2,
                    };
                }
                for (index, field) in ["browser", "properties"].iter().enumerate() {
                    if let Some(visible) = params[*field].as_bool() {
                        doc.collapsed[index] = !visible;
                    }
                }
            }
            "app.library_close" => self.library_editors.close(document()?, params["discard"] == true)?,
            "app.library_open" => self.open_library_picker(ctx)?,
            "app.library_save" => self.library_editors.save(ctx, document()?, params["export"] == true)?,
            "app.library_publish" => {
                let revision = params["expected_revision"].as_u64().ok_or("A library document revision is required.")?;
                self.publish_library_item_as(
                    ctx,
                    document()?,
                    revision,
                    params["source"].as_str().ok_or("A destination source is required.")?,
                    params["allow_older"] == true,
                    params["copy"] == true,
                )?;
            }
            "app.library_copy" => self.copy_library_document(ctx, document()?)?,
            _ => return Err("Unknown library command.".into()),
        }
        self.sync_library_bindings();
        ctx.request_repaint();
        Ok(())
    }
}
