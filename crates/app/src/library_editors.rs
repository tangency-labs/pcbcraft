//! Library definitions are document workspaces, separate from project instances and history.
mod canvas;
mod form;
mod io;
#[cfg(test)]
mod tests;
use crate::{Dialog, IN_BROWSER, Navigation, PcbApp};
use egui::{Context, Ui};
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
    pub(super) active: Option<u64>,
    next_id: u64,
    files: io::Files,
    notice: Option<String>,
    pub(super) focus_requested: bool,
}
#[derive(Debug)]
struct Document {
    session: Session,
    selected: Option<u64>,
    draft: Option<form::Draft>,
    camera: crate::canvas_camera::Camera,
    collapsed: [bool; 2],
    tool: canvas::Tool,
    error: Option<String>,
}
impl Editors {
    /// Return to retained documents without changing their selection or pending property input.
    pub(super) fn resume(&mut self) {
        self.active = self.documents.keys().next_back().copied();
        self.focus_requested = self.active.is_some();
    }
    pub(super) fn has_documents(&self) -> bool {
        !self.documents.is_empty()
    }
    /// The browser picker starts during the triggering interaction; native pickers use a worker.
    pub(super) fn open_picker(&mut self, ctx: &Context) -> Result<(), String> {
        self.files.start(ctx, None)
    }
    /// Include unapplied field input in leave/close protection, independently of the project.
    pub(super) fn unsaved(&self) -> bool {
        self.documents.values().any(Document::unsaved)
    }
    /// Only one bounded picker/publication owns resources at a time.
    pub(super) fn busy(&self) -> bool {
        self.files.busy()
    }
    /// Inspection identifies each document and its own revision, never the project revision.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inspect(&self) -> Value {
        json!({"active":self.active,"file_operation":self.busy(),"notice":self.notice,"documents":self.documents.iter().map(|(id,doc)|json!({"document":id,"state":doc.session.inspect(),"selected":doc.selected,"pending_fields":doc.pending(),"view":{"zoom_percent":(doc.camera.zoom()*100.0).round(),"center":doc.camera.center(),"grid_spacing_nm":doc.camera.snap_nm()}})).collect::<Vec<_>>()})
    }
    /// Admit a validated item with fresh runtime identity; duplicate portable IDs never replace
    /// unsaved open documents. Opening files and forks use this same ownership boundary.
    fn open(&mut self, item: Item, saved: bool) -> Result<u64, String> {
        if self.documents.len() >= 4 {
            return Err("Close a library document before opening another (maximum four).".into());
        }
        if self.documents.values().any(|doc| {
            let old = doc.session.snapshot();
            old.library == item.library && old.id == item.id
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
                session: Session::new(item, saved)?,
                selected: None,
                draft: None,
                camera,
                collapsed: [false, false],
                tool: canvas::Tool::Select,
                error: None,
            },
        );
        self.next_id = id;
        self.active = Some(id);
        self.focus_requested = true;
        Ok(id)
    }
    /// GUI and MCP edits share this document/revision check and engine transaction path.
    fn edit(&mut self, id: u64, revision: u64, action: Action) -> Result<(), String> {
        self.documents.get_mut(&id).ok_or("Unknown library document ID.")?.execute(revision, action)
    }
    /// Close only an explicitly addressed document. Discard is a conscious user/command choice;
    /// in-flight file owners cannot be destroyed or silently rebound to another document.
    fn close(&mut self, id: u64, discard: bool) -> Result<(), String> {
        if self.busy() {
            return Err("Finish the library file operation before closing a document.".into());
        }
        let doc = self.documents.get(&id).ok_or("Unknown library document ID.")?;
        if doc.unsaved() && !discard {
            return Err("Library item has unsaved changes.".into());
        }
        self.documents.remove(&id);
        if self.active == Some(id) {
            self.active = self.documents.keys().next_back().copied();
        }
        Ok(())
    }
    /// File completion affects its captured document and snapshot only. Newer edits remain dirty.
    pub(super) fn poll(&mut self) {
        let Some(result) = self.files.poll() else { return };
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
        if doc.pending() {
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
        self.session.dirty() || self.pending()
    }
    /// Selection cannot silently discard unfinished edits; each document retains its own draft.
    fn select(&mut self, id: Option<u64>) -> Result<(), String> {
        if id.is_some_and(|id| !self.session.snapshot().objects.contains_key(&id)) {
            return Err("Unknown library primitive ID.".into());
        }
        if self.selected != id {
            if self.pending() {
                let error = "Apply or reset the property fields before changing selection.".to_owned();
                self.error = Some(error.clone());
                return Err(error);
            }
            self.selected = id;
            self.draft = None;
            self.tool = canvas::Tool::Select;
            self.error = None;
        }
        Ok(())
    }
    /// Validate a shared transaction and invalidate cached forms/tools only after success.
    fn execute(&mut self, revision: u64, action: Action) -> Result<(), String> {
        if self.pending() {
            return Err("Apply or reset the property fields before editing the document.".into());
        }
        self.session.execute(revision, action)?;
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
        if self.draft.as_ref().is_some_and(|draft| draft.revision != self.session.revision()) {
            self.draft = None;
            self.error = Some("The library document changed; property fields were refreshed.".into());
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
            apply = ui.add_enabled(draft.dirty(), egui::Button::new("Apply")).clicked();
            reset = ui.button("Reset").clicked();
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
    /// Browser object rows use stable IDs; electrical numbers are editable labels only.
    fn browser(&mut self, ui: &mut Ui) {
        let item = self.session.snapshot();
        if ui.selectable_label(self.selected.is_none(), "Definition").clicked() {
            let _ = self.select(None);
        }
        properties::section(ui, if item.kind == Kind::Symbol { "Symbol primitives" } else { "Footprint primitives" });
        for (id, primitive) in &item.objects {
            let label = format!("{} · {}", canvas::name(primitive), id);
            if ui.selectable_label(self.selected == Some(*id), label).clicked() {
                let _ = self.select(Some(*id));
            }
        }
    }
    /// Compose cards and canvas from shared chrome while retaining a document-local camera.
    fn show(&mut self, ui: &mut Ui, navigation: Navigation) -> bool {
        let mut expanded = false;
        self.collapsed = authoring::workspace(ui, self.collapsed, |ui, region| match region {
            authoring::Region::Tools => self.tools(ui),
            authoring::Region::Browser => self.browser(ui),
            authoring::Region::Properties => {
                self.properties(ui, false);
                expanded = ui.button("More properties…").clicked();
            }
            authoring::Region::Canvas { area, covered } => self.canvas(ui, area, covered, navigation),
        });
        expanded
    }
}
impl PcbApp {
    /// Create a definition document without requiring or modifying a project.
    pub(super) fn new_library_item(&mut self, kind: Kind) {
        self.message = Item::new(kind).and_then(|item| self.library_editors.open(item, false)).err();
    }
    /// Fork the actual focused selection under a fresh identity; highlights grant no edit authority.
    pub(super) fn fork_library_selection(&mut self, kind: Kind) -> Result<(), String> {
        if self.library_editors.active.is_some()
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
                let board = project.board_view(project.board_target()).ok_or("Open a physical board first.")?;
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
        self.library_editors.open(item, false)?;
        Ok(())
    }
    /// Dedicated in-window definition workspaces leave project views and services intact.
    pub(super) fn library_frame(&mut self, ui: &mut Ui, integrated: bool) {
        let ctx = ui.ctx().clone();
        let hidden = self.views.hosts.get(&self.views.main).map(|host| host.layout.views()).unwrap_or_default();
        for view in self.views.all_mut().filter(|view| hidden.contains(&view.id())) {
            view.cancel_gesture();
        }
        let enabled = matches!(self.dialog, Dialog::None) && !self.files.busy() && !self.project_action_pending();
        let active = self.library_editors.active;
        let available = active.and_then(|id| self.library_editors.documents.get(&id));
        let can_save = available.is_some_and(|doc| !doc.pending()) && !self.library_editors.busy();
        let can_undo = available.is_some_and(|doc| !doc.pending() && doc.session.can_undo());
        let can_redo = available.is_some_and(|doc| !doc.pending() && doc.session.can_redo());
        let title = active.and_then(|id| self.library_editors.documents.get(&id)).map(|doc| (doc.session.snapshot().name.clone(), doc.unsaved()));
        let tabs: Vec<_> = self
            .library_editors
            .documents
            .iter()
            .map(|(id, doc)| (*id, format!("{}{}", doc.session.snapshot().name, if doc.unsaved() { " *" } else { "" })))
            .collect();
        enum Intent {
            Project,
            New(Kind),
            Open,
            Save(bool),
            Edit(Action),
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
                if menu::quick_action(ui, MenuIcon::Home, "Project workspace", "", true).clicked() {
                    intents.borrow_mut().push(Intent::Project);
                }
            },
            |ui| {
                menu::file(ui, |ui| {
                    for (label, kind) in [("New Symbol", Kind::Symbol), ("New Footprint", Kind::Footprint)] {
                        if pcbcraft_chrome::MenuItem::new(label).show(ui) {
                            intents.borrow_mut().push(Intent::New(kind));
                        }
                    }
                    if pcbcraft_chrome::MenuItem::new("Open Library Item…").show(ui) {
                        intents.borrow_mut().push(Intent::Open);
                    }
                    if pcbcraft_chrome::MenuItem::new("Export Native Definition…").show(ui) {
                        intents.borrow_mut().push(Intent::Save(true));
                    }
                    if pcbcraft_chrome::MenuItem::new("Close Library Item").show(ui) {
                        intents.borrow_mut().push(Intent::Close);
                    }
                    if !IN_BROWSER && pcbcraft_chrome::MenuItem::new("Quit").show(ui) {
                        intents.borrow_mut().push(Intent::Quit);
                    }
                });
                if menu::quick_action(ui, MenuIcon::Save, if IN_BROWSER { "Download Library Item" } else { "Save Library Copy" }, "", can_save)
                    .clicked()
                {
                    intents.borrow_mut().push(Intent::Save(false));
                }
                for (icon, label, action, available) in
                    [(MenuIcon::Undo, "Undo Library Edit", Action::Undo, can_undo), (MenuIcon::Redo, "Redo Library Edit", Action::Redo, can_redo)]
                {
                    if menu::quick_action(ui, icon, label, "", available).clicked() {
                        intents.borrow_mut().push(Intent::Edit(action));
                    }
                }
            },
            |ui| {
                if tabs.len() > 1 {
                    egui::ComboBox::from_id_salt("library_documents").selected_text("Documents").show_ui(ui, |ui| {
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
                intents.borrow_mut().push(Intent::Save(false));
            }
            if ctx.input_mut(|input| input.consume_shortcut(&crate::COMMAND_Z)) {
                intents.borrow_mut().push(Intent::Edit(Action::Undo));
            }
            if ctx.input_mut(|input| input.consume_shortcut(&crate::COMMAND_Y)) {
                intents.borrow_mut().push(Intent::Edit(Action::Redo));
            }
            if !IN_BROWSER && ctx.input_mut(|input| input.consume_shortcut(&crate::COMMAND_Q)) {
                self.begin_project_close(&ctx);
            }
        }
        for intent in intents.into_inner() {
            let result = match intent {
                Intent::Project => {
                    self.library_editors.active = None;
                    Ok(())
                }
                Intent::New(kind) => Item::new(kind).and_then(|item| self.library_editors.open(item, false)).map(|_| ()),
                Intent::Open => self.library_editors.files.start(&ctx, None),
                Intent::Save(export) => active.ok_or("No active library item.".into()).and_then(|id| self.library_editors.save(&ctx, id, export)),
                Intent::Edit(action) => active.ok_or("No active library item.".into()).and_then(|id| {
                    let revision = self.library_editors.documents.get(&id).ok_or("Unknown library document ID.")?.session.revision();
                    self.library_editors.edit(id, revision, action)
                }),
                Intent::Focus(id) => {
                    self.library_editors.active = Some(id);
                    Ok(())
                }
                Intent::Close => {
                    if let Some(id) = active {
                        if self.library_editors.documents.get(&id).is_some_and(Document::unsaved) {
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
        if let Some(notice) = &self.library_editors.notice {
            ui.label(notice);
        }
        if let Some(id) = self.library_editors.active
            && let Some(doc) = self.library_editors.documents.get_mut(&id)
        {
            ui.add_enabled_ui(enabled, |ui| {
                if ui.push_id(("library_document", id), |ui| doc.show(ui, self.preferences.navigation)).inner {
                    self.dialog = Dialog::LibraryProperties(id);
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
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
        self.project_dialogs(&ctx);
        if integrated {
            shell::resize_window_edges(ui);
        }
    }
    /// Explicit close/quit decisions protect every authoring document, including unapplied fields.
    pub(super) fn library_close_dialog(&mut self, ctx: &Context, id: Option<u64>) {
        let busy = self.library_editors.busy();
        if !busy {
            if let Some(id) = id
                && self.library_editors.documents.get(&id).is_some_and(|doc| !doc.unsaved())
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
            ui.label(if busy { "Finish the library file operation before closing." } else { "Unsaved library edits will be lost." });
            ui.horizontal(|ui| {
                review = ui.button("Review edits").clicked();
                discard = ui.add_enabled(!busy, egui::Button::new("Discard library edits")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if discard {
            if let Some(id) = id {
                self.message = self.library_editors.close(id, true).err();
            } else {
                self.library_editors.documents.clear();
                self.library_editors.active = None;
                self.begin_project_close(ctx);
            }
        } else if review {
            self.library_editors.active = id.or_else(|| self.library_editors.documents.iter().find(|(_, doc)| doc.unsaved()).map(|(id, _)| *id));
        } else if !(cancel || dismissed) {
            self.dialog = Dialog::LibraryClose(id);
        }
    }
    /// Expanded and compact properties operate on the same revision-bound field draft.
    pub(super) fn library_properties_dialog(&mut self, ctx: &Context, id: u64) {
        let Some(doc) = self.library_editors.documents.get_mut(&id) else { return };
        let mut close = false;
        let dismissed = shell::scrolling_dialog_width(ctx, &format!("library_details_{id}"), "Library Properties", 760.0, |ui| {
            doc.properties(ui, true);
            close = ui.button("Close").clicked();
        });
        if !(close || dismissed) {
            self.dialog = Dialog::LibraryProperties(id);
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
                let item = Item::new(kind())?;
                self.library_editors.open(item, false)?;
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
                let id = params["document"].as_u64();
                if let Some(id) = id
                    && !self.library_editors.documents.contains_key(&id)
                {
                    return Err("Unknown library document ID.".into());
                }
                self.library_editors.active = id;
                self.library_editors.focus_requested = id.is_some();
            }
            "app.library_view" => {
                let doc = self.library_editors.documents.get_mut(&document()?).ok_or("Unknown library document ID.")?;
                if params["definition"] == true && params.get("selected").is_some() {
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
                if params["definition"] == true {
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
            }
            "app.library_close" => self.library_editors.close(document()?, params["discard"] == true)?,
            "app.library_open" => self.library_editors.open_picker(ctx)?,
            "app.library_save" => self.library_editors.save(ctx, document()?, params["export"] == true)?,
            _ => return Err("Unknown library command.".into()),
        }
        ctx.request_repaint();
        Ok(())
    }
}
