//! Source registration and background loading are application services, separate from project files.
use crate::PcbApp;
pub(super) mod lifecycle;
use pcbcraft_engine::library::sources::{self, NativeSource};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{
    Arc,
    mpsc::{self, Receiver, TryRecvError},
};

pub(super) const STORAGE_KEY: &str = "pcbcraft.native-library-sources.v1";

/// Personal locators never enter project documents. Browser registrations retain names only.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Registration {
    pub(super) source: String,
    pub(super) path: String,
    #[serde(default)]
    pub(super) name: String,
    #[serde(skip)]
    pub(super) writable: bool,
}

#[derive(Debug)]
struct Job {
    generation: u64,
    installing: bool,
    source: String,
    /// Restore this diagnostic if the picker is cancelled while this request is still current.
    previous_status: Option<String>,
    receive: Receiver<LoadResult>,
}

/// None is a cancelled picker; a successful load carries parsed content and an optional personal locator.
type LoadResult = Result<Option<(Arc<NativeSource>, Option<String>, bool)>, String>;

/// Retain one worker slot until completion, including after removal, so a stalled filesystem
/// cannot accumulate unbounded workers. Dropping the app never joins a blocking reader.
#[derive(Debug, Default)]
pub(super) struct Sources {
    pub(super) registered: Vec<Registration>,
    pending: Vec<Registration>,
    job: Option<Job>,
    pub(super) statuses: std::collections::BTreeMap<String, String>,
    next: u64,
    #[cfg(target_arch = "wasm32")]
    last_browser_text: Option<String>,
    mutation: Option<lifecycle::Job>,
}

impl Sources {
    /// Remember browser names immediately, including Remove, rather than depending on a later
    /// periodic application save. Storage errors are reported once per changed registration list.
    #[cfg(target_arch = "wasm32")]
    fn persist_browser(&mut self) -> Result<(), String> {
        let text = self.storage_text();
        if self.last_browser_text.as_ref() == Some(&text) {
            return Ok(());
        }
        self.last_browser_text = Some(text.clone());
        web::remember_sources(STORAGE_KEY, &text).map_err(|e| format!("Could not remember library names in this browser: {e:?}"))
    }

    /// Restore only a bounded, valid personal source list; disk reads start later on workers.
    pub(super) fn restore(text: Option<String>) -> Self {
        let registered = text
            .filter(|text| text.len() <= 64 * 1024)
            .and_then(|text| serde_json::from_str::<Vec<Registration>>(&text).ok())
            .filter(|entries| {
                entries.len() <= sources::MAX_SOURCES
                    && entries.iter().all(|r| {
                        !r.source.is_empty()
                            && r.source.len() <= 64
                            && r.source.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                            && (cfg!(target_arch = "wasm32") || !r.path.is_empty())
                            && r.path.len() <= 4096
                    })
                    && entries.iter().map(|r| &r.source).collect::<std::collections::BTreeSet<_>>().len() == entries.len()
            })
            .unwrap_or_default();
        let pending = if cfg!(target_arch = "wasm32") { Vec::new() } else { registered.clone() };
        let statuses = if cfg!(target_arch = "wasm32") {
            registered.iter().map(|r| (r.source.clone(), "Needs reopening".into())).collect()
        } else {
            Default::default()
        };
        Self { pending, registered, statuses, ..Self::default() }
    }
    /// Persist personal folder registrations on desktop and names/IDs on the browser; never cache definitions.
    pub(super) fn storage_text(&self) -> String {
        serde_json::to_string(&self.registered).unwrap_or_else(|_| "[]".into())
    }

    /// Allocate a session source ID that cannot collide with restored or MCP-loaded registrations.
    pub(super) fn next_source(&mut self, session: &pcbcraft_engine::Session) -> String {
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
    pub(super) fn can_add(&self, session: &pcbcraft_engine::Session) -> bool {
        let mut ids: std::collections::BTreeSet<String> =
            self.registered.iter().map(|r| r.source.clone()).chain(self.statuses.keys().cloned()).collect();
        ids.extend(session.libraries().inspect().as_array().into_iter().flatten().filter_map(|s| s["source"].as_str()).map(str::to_owned));
        self.job.is_none() && self.mutation.is_none() && ids.len() < sources::MAX_SOURCES
    }

    /// Start exactly one folder read. The same parsed snapshot is installed by the source command
    /// transaction; errors and disconnects are reported to the registration that owns the request.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn load_path(&mut self, source: String, path: String, ctx: &egui::Context, generation: u64) -> Result<(), String> {
        if self.job.is_some() || self.mutation.is_some() {
            return Err("A library read is already running".into());
        }
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("pcbcraft-native-library".into())
            .spawn(move || {
                let root = std::path::Path::new(&path);
                let writable = pcbcraft_storage::library::writable(root);
                let result = pcbcraft_storage::library::load(root).map(|source| Some((Arc::new(source), Some(path), writable)));
                let _ = send.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        let previous_status = self.statuses.insert(source.clone(), "Loading…".into());
        self.job = Some(Job { source, previous_status, receive, generation, installing: false });
        Ok(())
    }

    /// Open a desktop picker inside the worker. Cancel preserves registration, catalog and prior status.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn pick_folder(&mut self, source: String, ctx: &egui::Context, generation: u64) -> Result<(), String> {
        if self.job.is_some() || self.mutation.is_some() {
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
                        let writable = pcbcraft_storage::library::writable(&path);
                        pcbcraft_storage::library::load(&path).map(|source| (Arc::new(source), Some(locator), writable))
                    })
                    .transpose();
                let _ = send.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        let previous_status = self.statuses.insert(source.clone(), "Loading…".into());
        self.job = Some(Job { source, previous_status, receive, generation, installing: false });
        Ok(())
    }

    /// Bundle input uses the same decoder on desktop and browser; browser selection is initiated
    /// synchronously during the click to retain its user-activation permission. Cancellation
    /// restores the source's previous status without changing its catalog or registration.
    pub(super) fn add_bundle(
        &mut self,
        source: String,
        ctx: &egui::Context,
        catalog: pcbcraft_engine::library::sources::Catalog,
    ) -> Result<(), String> {
        if self.job.is_some() || self.mutation.is_some() {
            return Err("A library read is already running".into());
        }
        let generation = catalog.generation();
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
                        let parsed = NativeSource::from_bundle(&bytes)?;
                        if catalog.contains_source(&parsed) {
                            return Err("Every included revision is already installed with identical content".into());
                        }
                        let Some(destination) =
                            rfd::FileDialog::new().set_title("Install library into a new folder").set_file_name("library").save_file()
                        else {
                            return Ok(None);
                        };
                        let locator = destination.to_str().ok_or("Library folder path is not valid UTF-8")?.to_owned();
                        pcbcraft_storage::library::install_bundle(&bytes, &destination).map(|source| Some((Arc::new(source), Some(locator), true)))
                    })
                    .transpose();
                let _ = send.send(result.map(Option::flatten));
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
        let previous_status = self.statuses.insert(source.clone(), "Loading…".into());
        self.job = Some(Job { source, previous_status, receive, generation, installing: cfg!(not(target_arch = "wasm32")) });
        Ok(())
    }

    /// A request slot spans browser selection and reads; removing it invalidates completion.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn pick_folder(&mut self, source: String, ctx: &egui::Context, generation: u64) -> Result<(), String> {
        if self.job.is_some() || self.mutation.is_some() {
            return Err("A library read is already running".into());
        }
        let (send, receive) = mpsc::channel();
        let promise = web::pick_folder();
        let context = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = send.send(web::read_folder(promise).await);
            context.request_repaint();
        });
        let previous_status = self.statuses.insert(source.clone(), "Loading…".into());
        self.job = Some(Job { source, previous_status, receive, generation, installing: false });
        Ok(())
    }

    /// Source reads are bounded to one worker across add, reload and relocation.
    /// Installing holds source mutation authority until validation/publication completes.
    pub(super) fn installing(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.installing)
    }

    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Remove registration immediately but retain an in-flight slot; late results cannot restore it.
    pub(super) fn remove(&mut self, source: &str, session: &mut pcbcraft_engine::Session) {
        self.registered.retain(|r| r.source != source);
        self.pending.retain(|r| r.source != source);
        self.statuses.remove(source);
        let _ = session.execute(sources::REMOVE, &json!({"source":source}));
    }

    /// Consume a worker result only while its registration request is still current. Failed
    /// refreshes remove stale catalog content without touching pinned project copies or history.
    /// Cancelled pickers restore their prior status only while the source request remains current.
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
                            Ok(Some((source, path, writable))) => match session.install_library_source(&job.source, source) {
                                Ok(()) => {
                                    self.statuses.insert(job.source.clone(), "Available".into());
                                    let data = session.libraries().inspect();
                                    let name = data
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .find(|s| s["source"] == job.source)
                                        .and_then(|s| s["name"].as_str())
                                        .unwrap_or(&job.source)
                                        .to_owned();
                                    if let Some(path) = path {
                                        self.registered.retain(|r| r.source != job.source);
                                        self.registered.push(Registration { source: job.source.clone(), path, name: name.clone(), writable });
                                    }
                                    #[cfg(target_arch = "wasm32")]
                                    {
                                        self.registered.retain(|r| r.source != job.source);
                                        self.registered.push(Registration { source: job.source, path: String::new(), name, writable: false });
                                    }
                                }
                                Err(error) => {
                                    session.mark_library_unavailable(&job.source);
                                    self.statuses.insert(job.source, error.to_string());
                                }
                            },
                            Ok(None) => {
                                if let Some(previous) = job.previous_status {
                                    self.statuses.insert(job.source, previous);
                                } else {
                                    self.statuses.remove(&job.source);
                                }
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
        if self.job.is_none() && self.mutation.is_none() && !self.pending.is_empty() {
            let registration = self.pending.remove(0);
            if let Err(error) = self.load_path(registration.source.clone(), registration.path, ctx, session.libraries().generation()) {
                self.statuses.insert(registration.source, error);
            }
        }
    }
}

impl PcbApp {
    /// Commit captured symbol-only or native-part data through the appropriate shared engine command.
    pub(super) fn place_native_template(&mut self, placement: &crate::parts_browser::Placement, mut params: serde_json::Value) {
        if let Some(part) = &placement.part {
            let definition = &part.resolved.definition;
            params["library"] = json!(definition.library);
            params["definition"] = json!(definition.id);
            params["definition_revision"] = json!(definition.revision);
            params["definition_digest"] = json!(sources::part_digest(&part.resolved));
            params["package"] = json!(part.package);
            params["requirements"] = json!(part.requirements);
            if let Some(manufacturer) = &part.manufacturer {
                params["manufacturer"] = json!(manufacturer.reference());
                params["manufacturer_digest"] = json!(sources::manufacturer_digest(manufacturer));
            }
            self.run(pcbcraft_engine::part_instances::PLACE_PART, &params);
        } else {
            self.place_catalog_symbol(&placement.definition, params);
        }
    }
    /// Bind symbol placement to the exact previewed content, including across source refresh/removal.
    pub(super) fn place_catalog_symbol(&mut self, definition: &pcbcraft_engine::schematic::SymbolDefinition, mut params: serde_json::Value) {
        params["definition"] = json!(definition.id);
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
        self.poll_library_mutation(ctx);
        let generation = self.session.libraries().generation();
        self.library_sources.poll(&mut self.session, ctx);
        #[cfg(target_arch = "wasm32")]
        if let Err(error) = self.library_sources.persist_browser() {
            self.message = Some(error);
        }
        if self.session.libraries().generation() != generation {
            self.repaint_editors(ctx);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(inline_js = r#"
    function chooseLibrary(folder) {
      return new Promise((resolve,reject)=>{
        const input=document.createElement('input'); input.type='file';
        if(folder){input.webkitdirectory=true;input.multiple=true;}else{input.accept='.json';}
        input.oncancel=()=>resolve(null);
        input.onchange=async()=>{try{
          const files=input.files||[];if(!files.length)return resolve(null);
          if(!folder){if(files[0].size>16777216)throw Error('Library bundle exceeds 16 MiB');return resolve(await files[0].arrayBuffer());}
          const documents={};const problems=[];let total=0,count=0;
          for(const file of files){
            const pieces=file.webkitRelativePath.split('/');pieces.shift();const path=pieces.join('/');
            // Root notices are not native documents; preserve the native folder adapter's scope.
            if(path!=='library.json'&&!/^(symbols|footprints|categories|parts|manufacturer-parts)\//.test(path))continue;
            if(++count>256)throw Error('Library source exceeds 256 files');
            if(path!=='library.json'&&!/^(symbols|footprints|categories|parts|manufacturer-parts)\/[^/]+\.json$/.test(path)){problems.push(path+': unsupported native item path');continue;}
            if(file.size>1048576){if(path==='library.json')throw Error('Manifest exceeds 1 MiB');problems.push(path+': file exceeds 1 MiB');continue;}
            if((total+=file.size)>8388608)throw Error('Library source exceeds 8 MiB');
            if(Object.hasOwn(documents,path))throw Error('Duplicate library path');
            try{documents[path]=new TextDecoder('utf-8',{fatal:true}).decode(await file.arrayBuffer());}catch(error){if(path==='library.json')throw error;problems.push(path+': could not read UTF-8 document');}
          }
          resolve({documents,problems});
        }catch(error){reject(error)}};input.click();
      });
    }
    export function rememberLibrarySources(key,text){localStorage.setItem(key,text)}
    export function pickLibraryBundle(){return chooseLibrary(false)}
    export function pickLibraryFolder(){return chooseLibrary(true)}
    "#)]
    extern "C" {
        #[wasm_bindgen(catch,js_name=rememberLibrarySources)]
        pub(super) fn remember_sources(key: &str, text: &str) -> Result<(), JsValue>;
        #[wasm_bindgen(js_name=pickLibraryBundle)]
        fn pick_bundle() -> js_sys::Promise;
        #[wasm_bindgen(js_name=pickLibraryFolder)]
        pub(super) fn pick_folder() -> js_sys::Promise;
    }
    /// Start selection synchronously in the triggering click's browser activation.
    pub(super) fn pick() -> js_sys::Promise {
        pick_bundle()
    }
    /// Validate transported folder text with exactly the engine's native document limits.
    pub(super) async fn read_folder(promise: js_sys::Promise) -> LoadResult {
        let value = wasm_bindgen_futures::JsFuture::from(promise).await.map_err(|e| format!("Could not open library: {e:?}"))?;
        if value.is_null() {
            return Ok(None);
        }
        let text =
            js_sys::JSON::stringify(&value).map_err(|e| format!("Could not read library: {e:?}"))?.as_string().ok_or("Invalid library documents")?;
        #[derive(Deserialize)]
        struct Folder {
            documents: sources::Documents,
            problems: Vec<String>,
        }
        let folder: Folder = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut source = NativeSource::load(&folder.documents)?;
        source.add_adapter_problems(folder.problems);
        Ok(Some((Arc::new(source), None, false)))
    }
    /// Bound picked bytes before copying into Wasm memory and use the native snapshot validator.
    pub(super) async fn read(promise: js_sys::Promise) -> LoadResult {
        let value = wasm_bindgen_futures::JsFuture::from(promise).await.map_err(|e| format!("Could not open library: {e:?}"))?;
        if value.is_null() {
            return Ok(None);
        }
        let bytes = js_sys::Uint8Array::new(&value);
        if bytes.length() as usize > sources::MAX_SOURCE_BYTES * 2 {
            return Err("Library bundle exceeds 16 MiB".into());
        }
        NativeSource::from_bundle(&bytes.to_vec()).map(|source| Some((Arc::new(source), None, false)))
    }
}

#[cfg(test)]
mod tests;
