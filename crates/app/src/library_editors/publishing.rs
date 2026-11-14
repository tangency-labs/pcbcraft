//! Desktop source publication is a bounded host operation; definition preparation stays in engine.
use super::*;
use pcbcraft_chrome::controls;
use pcbcraft_engine::{authoring::publishing as engine, library::sources};
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{
    Arc,
    mpsc::{self, Receiver, TryRecvError},
};

/// Short destination form retains its captured document authority while open.
#[derive(Clone, Debug)]
pub(crate) struct Draft {
    document: u64,
    revision: u64,
    source: String,
    allow_older: bool,
    copy: bool,
    error: Option<String>,
}

/// Native library creation asks for portable metadata before invoking the host folder picker.
#[derive(Debug)]
pub(crate) struct NewLibrary {
    id: String,
    name: String,
    license: String,
    error: Option<String>,
    return_to: Option<Box<Draft>>,
}
impl Default for NewLibrary {
    /// Original libraries start with the application's native authoring license, editable by the user.
    fn default() -> Self {
        Self { id: "my-library".into(), name: "My library".into(), license: "MIT OR Apache-2.0".into(), error: None, return_to: None }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
struct Request {
    document: u64,
    source: String,
    path: String,
    generation: u64,
    prepared: engine::Prepared,
}
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
enum Completion {
    NotWritten(String),
    Published(Request, Result<Arc<sources::NativeSource>, String>),
    Unconfirmed(Request, String),
    Created { source: String, path: String, parsed: Arc<sources::NativeSource>, generation: u64, return_to: Option<Box<Draft>> },
    Cancelled(Option<Box<Draft>>),
}

/// Progress captures retry authority before any write; a final result releases the worker slot.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
enum Event {
    Prepared(Request),
    Finished(Result<Completion, String>),
}

/// One host worker owns publication resources; dropping the app never joins blocked folder I/O.
#[derive(Debug, Default)]
pub(crate) struct Publisher {
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<Receiver<Event>>,
    #[cfg(not(target_arch = "wasm32"))]
    recovery: Option<Request>,
    #[cfg(not(target_arch = "wasm32"))]
    resume: Option<Box<Draft>>,
}
impl Publisher {
    /// Live inspection exposes retry ownership and portable addresses, never host filesystem paths.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inspect(&self) -> Value {
        json!({"busy":self.busy(),"unconfirmed":self.recovery.as_ref().map(|request| json!({
            "document":request.document,"source":request.source,"address":{"library":request.prepared.item().library,"id":request.prepared.item().id,"revision":request.prepared.item().revision}
        }))})
    }
    /// Uncertain visible records keep their destination protected until verified or discarded.
    pub(crate) fn unconfirmed_in(&self, library: &str) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.recovery.as_ref().is_some_and(|request| request.prepared.item().library == library)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = library;
            false
        }
    }
    /// The unresolved publication belongs to one explicit document, even after a worker disconnect.
    pub(super) fn unconfirmed(&self) -> Option<u64> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.recovery.as_ref().map(|request| request.document)
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }
    /// Explicit discard closes its retry state without removing any published native record.
    pub(super) fn discard(&mut self, document: u64) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.unconfirmed() == Some(document) {
            self.recovery = None;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = document;
    }
    /// Publication and creation share one slot with ordinary library file operations.
    pub(crate) fn busy(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pending.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// A channel disconnect releases the slot, while the captured publication is retained for
    /// verification rather than risking a second revision after an uncertain worker failure.
    #[cfg(not(target_arch = "wasm32"))]
    fn start(
        &mut self,
        ctx: &Context,
        work: impl FnOnce(&mpsc::SyncSender<Event>) -> Result<Completion, String> + Send + 'static,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("A library publication is already running.".into());
        }
        let (send, receive) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("pcbcraft-library-publish".into())
            .spawn(move || {
                let result = work(&send);
                let _ = send.send(Event::Finished(result));
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        self.pending = Some(receive);
        Ok(())
    }
}

impl PcbApp {
    /// Publish-capable host registrations are discoverable separately from engine-only source caches.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn inspect_library_destinations(&self) -> Value {
        let loaded = self.session.libraries().inspect();
        json!(
            self.writable_libraries()
                .into_iter()
                .map(|(source, name)| {
                    let library =
                        loaded.as_array().into_iter().flatten().find(|entry| entry["source"] == source).map(|entry| entry["library"].clone());
                    json!({"source":source,"library":library,"name":name,"kind":"native_folder","writable":true})
                })
                .collect::<Vec<_>>()
        )
    }
    /// Only available, registered native folders with nonreserved identities advertise publication.
    pub(crate) fn writable_libraries(&self) -> Vec<(String, String)> {
        let loaded = self.session.libraries().inspect();
        self.library_sources
            .registered
            .iter()
            .filter_map(|registration| {
                let source = loaded
                    .as_array()?
                    .iter()
                    .find(|source| source["source"] == registration.source && source["available"] == true && source["builtin"] == false)?;
                let library = source["library"].as_str()?;
                (registration.writable && !registration.path.is_empty() && !engine::reserved_destination(library))
                    .then(|| (registration.source.clone(), source["name"].as_str().unwrap_or(library).to_owned()))
            })
            .collect()
    }

    /// New items inherit the explicitly selected writable collection; this is a suggestion,
    /// not a published identity, and remains editable in the Publish dialog.
    pub(crate) fn preferred_library_source(&self) -> Option<String> {
        let source = self.views.active.library.as_ref().and_then(|state| state.source_id()).or_else(|| {
            self.library_editors.active.and_then(|id| self.library_editors.documents.get(&id)).and_then(|doc| doc.publish_source.as_deref())
        });
        source.filter(|source| self.writable_libraries().iter().any(|(id, _)| id == *source)).map(str::to_owned)
    }

    /// Open one destination picker for ordinary publication and explicit Save As copies.
    pub(crate) fn begin_library_publish(&mut self, document: u64) -> Result<(), String> {
        if IN_BROWSER {
            return Err("Publishing is desktop-only; download a draft or export a native definition instead.".into());
        }
        let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
        if doc.pending_any() {
            return Err("Apply or reset property fields before publishing.".into());
        }
        if self.library_editors.busy() || self.library_sources.busy() || self.library_mutation_busy() {
            return Err("Finish the current library operation first.".into());
        }
        let destinations = self.writable_libraries();
        let loaded = self.session.libraries().inspect();
        let item = doc.session.snapshot();
        let source = doc
            .publish_source
            .as_ref()
            .filter(|source| destinations.iter().any(|(id, _)| id == *source))
            .cloned()
            .or_else(|| {
                destinations
                    .iter()
                    .find(|(id, _)| loaded.as_array().into_iter().flatten().any(|entry| entry["source"] == *id && entry["library"] == item.library))
                    .map(|(id, _)| id.clone())
            })
            .unwrap_or_default();
        self.dialog =
            Dialog::LibraryPublish(Draft { document, revision: doc.session.revision(), source, allow_older: false, copy: false, error: None });
        Ok(())
    }

    /// Keep routine publication compact; source addresses, paths and licenses are available in
    /// Details, while an older-base decision has its own clearly spaced warning section.
    pub(crate) fn library_publish_dialog(&mut self, ctx: &Context, mut draft: Draft) {
        let destinations = self.writable_libraries();
        if !destinations.iter().any(|(source, _)| *source == draft.source) {
            draft.source.clear();
        }
        let loaded = self.session.libraries().inspect();
        let library =
            loaded.as_array().into_iter().flatten().find(|source| source["source"] == draft.source).and_then(|source| source["library"].as_str());
        let doc = self.library_editors.documents.get(&draft.document);
        let item = doc.map(|doc| doc.session.snapshot());
        let existing = item.as_ref().is_some_and(|item| item.library != "user");
        let different = item.as_ref().is_some_and(|item| existing && library.is_some_and(|library| item.library != library));
        let copy = draft.copy || different;
        let latest = item
            .as_ref()
            .map(|item| self.session.libraries().highest_revision(library.unwrap_or(&item.library), source_kind(item.kind), &item.id))
            .unwrap_or(0);
        let base = doc.and_then(|doc| doc.session.published()).map(|base| base.revision);
        let needs_confirmation = !copy && latest > 0 && base.is_none_or(|base| latest > base);
        let changed = doc.is_some_and(|doc| doc.session.publication_dirty());
        let verifying = self.library_editors.publisher.unconfirmed() == Some(draft.document);
        let title = format!("Publish {}", item.as_ref().map(|item| item.name.as_str()).unwrap_or("library item"));
        let mut create = false;
        let action = pcbcraft_chrome::catalog::property_form_enabled(
            ctx,
            "library_publish",
            &title,
            452.,
            if verifying {
                "Verify publication"
            } else if copy {
                "Publish copy"
            } else {
                "Publish"
            },
            verifying || (!draft.source.is_empty() && (copy || changed) && (!needs_confirmation || draft.allow_older)),
            |ui| {
                let choices: Vec<_> = destinations.iter().map(|(id, name)| (id.as_str(), name.clone())).collect();
                let before = draft.source.clone();
                create = properties::choice_with_action(
                    ui,
                    "Library",
                    egui::Id::unique("publish_destination"),
                    &mut draft.source,
                    &choices,
                    "New library…",
                );
                if before != draft.source {
                    draft.allow_older = false;
                    draft.error = None;
                    ctx.request_repaint();
                }
                if existing {
                    if different {
                        properties::field(ui, "Save as", |ui| {
                            ui.label("New item").on_hover_text("Creates a new item and continues editing that copy. The original remains unchanged.");
                        });
                    } else {
                        let mut mode = if draft.copy { "copy" } else { "revision" }.to_owned();
                        properties::choice(
                            ui,
                            "Save as",
                            egui::Id::unique("publication_mode"),
                            &mut mode,
                            &[("revision", "New revision".into()), ("copy", "New item".into())],
                        )
                        .on_hover_text("New item creates an independent copy. New revision continues the existing item.");
                        if draft.copy != (mode == "copy") {
                            draft.copy = mode == "copy";
                            ctx.request_repaint();
                        }
                    }
                }
                if needs_confirmation {
                    properties::section(ui, "Newer revision available");
                    let warning = base
                        .map(|base| format!("This draft is based on revision {base} and may omit changes from revision {latest}."))
                        .unwrap_or_else(|| "This draft does not record its starting revision and may omit newer changes.".into());
                    ui.label(warning);
                    if controls::checkbox(ui, &mut draft.allow_older, "Publish without merging").changed() {
                        ctx.request_repaint();
                    }
                    ui.add_space(pcbcraft_chrome::Tokens::get().space_8);
                }
                controls::collapsing(ui, ("publication_details", draft.document), "Details", false, None, |ui| {
                    if let Some(item) = &item {
                        for (label, value) in [
                            ("Type", if item.kind == Kind::Symbol { "Symbol".into() } else { "Footprint".into() }),
                            (if copy { "Source ID" } else { "Item ID" }, item.id.clone()),
                            ("Based on", base.map(|revision| format!("Revision {revision}")).unwrap_or_else(|| "Not recorded".into())),
                            ("Library ID", library.unwrap_or("Not selected").into()),
                            ("License", item.license.clone()),
                        ] {
                            properties::field(ui, label, |ui| {
                                ui.add(egui::Label::new(&value).truncate()).on_hover_text(&value);
                            });
                        }
                    }
                    if let Some(registration) = self.library_sources.registered.iter().find(|entry| entry.source == draft.source) {
                        properties::field(ui, "Folder", |ui| {
                            ui.add(egui::Label::new(&registration.path).truncate()).on_hover_text(&registration.path);
                        });
                    }
                });
                if let Some(error) = &draft.error {
                    ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
                }
            },
        );
        if create {
            let license = item.as_ref().map(|item| item.license.clone()).unwrap_or_else(|| "MIT OR Apache-2.0".into());
            self.dialog = Dialog::NewLibrary(NewLibrary { license, return_to: Some(Box::new(draft)), ..Default::default() });
        } else if action.primary {
            let result = if draft.copy {
                self.publish_library_item_as(ctx, draft.document, draft.revision, &draft.source, draft.allow_older, true)
            } else {
                self.publish_library_item(ctx, draft.document, draft.revision, &draft.source, draft.allow_older)
            };
            match result {
                Ok(()) => {}
                Err(error) => {
                    draft.error = Some(error);
                    self.dialog = Dialog::LibraryPublish(draft);
                }
            }
        } else if !action.close {
            self.dialog = Dialog::LibraryPublish(draft);
        }
    }

    /// A native library form is metadata, followed by a host-owned unused-folder selection.
    pub(crate) fn new_library_dialog(&mut self, ctx: &Context, mut draft: NewLibrary) {
        let mut valid = engine::new_library(&draft.id, &draft.name, &draft.license);
        if valid.is_ok() && self.session.libraries().inspect().as_array().into_iter().flatten().any(|source| source["library"] == draft.id) {
            valid = Err("This library ID already exists. Choose a new ID, or use Add library for another copy.".into());
        }
        let action = pcbcraft_chrome::catalog::property_form_enabled(
            ctx,
            "new_native_library",
            "New library",
            452.0,
            "Choose folder…",
            valid.is_ok(),
            |ui| {
                let previous_name = draft.name.clone();
                if properties::text(ui, "Name", &mut draft.name).changed() && draft.id == collection_id(&previous_name) {
                    draft.id = collection_id(&draft.name);
                }
                properties::text(ui, "Library ID", &mut draft.id);
                properties::text(ui, "License", &mut draft.license);
                if let Some(error) = &draft.error {
                    ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
                }
                if draft.error.is_none()
                    && let Err(error) = &valid
                {
                    ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
                }
            },
        );
        if action.primary {
            let result = engine::new_library(&draft.id, &draft.name, &draft.license)
                .and_then(|manifest| self.create_native_library(ctx, manifest, &draft.id, draft.return_to.clone()));
            if let Err(error) = result {
                draft.error = Some(error);
                self.dialog = Dialog::NewLibrary(draft);
            }
        } else if !action.close {
            self.dialog = Dialog::NewLibrary(draft);
        } else if let Some(return_to) = draft.return_to {
            self.dialog = Dialog::LibraryPublish(*return_to);
        }
    }

    /// Ordinary publication and explicit Save As use one registered-source handler.
    pub(crate) fn publish_library_item(
        &mut self,
        ctx: &Context,
        document: u64,
        revision: u64,
        source: &str,
        allow_older: bool,
    ) -> Result<(), String> {
        self.publish_library_item_as(ctx, document, revision, source, allow_older, false)
    }

    /// Destination changes prepare an explicit derivative; only host-confirmed writes rebind it.
    pub(crate) fn publish_library_item_as(
        &mut self,
        ctx: &Context,
        document: u64,
        revision: u64,
        source: &str,
        allow_older: bool,
        copy: bool,
    ) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, document, revision, source, allow_older, copy);
            Err("Publishing is unavailable in the browser.".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.library_editors.busy() || self.library_sources.busy() || self.library_mutation_busy() {
                return Err("Finish the current library operation first.".into());
            }
            let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
            if revision != doc.session.revision() || doc.pending_any() {
                return Err("Library item changed or has unapplied fields; review it before publishing.".into());
            }
            if let Some(recovery) = self.library_editors.publisher.recovery.clone() {
                if recovery.document != document {
                    return Err("Verify the previous uncertain publication before publishing another item.".into());
                }
                let result = self.library_editors.publisher.start(ctx, move |_| verify_publication(recovery));
                if result.is_ok() {
                    self.message = None;
                    self.library_editors.notice = Some("Verifying published revision…".into());
                }
                return result;
            }
            if !self.writable_libraries().iter().any(|(id, _)| id == source) {
                return Err("Choose an available writable native folder library.".into());
            }
            let loaded = self.session.libraries().inspect();
            let entry = loaded.as_array().into_iter().flatten().find(|entry| entry["source"] == source).ok_or("Library is unavailable.")?;
            let library = entry["library"].as_str().ok_or("Missing library identity.")?;
            let item = doc.session.snapshot();
            let copy = copy || (item.library != "user" && item.library != library);
            if !copy && doc.session.published().is_some() && !doc.session.publication_dirty() {
                return Err("This revision is already published.".into());
            }
            let manifest = engine::new_library(library, entry["name"].as_str().unwrap_or(library), entry["license"].as_str().unwrap_or_default())?;
            let latest = self.session.libraries().highest_revision(library, source_kind(doc.session.snapshot().kind), &doc.session.snapshot().id);
            // The worker reads the exact on-disk manifest; preparation there still uses the
            // captured revision/catalog authority and the shared engine function below.
            let item = doc.session.snapshot();
            let base = doc.session.published().map(|address| address.revision);
            if copy {
                engine::prepare_copy(item.clone(), &manifest)?;
            } else {
                engine::prepare(item.clone(), &manifest, latest, base, allow_older)?;
            }
            let path = self.library_sources.registered.iter().find(|r| r.source == source).ok_or("Missing library registration.")?.path.clone();
            let source = source.to_owned();
            let generation = self.session.libraries().generation();
            let source_for_notice = source.clone();
            let result = self.library_editors.publisher.start(ctx, move |send| {
                let disk_manifest = pcbcraft_storage::library::publishing::read_manifest(std::path::Path::new(&path))?;
                if serde_json::from_str::<Value>(&disk_manifest).map_err(|e| e.to_string())?
                    != serde_json::from_str::<Value>(&manifest).map_err(|e| e.to_string())?
                {
                    return Err("Destination library changed; refresh before publishing.".into());
                }
                let prepared = if copy {
                    engine::prepare_copy(item, &disk_manifest)?
                } else {
                    engine::prepare(item, &disk_manifest, latest, base, allow_older)?
                };
                let request = Request { document, source, path, generation, prepared };
                // Retain retry identity on the UI side before any final record can become visible.
                send.send(Event::Prepared(request.clone())).map_err(|_| "Publisher owner disconnected before writing.")?;
                match pcbcraft_storage::library::publishing::publish(std::path::Path::new(&request.path), &request.prepared) {
                    Ok(()) => Ok(refresh_publication(request)),
                    Err(pcbcraft_storage::library::publishing::Failure::BeforeWrite(error)) => Ok(Completion::NotWritten(error)),
                    Err(pcbcraft_storage::library::publishing::Failure::Published(error)) => Ok(Completion::Unconfirmed(request, error)),
                }
            });
            if result.is_ok() {
                self.message = None;
                if let Some(doc) = self.library_editors.documents.get_mut(&document) {
                    doc.publish_source = Some(source_for_notice);
                }
                self.library_editors.notice = Some(if copy { "Publishing copy…" } else { "Publishing…" }.into());
            }
            result
        }
    }

    /// Start one create worker after validating capacity; cancellation registers nothing.
    fn create_native_library(&mut self, ctx: &Context, manifest: String, folder_name: &str, return_to: Option<Box<Draft>>) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, manifest, folder_name, return_to);
            Err("Native library creation is desktop-only.".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let library = engine::destination(&manifest)?;
            if self.session.libraries().inspect().as_array().into_iter().flatten().any(|source| source["library"] == library) {
                return Err("This library ID already exists. Choose a new ID, or add another copy of the existing library.".into());
            }
            if self.library_editors.busy() || self.library_mutation_busy() || !self.library_sources.can_add(&self.session) {
                return Err("Finish the current library operation first.".into());
            }
            let source = self.library_sources.next_source(&self.session);
            let generation = self.session.libraries().generation();
            let name = folder_name.to_owned();
            let result = self.library_editors.publisher.start(ctx, move |_| {
                let Some(path) = rfd::FileDialog::new().set_title("Create library in a new folder").set_file_name(name).save_file() else {
                    return Ok(Completion::Cancelled(return_to));
                };
                let locator = path.to_str().filter(|s| s.len() <= 4096).ok_or("Choose a UTF-8 folder path of at most 4096 bytes.")?.to_owned();
                let parsed = pcbcraft_storage::library::publishing::create(&path, &manifest)?;
                Ok(Completion::Created { source, path: locator, parsed: Arc::new(parsed), generation, return_to })
            });
            if result.is_ok() {
                self.message = None;
                self.library_editors.notice = Some("Creating library…".into());
            }
            result
        }
    }

    /// Adopt only durably confirmed snapshots. Refresh failures are separate and never cause a
    /// second publication; newer edits and project history remain untouched.
    pub(crate) fn poll_library_publication(&mut self, ctx: &Context) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if matches!(self.dialog, Dialog::None)
                && !self.library_editors.busy()
                && !self.files.busy()
                && !self.library_sources.busy()
                && !self.library_mutation_busy()
                && let Some(draft) = self.library_editors.publisher.resume.take()
            {
                self.resume_library_publish(draft);
            }
            let Some(receive) = self.library_editors.publisher.pending.as_ref() else { return };
            let result = match receive.try_recv() {
                Ok(Event::Prepared(request)) => {
                    self.library_editors.publisher.recovery = Some(request);
                    ctx.request_repaint();
                    return;
                }
                Ok(Event::Finished(result)) => result,
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    self.library_editors.publisher.pending = None;
                    self.library_editors.notice =
                        Some("Library publisher stopped unexpectedly. Publish again to verify the captured revision before retrying.".into());
                    return;
                }
            };
            self.library_editors.publisher.pending = None;
            self.library_editors.notice = Some(match result {
                Ok(Completion::Cancelled(return_to)) => {
                    self.library_editors.notice = None;
                    if let Some(draft) = return_to {
                        self.resume_library_publish(draft);
                    }
                    ctx.request_repaint();
                    return;
                }
                Ok(Completion::NotWritten(error)) => {
                    self.library_editors.publisher.recovery = None;
                    error
                }
                Err(error) => error,
                Ok(Completion::Unconfirmed(request, error)) => {
                    self.library_editors.publisher.recovery = Some(request);
                    format!("{error}. Publish again to verify this same revision without writing another.")
                }
                Ok(Completion::Published(request, refresh)) => {
                    if let Some(doc) = self.library_editors.documents.get_mut(&request.document) {
                        doc.session.confirm_publication(&request.prepared);
                    }
                    self.library_editors.publisher.recovery = None;
                    let saved =
                        format!("Published {}/{} r{}", request.prepared.item().library, request.prepared.item().id, request.prepared.item().revision);
                    match self.adopt_published_source(&request.source, &request.path, request.generation, refresh) {
                        Ok(()) => saved,
                        Err(error) => format!("{saved}. Refresh failed: {error}. The revision is saved; refresh the source without republishing."),
                    }
                }
                Ok(Completion::Created { source, path, parsed, generation, return_to }) => {
                    match self.adopt_published_source(&source, &path, generation, Ok(parsed)) {
                        Ok(()) => {
                            if let Some(mut draft) = return_to
                                && let Some(doc) = self.library_editors.documents.get_mut(&draft.document)
                            {
                                doc.publish_source = Some(source.clone());
                                draft.source = source;
                                draft.revision = doc.session.revision();
                                draft.error = None;
                                self.resume_library_publish(draft);
                            }
                            "Created and registered library".into()
                        }
                        Err(error) => format!("Library was created at {path}, but registration failed: {error}. Add its existing folder to recover."),
                    }
                }
            });
            self.repaint_editors(ctx);
        }
    }

    /// A delayed library creation never replaces another dialog or a close decision. Its
    /// originating document is checked again when that decision has finished.
    #[cfg(not(target_arch = "wasm32"))]
    fn resume_library_publish(&mut self, mut draft: Box<Draft>) {
        let Some(doc) = self.library_editors.documents.get(&draft.document) else { return };
        draft.revision = doc.session.revision();
        if matches!(self.dialog, Dialog::None) {
            self.dialog = Dialog::LibraryPublish(*draft);
        } else {
            self.library_editors.publisher.resume = Some(draft);
        }
    }

    /// A delayed refresh cannot replace a catalog changed through another host or MCP call.
    #[cfg(not(target_arch = "wasm32"))]
    fn adopt_published_source(
        &mut self,
        source: &str,
        path: &str,
        generation: u64,
        refresh: Result<Arc<sources::NativeSource>, String>,
    ) -> Result<(), String> {
        if generation != self.session.libraries().generation() {
            return Err("Library sources changed while publishing.".into());
        }
        let parsed = refresh?;
        self.session.install_library_source(source, parsed).map_err(|e| e.to_string())?;
        let data = self.session.libraries().inspect();
        let name = data
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["source"] == source)
            .and_then(|entry| entry["name"].as_str())
            .unwrap_or(source)
            .to_owned();
        self.library_sources.registered.retain(|registration| registration.source != source);
        self.library_sources.registered.push(crate::native_sources::Registration { source: source.into(), path: path.into(), name, writable: true });
        self.library_sources.statuses.insert(source.into(), "Available".into());
        Ok(())
    }

    /// Open the selected catalog revision with its identity intact, or explicitly create a fork.
    /// Both paths use the normal document and pane host, leaving pinned project records untouched.
    pub(crate) fn edit_catalog_selection(&mut self, ctx: &Context, copy: bool) -> Result<(), String> {
        let preferred = self.preferred_library_source();
        let state = self.views.active.library.as_ref().ok_or("Choose a Parts Library pane first.")?;
        let key = state.selected_key().ok_or("Select a symbol or footprint first.")?;
        let project = state.project_id();
        if project.is_some() && project != self.session.project().map(|project| project.id()) {
            return Err("This project selection is no longer current.".into());
        }
        if !copy && !state.source_id().is_some_and(|source| self.writable_libraries().iter().any(|(id, _)| id == source)) {
            return Err("This library is read-only; use Edit copy instead.".into());
        }
        let catalog = self.session.libraries();
        if project.is_none() && catalog.conflicting(key) {
            return Err("Resolve this item's conflicting revision before editing.".into());
        }
        let inventory = project.map(|_| self.project_library_inventory()).transpose()?;
        let pinned = inventory.as_ref().and_then(|inventory| inventory.item(key));
        let item = match key.kind {
            sources::Kind::Symbol => {
                let definition =
                    if project.is_some() { pinned.and_then(|item| item.symbol()) } else { catalog.symbol(&key.library, &key.id, key.revision) }
                        .ok_or("Selected symbol is unavailable.")?;
                if copy { Item::from_symbol(&definition)? } else { Item::edit_symbol(&definition)? }
            }
            sources::Kind::Footprint => {
                let definition =
                    if project.is_some() { pinned.and_then(|item| item.footprint()) } else { catalog.footprint(&key.library, &key.id, key.revision) }
                        .ok_or("Selected footprint is unavailable.")?;
                if copy { Item::from_footprint(&definition)? } else { Item::edit_footprint(&definition)? }
            }
            _ => return Err("This editor supports symbols and footprints.".into()),
        };
        let id = self.library_editors.open(item, !copy)?;
        self.library_editors.suggest_destination(id, preferred);
        if !copy {
            self.library_editors.documents.get_mut(&id).ok_or("Missing library document.")?.session.bind_published();
        }
        self.place_library_document(ctx, id, pcbcraft_chrome::workspace::Placement::Here)
    }

    /// Explicit editor forks retain unfinished geometry but receive a fresh native item identity.
    pub(super) fn copy_library_document(&mut self, ctx: &Context, document: u64) -> Result<(), String> {
        let preferred = self.preferred_library_source();
        let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
        if doc.pending_any() {
            return Err("Apply or reset property fields before making a copy.".into());
        }
        let item = doc.session.snapshot().edit_copy()?;
        let id = self.library_editors.open(item, false)?;
        self.library_editors.suggest_destination(id, preferred);
        self.place_library_document(ctx, id, pcbcraft_chrome::workspace::Placement::Here)
    }

    /// Keep the collection and publication state visible using the ordinary workspace status bar.
    pub(super) fn library_publication_status(&self, document: u64) -> Option<(String, &'static str)> {
        let doc = self.library_editors.documents.get(&document)?;
        if IN_BROWSER {
            return Some(("Draft file".into(), if doc.unsaved() { "Unsaved changes" } else { "Unchanged" }));
        }
        let name = doc
            .publish_source
            .as_ref()
            .and_then(|source| self.library_sources.registered.iter().find(|entry| entry.source == *source))
            .map(|entry| entry.name.as_str());
        let left = if let Some(address) = doc.session.published() {
            let loaded = self.session.libraries().inspect();
            let name = loaded
                .as_array()
                .into_iter()
                .flatten()
                .find(|source| source["library"] == address.library)
                .and_then(|source| source["name"].as_str())
                .unwrap_or(&address.library);
            format!("Library: {name} · r{}", address.revision)
        } else if let Some(name) = name {
            format!("Publish to: {name}")
        } else if doc.session.snapshot().library != "user" {
            format!("Library: {} · Draft", doc.session.snapshot().library)
        } else {
            "Library not selected".into()
        };
        let right = if doc.pending_any() {
            "Unapplied fields"
        } else if doc.session.published().is_none() {
            if doc.session.snapshot().library == "user" { "Not published" } else { "Draft file" }
        } else if doc.session.publication_dirty() {
            "Unpublished changes"
        } else {
            "Published"
        };
        Some((left, right))
    }
}

/// Authoring kinds map explicitly to catalog kinds, without interpreting file extensions.
fn source_kind(kind: Kind) -> sources::Kind {
    if kind == Kind::Symbol { sources::Kind::Symbol } else { sources::Kind::Footprint }
}

/// Suggest a portable ID as the name changes, while retaining any independently edited ID.
fn collection_id(name: &str) -> String {
    let mut id = String::new();
    for character in name.chars().take(64) {
        if character.is_ascii_alphanumeric() {
            id.push(character.to_ascii_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches('-');
    if id.is_empty() { "library".into() } else { id.into() }
}

/// Refresh is intentionally a separate result after a successful durable write.
#[cfg(not(target_arch = "wasm32"))]
fn refresh_publication(request: Request) -> Completion {
    let refresh = pcbcraft_storage::library::load(std::path::Path::new(&request.path)).map(Arc::new);
    Completion::Published(request, refresh)
}

/// An uncertain save retries confirmation of the same exact bytes, never publication allocation.
#[cfg(not(target_arch = "wasm32"))]
fn verify_publication(request: Request) -> Result<Completion, String> {
    match pcbcraft_storage::library::publishing::recover(std::path::Path::new(&request.path), &request.prepared) {
        Ok(()) => Ok(refresh_publication(request)),
        Err(pcbcraft_storage::library::publishing::Failure::Published(error)) => Ok(Completion::Unconfirmed(request, error)),
        Err(pcbcraft_storage::library::publishing::Failure::BeforeWrite(error)) => Ok(Completion::Unconfirmed(request, error)),
    }
}
