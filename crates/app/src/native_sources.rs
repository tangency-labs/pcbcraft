//! Source registration and background loading are application services, separate from project files.
use crate::{Dialog, PcbApp};
use pcbcraft_chrome::library_sources as chrome;
use pcbcraft_engine::library::sources::{self, Kind, NativeSource};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{
    Arc,
    mpsc::{self, Receiver, TryRecvError},
};

pub(super) const STORAGE_KEY: &str = "pcbcraft.native-library-sources.v1";

/// Personal locators never enter project documents. Uploaded sources last only for this session.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    source: String,
    path: String,
}

#[derive(Debug)]
struct Job {
    generation: u64,
    source: String,
    receive: Receiver<LoadResult>,
}

/// None is a cancelled picker; a successful load carries parsed content and an optional personal locator.
type LoadResult = Result<Option<(Arc<NativeSource>, Option<String>)>, String>;

/// Retain one worker slot until completion, including after removal, so a stalled filesystem
/// cannot accumulate unbounded workers. Dropping the app never joins a blocking reader.
#[derive(Debug, Default)]
pub(super) struct Sources {
    registered: Vec<Registration>,
    pending: Vec<Registration>,
    job: Option<Job>,
    statuses: std::collections::BTreeMap<String, String>,
    next: u64,
    query: String,
    kind: Option<Kind>,
}

impl Sources {
    /// Restore only a bounded, valid personal source list; disk reads start later on workers.
    pub(super) fn restore(text: Option<String>) -> Self {
        let registered = text
            .filter(|text| text.len() <= 64 * 1024)
            .and_then(|text| serde_json::from_str::<Vec<Registration>>(&text).ok())
            .filter(|entries| {
                entries.len() <= sources::MAX_SOURCES
                    && entries.iter().all(|r| {
                        r.source != "builtin"
                            && !r.source.is_empty()
                            && r.source.len() <= 64
                            && r.source.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                            && !r.path.is_empty()
                            && r.path.len() <= 4096
                    })
                    && entries.iter().map(|r| &r.source).collect::<std::collections::BTreeSet<_>>().len() == entries.len()
            })
            .unwrap_or_default();
        Self { pending: registered.clone(), registered, ..Self::default() }
    }
    /// Persist only desktop folder registrations, excluding cached definitions and status text.
    pub(super) fn storage_text(&self) -> String {
        serde_json::to_string(&self.registered).unwrap_or_else(|_| "[]".into())
    }

    /// Allocate a session source ID that cannot collide with restored or MCP-loaded registrations.
    fn next_source(&mut self, session: &pcbcraft_engine::Session) -> String {
        loop {
            self.next += 1;
            let source = format!("native-{}", self.next);
            if !self.registered.iter().any(|r| r.source == source)
                && !self.statuses.contains_key(&source)
                && !session.libraries().inspect().as_array().into_iter().flatten().any(|s| s["source"] == source)
            {
                return source;
            }
        }
    }

    /// Include unavailable and pending registrations in the limit, so repeated failed additions
    /// cannot grow personal settings or the manager list beyond the catalog's source bound.
    fn can_add(&self, session: &pcbcraft_engine::Session) -> bool {
        let mut ids: std::collections::BTreeSet<String> =
            self.registered.iter().map(|r| r.source.clone()).chain(self.statuses.keys().cloned()).collect();
        ids.extend(
            session
                .libraries()
                .inspect()
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["source"].as_str())
                .filter(|id| *id != "builtin")
                .map(str::to_owned),
        );
        self.job.is_none() && ids.len() < sources::MAX_SOURCES
    }

    /// Start exactly one folder read. The same parsed snapshot is installed by the source command
    /// transaction; errors and disconnects are reported to the registration that owns the request.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_path(&mut self, source: String, path: String, ctx: &egui::Context, generation: u64) -> Result<(), String> {
        if self.job.is_some() {
            return Err("A library read is already running".into());
        }
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("pcbcraft-native-library".into())
            .spawn(move || {
                let result = pcbcraft_storage::library::load(std::path::Path::new(&path)).map(|source| Some((Arc::new(source), Some(path))));
                let _ = send.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        self.statuses.insert(source.clone(), "Loading…".into());
        self.job = Some(Job { source, receive, generation });
        Ok(())
    }

    /// Open a desktop picker inside the worker. Cancel returns no registration or catalog change.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_folder(&mut self, source: String, ctx: &egui::Context, generation: u64) -> Result<(), String> {
        if self.job.is_some() {
            return Err("A library read is already running".into());
        }
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("pcbcraft-library-picker".into())
            .spawn(move || {
                let result = rfd::FileDialog::new()
                    .set_title("Native PCBcraft library folder")
                    .pick_folder()
                    .map(|path| {
                        let locator = path.to_str().ok_or("Library folder path is not valid UTF-8")?.to_owned();
                        pcbcraft_storage::library::load(&path).map(|source| (Arc::new(source), Some(locator)))
                    })
                    .transpose();
                let _ = send.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        self.statuses.insert(source.clone(), "Loading…".into());
        self.job = Some(Job { source, receive, generation });
        Ok(())
    }

    /// Upload uses the same bundle decoder on desktop and browser; browser selection is initiated
    /// synchronously during the click to retain its user-activation permission.
    fn upload(&mut self, source: String, ctx: &egui::Context, generation: u64) -> Result<(), String> {
        if self.job.is_some() {
            return Err("A library read is already running".into());
        }
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("pcbcraft-library-upload".into())
            .spawn(move || {
                use std::io::Read;
                let result = rfd::FileDialog::new()
                    .add_filter("PCBcraft library bundle", &["json"])
                    .pick_file()
                    .map(|path| {
                        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                        let mut bytes = Vec::new();
                        file.take((sources::MAX_SOURCE_BYTES * 2 + 1) as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                        NativeSource::from_bundle(&bytes).map(|source| (Arc::new(source), None))
                    })
                    .transpose();
                let _ = send.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        #[cfg(target_arch = "wasm32")]
        {
            let promise = web::pick();
            wasm_bindgen_futures::spawn_local(async move {
                let result = web::read(promise).await;
                let _ = send.send(result);
                ctx.request_repaint();
            });
        }
        self.statuses.insert(source.clone(), "Loading…".into());
        self.job = Some(Job { source, receive, generation });
        Ok(())
    }

    /// Remove registration immediately but retain an in-flight slot; late results cannot restore it.
    fn remove(&mut self, source: &str, session: &mut pcbcraft_engine::Session) {
        self.registered.retain(|r| r.source != source);
        self.pending.retain(|r| r.source != source);
        self.statuses.remove(source);
        let _ = session.execute(sources::REMOVE, &json!({"source":source}));
    }

    /// Consume a worker result only while its registration request is still current. Failed
    /// refreshes remove stale catalog content without touching pinned project copies or history.
    fn poll(&mut self, session: &mut pcbcraft_engine::Session, ctx: &egui::Context) {
        let result = self.job.as_ref().map(|job| job.receive.try_recv());
        if let Some(result) = result {
            match result {
                Ok(result) => {
                    if let Some(job) = self.job.take()
                        && self.statuses.contains_key(&job.source)
                    {
                        if job.generation != session.libraries().generation() {
                            self.statuses.insert(job.source, "Library sources changed during loading; load this source again".into());
                            ctx.request_repaint();
                            return;
                        }
                        match result {
                            Ok(Some((source, path))) => match session.install_library_source(&job.source, source) {
                                Ok(()) => {
                                    self.statuses.insert(job.source.clone(), "Available".into());
                                    if let Some(path) = path {
                                        self.registered.retain(|r| r.source != job.source);
                                        self.registered.push(Registration { source: job.source, path });
                                    }
                                }
                                Err(error) => {
                                    session.mark_library_unavailable(&job.source);
                                    self.statuses.insert(job.source, error.to_string());
                                }
                            },
                            Ok(None) => {
                                self.statuses.remove(&job.source);
                            }
                            Err(error) => {
                                session.mark_library_unavailable(&job.source);
                                self.statuses.insert(job.source, error);
                            }
                        }
                        ctx.request_repaint();
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    if let Some(job) = self.job.take()
                        && self.statuses.contains_key(&job.source)
                    {
                        if job.generation != session.libraries().generation() {
                            self.statuses.insert(job.source, "Library sources changed during loading; load this source again".into());
                            ctx.request_repaint();
                            return;
                        }
                        session.mark_library_unavailable(&job.source);
                        self.statuses.insert(job.source, "Library reader disconnected".into());
                    }
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.job.is_none() && !self.pending.is_empty() {
            let registration = self.pending.remove(0);
            if let Err(error) = self.load_path(registration.source.clone(), registration.path, ctx, session.libraries().generation()) {
                self.statuses.insert(registration.source, error);
            }
        }
    }
}

impl PcbApp {
    /// Bind a placement command to the exact definition shown by the preview, so source refresh
    /// or removal cannot substitute geometry while the project's revision remains unchanged.
    /// Starter-only placement retains its legacy saved-copy resolution behavior.
    pub(super) fn place_catalog_symbol(&mut self, definition: &pcbcraft_engine::schematic::SymbolDefinition, mut params: serde_json::Value) {
        params["definition"] = json!(definition.id);
        if definition.library == "starter" {
            self.run(pcbcraft_engine::PLACE_SYMBOL, &params);
            return;
        }
        match sources::symbol_digest(definition) {
            Ok(digest) => {
                params["library"] = json!(definition.library);
                params["definition_revision"] = json!(definition.revision());
                params["definition_digest"] = json!(digest);
                self.run(pcbcraft_engine::PLACE_SYMBOL, &params);
            }
            Err(error) => self.message = Some(error),
        }
    }

    /// Poll the shared catalog service once per root frame, independently of project replacement.
    pub(super) fn poll_library_sources(&mut self, ctx: &egui::Context) {
        let generation = self.session.libraries().generation();
        self.library_sources.poll(&mut self.session, ctx);
        if self.session.libraries().generation() != generation {
            self.repaint_editors(ctx);
        }
    }

    /// Manage personal sources and inspect footprints without coupling to assignment or Properties.
    /// Closing the dialog keeps registrations; removing one never edits pinned project data.
    pub(super) fn library_sources_dialog(&mut self, ctx: &egui::Context) {
        let mut remove: Option<String> = None;
        #[cfg(not(target_arch = "wasm32"))]
        let mut refresh: Option<Registration> = None;
        #[cfg(not(target_arch = "wasm32"))]
        let mut relocate: Option<String> = None;
        #[cfg(not(target_arch = "wasm32"))]
        let mut add = false;
        let mut upload = false;
        let loaded = self.session.libraries().inspect();
        let can_add = self.library_sources.can_add(&self.session);
        let mut ids: std::collections::BTreeSet<String> =
            loaded.as_array().into_iter().flatten().filter_map(|s| s["source"].as_str().map(str::to_owned)).collect();
        ids.extend(self.library_sources.statuses.keys().cloned());
        ids.extend(self.library_sources.registered.iter().map(|r| r.source.clone()));
        let closing = chrome::dialog(ctx, |ui| {
            chrome::panes(ui, |ui, pane| {
                if pane == 0 {
                    chrome::heading(ui, "Sources", ids.len());
                    let imports = chrome::imports(ui, can_add, cfg!(not(target_arch = "wasm32")));
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        add = imports.0;
                    }
                    upload = imports.1;
                    chrome::list(ui, "source_registrations", |ui| {
                        for id in &ids {
                            let data = loaded.as_array().into_iter().flatten().find(|s| s["source"] == *id);
                            let title = data.and_then(|s| s["name"].as_str()).unwrap_or(id);
                            let registration = self.library_sources.registered.iter().find(|r| r.source == *id);
                            let problems: Vec<&str> =
                                data.into_iter().flat_map(|s| s["problems"].as_array()).flatten().filter_map(|p| p.as_str()).collect();
                            let source = chrome::Source {
                                name: title,
                                library: data.and_then(|s| s["library"].as_str()).unwrap_or_default(),
                                license: data.and_then(|s| s["license"].as_str()).unwrap_or_default(),
                                location: registration.map(|r| r.path.as_str()),
                                status: self.library_sources.statuses.get(id).map(String::as_str),
                                problems: &problems,
                                symbols: data.and_then(|s| s["symbols"].as_u64()).unwrap_or_default(),
                                footprints: data.and_then(|s| s["footprints"].as_u64()).unwrap_or_default(),
                                builtin: id == "builtin",
                                available: data.is_some_and(|s| s["available"] != false),
                                loading: self.library_sources.job.as_ref().is_some_and(|job| job.source == *id),
                            };
                            let action = ui.push_id(id, |ui| chrome::source(ui, source, self.library_sources.job.is_none())).inner;
                            match action {
                                chrome::SourceAction::Remove => remove = Some(id.clone()),
                                #[cfg(not(target_arch = "wasm32"))]
                                chrome::SourceAction::Refresh => refresh = registration.cloned(),
                                #[cfg(not(target_arch = "wasm32"))]
                                chrome::SourceAction::Relocate => relocate = Some(id.clone()),
                                _ => {}
                            }
                        }
                    });
                } else {
                    // Search/filter changes are view state and never transact project data.
                    let mut kind = match self.library_sources.kind {
                        None => chrome::Filter::All,
                        Some(Kind::Symbol) => chrome::Filter::Symbols,
                        Some(Kind::Footprint) => chrome::Filter::Footprints,
                    };
                    chrome::search(ui, &mut self.library_sources.query);
                    chrome::filters(ui, &mut kind);
                    self.library_sources.kind = match kind {
                        chrome::Filter::All => None,
                        chrome::Filter::Symbols => Some(Kind::Symbol),
                        chrome::Filter::Footprints => Some(Kind::Footprint),
                    };
                    let mut params = json!({"query":self.library_sources.query});
                    if let Some(kind) = self.library_sources.kind {
                        params["kind"] = json!(kind);
                    }
                    if let Ok(items) = self.session.execute(sources::SEARCH, &params) {
                        let items: Vec<chrome::Item<'_>> = items
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|item| chrome::Item {
                                name: item["name"].as_str().unwrap_or_default(),
                                library: item["key"]["library"].as_str().unwrap_or_default(),
                                id: item["key"]["id"].as_str().unwrap_or_default(),
                                revision: item["key"]["revision"].as_u64().unwrap_or_default(),
                                footprint: item["key"]["kind"] == "footprint",
                                license: item["license"].as_str().unwrap_or_default(),
                            })
                            .collect();
                        chrome::heading(ui, "Catalog", items.len());
                        chrome::results(ui, &items);
                    }
                }
            });
        });
        if let Some(source) = remove {
            self.library_sources.remove(&source, &mut self.session);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if add {
                let source = self.library_sources.next_source(&self.session);
                if let Err(error) = self.library_sources.pick_folder(source.clone(), ctx, self.session.libraries().generation()) {
                    self.library_sources.statuses.insert(source, error);
                }
            }
            if let Some(registration) = refresh
                && let Err(error) =
                    self.library_sources.load_path(registration.source.clone(), registration.path, ctx, self.session.libraries().generation())
            {
                self.library_sources.statuses.insert(registration.source, error);
            }
            if let Some(source) = relocate
                && let Err(error) = self.library_sources.pick_folder(source.clone(), ctx, self.session.libraries().generation())
            {
                self.library_sources.statuses.insert(source, error);
            }
        }
        if upload {
            let source = self.library_sources.next_source(&self.session);
            if let Err(error) = self.library_sources.upload(source.clone(), ctx, self.session.libraries().generation()) {
                self.library_sources.statuses.insert(source, error);
            }
        }
        if !closing {
            self.dialog = Dialog::LibrarySources;
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(
        inline_js = "export function pickLibraryBundle(limit) { return new Promise((resolve,reject)=>{ const input=document.createElement('input'); input.type='file'; input.accept='.json'; input.oncancel=()=>resolve(null); input.onchange=async()=>{ const file=input.files[0]; if(!file)return resolve(null); if(file.size>limit)return reject(new Error('Library bundle exceeds 16 MiB')); try{resolve(await file.arrayBuffer())}catch(e){reject(e)} }; input.click(); }); }"
    )]
    extern "C" {
        #[wasm_bindgen(js_name=pickLibraryBundle)]
        fn pick_bundle(limit: u32) -> js_sys::Promise;
    }
    /// Begin browser selection during the triggering click, before yielding the event loop.
    pub(super) fn pick() -> js_sys::Promise {
        pick_bundle((sources::MAX_SOURCE_BYTES * 2) as u32)
    }
    /// Bound picked bytes before copying into Wasm memory and use the native snapshot validator.
    pub(super) async fn read(promise: js_sys::Promise) -> LoadResult {
        let value = wasm_bindgen_futures::JsFuture::from(promise).await.map_err(|e| format!("Could not upload library: {e:?}"))?;
        if value.is_null() {
            return Ok(None);
        }
        let bytes = js_sys::Uint8Array::new(&value);
        if bytes.length() as usize > sources::MAX_SOURCE_BYTES * 2 {
            return Err("Library bundle exceeds 16 MiB".into());
        }
        NativeSource::from_bundle(&bytes.to_vec()).map(|source| Some((Arc::new(source), None)))
    }
}

#[cfg(test)]
mod tests;
