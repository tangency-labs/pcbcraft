//! Per-pane presentation over one definition session; host placement never owns authored data.
use super::*;
use crate::ViewId;
use pcbcraft_chrome::workspace::Placement;

impl Editors {
    /// Read the camera belonging to this definition view, including presentations parked while
    /// another pane of the same document renders. The backdrop must not borrow another view's grid.
    pub(crate) fn view_camera(&self, definition: u64, view: ViewId) -> Option<&crate::canvas_camera::Camera> {
        let doc = self.documents.get(&definition)?;
        if doc.bound_view == Some(view) { Some(&doc.camera) } else { doc.presentations.get(&view).map(|presentation| &presentation.camera) }
    }
}

#[derive(Debug)]
pub(super) struct Presentation {
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
impl Presentation {
    /// Duplicate navigation and panel choices while leaving selection and field/gesture drafts independent.
    fn fresh(doc: &mut Document) -> Self {
        Self {
            selected: None,
            selection: Default::default(),
            prototype: doc.prototype.clone(),
            drawing_purpose: doc.drawing_purpose,
            drawing_stroke_nm: doc.drawing_stroke_nm,
            drawing_fill: doc.drawing_fill,
            array_requested: false,
            details_requested: false,
            draft: None,
            camera: doc.camera.fork_navigation(),
            collapsed: doc.collapsed,
            tool: canvas::Tool::Select,
            ribbon_tab: doc.ribbon_tab,
            error: None,
        }
    }
    /// Only presentation exchanges render slots; the definition's session/history never moves.
    fn swap(&mut self, doc: &mut Document) {
        std::mem::swap(&mut self.selected, &mut doc.selected);
        std::mem::swap(&mut self.selection, &mut doc.selection);
        std::mem::swap(&mut self.prototype, &mut doc.prototype);
        std::mem::swap(&mut self.drawing_purpose, &mut doc.drawing_purpose);
        std::mem::swap(&mut self.drawing_stroke_nm, &mut doc.drawing_stroke_nm);
        std::mem::swap(&mut self.drawing_fill, &mut doc.drawing_fill);
        std::mem::swap(&mut self.array_requested, &mut doc.array_requested);
        std::mem::swap(&mut self.details_requested, &mut doc.details_requested);
        std::mem::swap(&mut self.draft, &mut doc.draft);
        std::mem::swap(&mut self.camera, &mut doc.camera);
        std::mem::swap(&mut self.collapsed, &mut doc.collapsed);
        std::mem::swap(&mut self.tool, &mut doc.tool);
        std::mem::swap(&mut self.ribbon_tab, &mut doc.ribbon_tab);
        std::mem::swap(&mut self.error, &mut doc.error);
    }
    /// Retained field input participates in document close and save protection.
    fn pending(&self) -> bool {
        self.draft.as_ref().is_some_and(form::Draft::dirty)
    }
}
impl Document {
    /// Bind a live pane's retained presentation. New simultaneous views copy navigation only;
    /// a reopened document with no surviving pane resumes its last fields and selection.
    fn bind_view(&mut self, view: ViewId, simultaneous: bool) {
        if self.bound_view == Some(view) {
            return;
        }
        let state = self.presentations.remove(&view).or_else(|| (self.bound_view.is_some() && simultaneous).then(|| Presentation::fresh(self)));
        if let Some(mut state) = state {
            state.swap(self);
            if let Some(old) = self.bound_view {
                self.presentations.insert(old, state);
            }
        }
        self.bound_view = Some(view);
    }
    /// Hidden or closed panes retain unapplied fields until a deliberate document close/discard.
    pub(crate) fn pending_any(&self) -> bool {
        self.pending() || self.presentations.values().any(Presentation::pending)
    }
}
impl PcbApp {
    /// Review selects a live pane with pending fields, or recovers a closed pane's retained
    /// fields into the current host. It never silently discards a second view's draft.
    pub(crate) fn review_library_edits(&mut self, ctx: &Context, requested: Option<u64>) {
        let id = requested.or_else(|| self.library_editors.documents.iter().find(|(_, doc)| doc.unsaved()).map(|(id, _)| *id));
        let Some(id) = id else { return };
        let pending =
            self.library_editors.documents.get(&id).and_then(|doc| {
                if doc.pending() { doc.bound_view } else { doc.presentations.iter().find(|(_, v)| v.pending()).map(|(view, _)| *view) }
            });
        if let Some(view) = pending
            && self.views.view(view).is_some_and(|view| view.definition == Some(id))
            && let Some(host) = self.views.owner(view)
        {
            self.home = false;
            if host == self.views.host {
                self.activate_editor(ctx, view);
            } else if let Some(layout) = self.views.hosts.get_mut(&host) {
                layout.focused = view;
                layout.layout.activate(view);
            }
            self.bind_library_view(view);
            ctx.send_viewport_cmd_to(crate::editor_windows::viewport(host), egui::ViewportCommand::Focus);
        } else {
            self.message = self.place_library_document(ctx, id, Placement::Here).err();
            if let Some(doc) = self.library_editors.documents.get_mut(&id)
                && !doc.pending()
                && let Some(old) = doc.presentations.iter().find(|(_, state)| state.pending()).map(|(view, _)| *view)
                && let Some(mut state) = doc.presentations.remove(&old)
            {
                state.swap(doc);
            }
        }
    }
    /// Fit the bound presentation through the same camera request as the ribbon.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn fit_library_document(&mut self, id: u64) {
        if let Some(doc) = self.library_editors.documents.get_mut(&id) {
            doc.camera.request_fit();
        }
    }
    /// Inspection reports the addressed pane, not the last sibling rendered in another host.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn inspect_library_view(&self, view: ViewId) -> Option<Value> {
        let id = self.views.view(view)?.definition?;
        let doc = self.library_editors.documents.get(&id)?;
        macro_rules! state {($state:expr,$pending:expr)=>{json!({"document":id,"selected":$state.selected,"selection":$state.selection,"pending_fields":$pending,
            "ribbon_tab":(["draw","library","view"][$state.ribbon_tab]),"browser":!$state.collapsed[0],"properties":!$state.collapsed[1],
            "zoom_percent":($state.camera.zoom()*100.0).round(),"center":$state.camera.center(),"grid_spacing_nm":$state.camera.snap_nm()})};}
        if doc.bound_view == Some(view) { Some(state!(doc, doc.pending())) } else { doc.presentations.get(&view).map(|v| state!(v, v.pending())) }
    }
    /// Native titles and pane headers resolve the actual document instead of its parked project target.
    pub(crate) fn view_title(&self, view: ViewId) -> String {
        if self.views.view(view).is_some_and(|view| view.library.is_some()) {
            return "Parts Library".into();
        }
        self.views
            .view(view)
            .and_then(|view| view.definition)
            .and_then(|id| self.library_editors.documents.get(&id))
            .map(|doc| {
                format!(
                    "{} › {}",
                    if doc.session.snapshot().kind == Kind::Symbol { "Symbol Editor" } else { "Footprint Editor" },
                    doc.session.snapshot().name
                )
            })
            .or_else(|| self.views.view(view).and_then(|view| view.sheet_path(&self.session)))
            .unwrap_or_default()
    }
    /// An unfocused host cancels only its own definition previews; repainting it must not
    /// cancel a two-click tool being used in another native window.
    pub(crate) fn cancel_library_host_gestures(&mut self) {
        let views = &self.views;
        let host = views.host;
        for doc in self.library_editors.documents.values_mut() {
            if doc.bound_view.is_some_and(|view| views.owner(view) == Some(host)) {
                doc.tool = canvas::Tool::Select;
            }
            for (view, state) in &mut doc.presentations {
                if views.owner(*view) == Some(host) {
                    state.tool = canvas::Tool::Select;
                }
            }
        }
    }
    /// Drop pointer previews across focus/layout transitions without discarding property fields.
    pub(crate) fn cancel_library_gestures(&mut self) {
        for doc in self.library_editors.documents.values_mut() {
            doc.tool = canvas::Tool::Select;
            for state in doc.presentations.values_mut() {
                state.tool = canvas::Tool::Select;
            }
        }
    }
    /// Capture the source of a picker without moving filesystem work onto the editor loop.
    pub(crate) fn open_library_picker(&mut self, ctx: &Context) -> Result<(), String> {
        self.library_editors.open_picker(ctx)?;
        self.library_editors.open_target = Some(self.views.active.id());
        Ok(())
    }
    /// A completed import opens in its initiating pane, or the surviving main host if that
    /// pane was closed. It never replaces another definition's session or unsaved fields.
    pub(crate) fn adopt_opened_library(&mut self, ctx: &Context) {
        if let Some(id) = self.library_editors.pending_document.take() {
            if let Some(view) = self.library_editors.open_target.take()
                && let Some(host) = self.views.owner(view)
            {
                self.bind_editor_host(host);
                self.activate_editor(ctx, view);
            }
            self.message = self.place_library_document(ctx, id, Placement::Here).err();
            self.bind_editor_host(self.views.main);
        }
    }
    /// Resume the last document in this pane rather than opening an overlay on the main host.
    pub(crate) fn resume_library_workspace(&mut self, ctx: &Context) {
        self.library_editors.resume();
        if let Some(id) = self.library_editors.active {
            self.message = self.place_library_document(ctx, id, Placement::Here).err();
        }
    }
    /// Resolve a definition's kind without borrowing its presentation or assigning focus.
    pub(crate) fn library_kind(&self, id: u64) -> Option<Kind> {
        self.library_editors.documents.get(&id).map(|doc| doc.session.snapshot().kind)
    }

    /// Resolve a pane by stable runtime identity and bind its document-local presentation.
    /// Project views and unavailable/closed IDs never acquire a definition's edit authority.
    pub(crate) fn bind_library_view(&mut self, view: ViewId) -> Option<u64> {
        let id = self.views.view(view)?.definition?;
        let simultaneous = self
            .views
            .hosts
            .values()
            .flat_map(|host| host.layout.views())
            .any(|other| other != view && self.views.view(other).is_some_and(|v| v.definition == Some(id)));
        self.library_editors.documents.get_mut(&id)?.bind_view(view, simultaneous);
        Some(id)
    }
    /// Keep the bound host's focused presentation current without changing its document or selection.
    pub(crate) fn sync_library_focus(&mut self) {
        let id = if self.home && self.views.host == self.views.main { None } else { self.bind_library_view(self.views.active.id()) };
        self.library_editors.active = id;
    }
    /// Place an existing definition through the same pane/window layout as project editors.
    /// The old project target stays parked in that pane, and neither placement nor closing a
    /// view changes the definition session, save worker or project history.
    pub(crate) fn place_library_document(&mut self, ctx: &Context, document: u64, placement: Placement) -> Result<(), String> {
        if !self.library_editors.documents.contains_key(&document) {
            return Err("Unknown library document ID.".into());
        }
        if placement == Placement::Window && IN_BROWSER {
            return Err("Native windows are unavailable in the browser.".into());
        }
        let source = self.views.active.id();
        let (view, host) = match placement {
            Placement::Here => (source, self.views.host),
            Placement::Beside => (self.views.open_beside(source).ok_or("Could not create a library pane.")?, self.views.host),
            Placement::Window => {
                let host = self.views.open_window(source).ok_or("Could not create a library window.")?;
                let view = self.views.hosts.get(&host).ok_or("Library window is unavailable.")?.focused;
                (view, host)
            }
        };
        if let Some(target) = self.views.view_mut(view) {
            target.release_input();
            target.definition = Some(document);
            target.library = None;
        }
        self.home = false;
        if host == self.views.host {
            self.activate_editor(ctx, view);
        }
        self.bind_library_view(view);
        self.library_editors.focus(Some(document))?;
        self.sync_library_focus();
        self.library_editors.pending_document = None;
        ctx.send_viewport_cmd_to(crate::editor_windows::viewport(host), egui::ViewportCommand::Focus);
        self.repaint_editors(ctx);
        Ok(())
    }
    /// Render any definition pane in its scoped rectangle, including mixed project/library splits.
    pub(crate) fn library_pane(&mut self, ui: &mut Ui, view: ViewId) {
        let Some(id) = self.bind_library_view(view) else { return };
        if self.views.active.id() == view && matches!(self.dialog, Dialog::None) && !egui::Popup::is_any_open(ui.ctx()) {
            self.take_panel_escape(ui.ctx());
        }
        let Some(doc) = self.library_editors.documents.get_mut(&id) else { return };
        let active = self.views.active.id() == view;
        let Some(panel) = self.views.view_mut(view).map(|view| &mut view.panels[crate::workspace_panels::PROPERTIES]) else { return };
        let expanded = ui.push_id(("library_document", view, id), |ui| doc.show(ui, self.preferences.navigation, active, panel)).inner;
        let array = std::mem::take(&mut doc.array_requested).then(|| editing::ArrayDraft::new(id, view, doc));
        if let Some(array) = array {
            self.dialog = Dialog::LibraryArray(Box::new(array));
        }
        if expanded {
            self.dialog = Dialog::LibraryProperties(id, view);
            ui.ctx().send_viewport_cmd_to(crate::editor_windows::viewport(self.views.main), egui::ViewportCommand::Focus);
        }
    }
    /// A closed definition removes only its runtime bindings. Hidden draft state remains with
    /// every still-open definition, including after its last pane/window was closed.
    pub(crate) fn sync_library_bindings(&mut self) {
        for view in self.views.all_mut() {
            if view.definition.is_some_and(|id| !self.library_editors.documents.contains_key(&id)) {
                view.definition = None;
                view.release_input();
                view.sync(&self.session);
            }
        }
        let panes: Vec<_> = self.views.hosts.values().flat_map(|host| host.layout.views()).collect();
        for view in panes {
            self.bind_library_view(view);
        }
        for doc in self.library_editors.documents.values_mut() {
            doc.presentations.retain(|view, state| self.views.view(*view).is_some() || state.pending());
        }
        self.sync_library_focus();
    }
}
