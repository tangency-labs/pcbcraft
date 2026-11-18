//! File job lifecycle shared by desktop and browser.

use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

use inspection::Finished;
#[cfg(target_arch = "wasm32")]
use pcbcraft_engine::persistence::Encoding;
use pcbcraft_engine::{Project, Session};

/// Desktop working folders and portable archives use the same split documents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SaveFormat {
    #[default]
    Folder,
    Archive,
}

mod abandon;
#[cfg(any(target_arch = "wasm32", test))]
mod download;
mod inspection;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod stand_in;
#[cfg(target_arch = "wasm32")]
mod web;

/// Suggest a file name, without extension, for a project's first desktop save or a browser
/// download. Characters that can't appear in a file name on common desktops become `-`; a name
/// with nothing left suggests `project`.
fn file_name_for(name: &str) -> String {
    let file: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c }).collect();
    let file = file.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if file.is_empty() { "project".to_owned() } else { file.to_owned() }
}

/// The name a browser download of the project `name` is offered under: the desktop suggestion
/// from [`file_name_for`] with the archive's `.pc` extension (#164).
#[cfg(any(target_arch = "wasm32", test))]
fn download_file_name(name: &str) -> String {
    format!("{}.pc", file_name_for(name))
}

/// Turn the browser's leave-page warning on or off.
#[cfg(target_arch = "wasm32")]
pub(super) fn warn_before_leaving(unsaved: bool) {
    web::set_unsaved(unsaved);
}

#[derive(Debug, Default)]
pub(super) struct ProjectFiles {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) recent: crate::recent_projects::RecentProjects,
    saved: Option<Saved>,
    job: Option<Job>,
    /// How the last file operation ended, for editor inspection.
    last: Option<Finished>,
    /// File jobs finished so far. Jobs run one at a time, so the running one is `completed + 1`
    /// and `last` is `completed`: inspection reports these as job IDs.
    completed: u64,
    /// Workers of abandoned opens that may still be running.
    stalled: abandon::Stalled,
    /// In tests, saves wait here for the test to finish them instead of opening a native picker.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    stand_in: Option<StandIn>,
}

/// A save waiting for a test to say how the person answered the picker.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[derive(Debug, Default)]
pub(super) struct StandIn {
    pending: Option<StandInSave>,
}

/// What a real save worker would have been given, and where to send its result.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[derive(Debug)]
struct StandInSave {
    snapshot: Arc<Project>,
    previous: Option<(Arc<Project>, native::Target)>,
    format: Option<SaveFormat>,
    send: mpsc::Sender<Result<Outcome, String>>,
}

#[derive(Debug)]
struct Saved {
    snapshot: Arc<Project>,
    /// Worker-prepared opening upgrade; snapshot remains the original on-disk save baseline.
    opened: Option<Arc<Project>>,
    /// What preparing `opened` changed, for the open notice.
    upgrades: Upgrades,
    #[cfg(not(target_arch = "wasm32"))]
    target: native::Target,
    #[cfg(target_arch = "wasm32")]
    bytes: Arc<[u8]>,
    #[cfg(target_arch = "wasm32")]
    encoding: Encoding,
    #[cfg(target_arch = "wasm32")]
    needs_manifest: bool,
    #[cfg(target_arch = "wasm32")]
    documents: Option<Arc<pcbcraft_engine::persistence::Documents>>,
}

impl Saved {
    /// Prepare an in-memory migration after full validation; failed work is never adopted.
    /// Keeping the original snapshot makes the next Save encode changed references and preserves
    /// raw-byte comparison for unrelated documents and external-change detection.
    fn prepare_open(mut self) -> Result<Self, String> {
        let migrated = pcbcraft_engine::persistence::migrate_bundled_libraries(self.snapshot.clone())?;
        let opened = pcbcraft_engine::persistence::adopt_paper(migrated.clone());
        self.upgrades = Upgrades { libraries: !Arc::ptr_eq(&migrated, &self.snapshot), paper: !Arc::ptr_eq(&opened, &migrated) };
        self.opened = (!Arc::ptr_eq(&opened, &self.snapshot)).then_some(opened);
        Ok(self)
    }
}

/// Changes prepared in memory when a project opens, written by the next Save or Download.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Upgrades {
    /// Bundled library references moved to their current addresses.
    libraries: bool,
    /// Sheets from before paper got paper around their drawing.
    paper: bool,
}

impl Upgrades {
    /// The open notice for these changes, or `None` when nothing changed.
    fn notice(self) -> Option<String> {
        let save = if crate::IN_BROWSER { "Download" } else { "Save" };
        let what = match (self.libraries, self.paper) {
            (false, false) => return None,
            (true, false) => "migrate bundled library references",
            (false, true) => "add paper to older sheets",
            (true, true) => "migrate bundled library references and add paper to older sheets",
        };
        Some(format!("Project opened · {save} will {what}"))
    }
}

#[derive(Debug)]
struct Job {
    receiver: Receiver<Result<Outcome, String>>,
    revision: u64,
    /// `open` or `save`, for editor inspection.
    operation: &'static str,
    /// Where the job reads or writes, when known before it runs (a picker chooses it later).
    destination: Option<String>,
    #[cfg(target_arch = "wasm32")]
    download: Option<web::Download>,
}

enum Outcome {
    Cancelled,
    Opened(Saved),
    /// A completed desktop save, with the snapshot it started from when the save also named an
    /// Untitled project. The saved snapshot is then that one's renamed copy.
    #[cfg(not(target_arch = "wasm32"))]
    Saved(Saved, Option<Arc<Project>>),
    #[cfg(target_arch = "wasm32")]
    Downloaded,
}

pub(super) enum Update {
    Opened,
    Message(String),
    Error(String),
}

impl ProjectFiles {
    /// Supply a local-copy discriminator only after a project has a successfully adopted file.
    pub(super) fn arrangement_location(&self, session: &Session) -> Option<String> {
        let saved = self.saved.as_ref().filter(|saved| session.project().is_some_and(|project| saved.snapshot.same_document(project)))?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            Some(format!("desktop:{}", saved.target.arrangement_location()))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = saved;
            Some("browser".into())
        }
    }

    /// Restore desktop history from app storage without reading any project files at startup.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn with_recent(text: Option<&str>) -> Self {
        Self { recent: text.map(crate::recent_projects::RecentProjects::parse).unwrap_or_default(), ..Self::default() }
    }

    /// Reopen a recent path through the ordinary native worker and revision check. No picker
    /// or disk access runs on the editor thread; an existing job retains its receiver.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn open_path(&mut self, path: std::path::PathBuf, revision: u64, ctx: &egui::Context) -> Result<(), String> {
        if self.busy() {
            return Err("A project file operation is already running".to_owned());
        }
        let (send, receiver) = mpsc::channel();
        let destination = Some(path.display().to_string());
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("pcbcraft-open-recent".to_owned())
            .spawn(move || {
                let result = native::load(&path).map(Outcome::Opened).map_err(|error| format!("Could not open {}: {error}", path.display()));
                let _ = send.send(result);
                ctx.request_repaint();
            })
            .map_err(|error| format!("Could not start opening the project: {error}"))?;
        self.job = Some(Job { receiver, revision, operation: "open", destination });
        Ok(())
    }

    /// Whether a file operation is outstanding. Edits may continue during a save.
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Whether replacing the current project would discard edits since its last known saved snapshot.
    pub(super) fn dirty(&self, session: &Session) -> bool {
        session.snapshot().is_some_and(|project| {
            !self.saved.as_ref().is_some_and(|saved| {
                Arc::ptr_eq(&saved.snapshot, &project) || saved.opened.as_ref().is_some_and(|opened| Arc::ptr_eq(opened, &project))
            })
        })
    }

    /// Whether a plain Save of the open project would write in place, without asking where: the
    /// project was opened from or saved to a current-format location. Untitled projects need
    /// the person to choose one.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn has_location(&self, session: &Session) -> bool {
        session.snapshot().is_some_and(|project| self.saved.as_ref().is_some_and(|saved| saved.snapshot.same_document(&project)))
    }

    /// Start opening a file. The completed result is ignored if the editor changes in the meantime.
    pub(super) fn open(&mut self, revision: u64, ctx: &egui::Context, folder: bool) -> Result<(), String> {
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("pcbcraft-open".to_owned())
            .spawn(move || {
                let _ = send.send(native::open(folder));
                ctx.request_repaint();
            })
            .map_err(|error| format!("Could not start opening the project: {error}"))?;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = folder;
            let file = web::pick_file();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = send.send(web::open(file).await);
                ctx.request_repaint();
            });
        }
        self.job = Some(Job {
            receiver: receive,
            revision,
            operation: "open",
            destination: None,
            #[cfg(target_arch = "wasm32")]
            download: None,
        });
        Ok(())
    }

    /// Capture an immutable snapshot. An explicit encoding starts a Save As or browser download.
    /// Browser exports of migrated files encode the current schema instead of reusing old bytes.
    pub(super) fn save(&mut self, session: &Session, encoding: Option<SaveFormat>, ctx: &egui::Context) -> Result<(), String> {
        let snapshot = session.snapshot().ok_or("no project open")?;
        let (send, receive) = mpsc::channel();
        #[cfg(target_arch = "wasm32")]
        let destination = None;
        #[cfg(not(target_arch = "wasm32"))]
        let previous = self.previous(&snapshot);
        #[cfg(not(target_arch = "wasm32"))]
        let destination = previous.as_ref().filter(|_| encoding.is_none()).map(|(_, target)| target.path().display().to_string());
        #[cfg(not(target_arch = "wasm32"))]
        {
            #[cfg(test)]
            let send = match &mut self.stand_in {
                Some(stand_in) => {
                    stand_in.pending = Some(StandInSave { snapshot: Arc::clone(&snapshot), previous: previous.clone(), format: encoding, send });
                    None
                }
                None => Some(send),
            };
            #[cfg(not(test))]
            let send = Some(send);
            if let Some(send) = send {
                let ctx = ctx.clone();
                std::thread::Builder::new()
                    .name("pcbcraft-save".to_owned())
                    .spawn(move || {
                        let _ = send.send(native::save(snapshot, previous, encoding));
                        ctx.request_repaint();
                    })
                    .map_err(|error| format!("Could not start saving the project: {error}"))?;
            }
        }
        #[cfg(target_arch = "wasm32")]
        let download = {
            let _ = encoding;
            let encoding = Encoding::Zip;
            let unchanged = self
                .saved
                .as_ref()
                .filter(|saved| saved.encoding == encoding && Arc::ptr_eq(&saved.snapshot, &snapshot) && !saved.needs_manifest)
                .map(|saved| Arc::clone(&saved.bytes));
            let original =
                self.saved.as_ref().and_then(|saved| download::Original::for_project(saved.snapshot.clone(), saved.documents.clone()?, &snapshot));
            web::Download::new(snapshot, unchanged, original, send)?
        };
        self.job = Some(Job {
            receiver: receive,
            revision: session.revision(),
            operation: "save",
            destination,
            #[cfg(target_arch = "wasm32")]
            download: Some(download),
        });
        ctx.request_repaint();
        Ok(())
    }

    /// The saved baseline and destination of the open project, when it has one.
    #[cfg(not(target_arch = "wasm32"))]
    fn previous(&self, snapshot: &Arc<Project>) -> Option<(Arc<Project>, native::Target)> {
        self.saved.as_ref().filter(|saved| saved.snapshot.same_document(snapshot)).map(|saved| (Arc::clone(&saved.snapshot), saved.target.clone()))
    }

    /// Save As to `path` without a picker (#409). The worker, destination checks and completion
    /// are those of a picked Save As: the destination must not exist yet (an archive name gains
    /// `.pc`), an Untitled project takes its name, and only a successful save is adopted.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn save_to(&mut self, session: &Session, format: SaveFormat, path: std::path::PathBuf, ctx: &egui::Context) -> Result<(), String> {
        if self.busy() {
            return Err("A project file operation is already running".to_owned());
        }
        let snapshot = session.snapshot().ok_or("no project open")?;
        let previous = self.previous(&snapshot);
        let path = native::with_extension(format, path)?;
        let destination = Some(path.display().to_string());
        let (send, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("pcbcraft-save-as".to_owned())
            .spawn(move || {
                let _ = send.send(native::save_new(snapshot, previous.as_ref(), format, path));
                ctx.request_repaint();
            })
            .map_err(|error| format!("Could not start saving the project: {error}"))?;
        self.job = Some(Job { receiver, revision: session.revision(), operation: "save", destination });
        Ok(())
    }

    /// Describe migration or manifest generation where it affects the next save.
    pub(super) fn open_notice(&self) -> String {
        if let Some(notice) = self.saved.as_ref().filter(|saved| saved.opened.is_some()).and_then(|saved| saved.upgrades.notice()) {
            return notice;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.saved.as_ref().map_or_else(|| "Project opened".to_owned(), |saved| saved.target.open_notice())
        }
        #[cfg(target_arch = "wasm32")]
        {
            match &self.saved {
                Some(saved) if saved.needs_manifest => "Project opened · Download will upgrade the project package".to_owned(),
                _ => "Project opened".to_owned(),
            }
        }
    }

    /// Accept completed jobs on the editor thread. Keep edits made after the saved revision.
    /// Record desktop history only for accepted opens and successful saves.
    /// A save that named an Untitled project renames it in the session too, history included,
    /// without changing the revision; cancelled and failed saves leave the session as it was.
    pub(super) fn poll(&mut self, session: &mut Session, ctx: &egui::Context) -> Option<Update> {
        let job = self.job.as_mut()?;
        #[cfg(target_arch = "wasm32")]
        if let Some(download) = &mut job.download {
            download.advance();
        }
        let revision = job.revision;
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(16));
                return None;
            }
            Err(TryRecvError::Disconnected) => Err("file operation ended unexpectedly".to_owned()),
        };
        let Job { operation, destination, .. } = self.job.take()?;
        let mut finished = Finished { operation, status: "succeeded", path: destination, error: None };
        let update = match result {
            Ok(Outcome::Cancelled) => {
                finished.status = "cancelled";
                None
            }
            Ok(Outcome::Opened(saved)) if session.revision() == revision => {
                let opened = saved.opened.as_ref().unwrap_or(&saved.snapshot);
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.recent.remember_project(Arc::clone(opened), saved.target.path().to_owned());
                    finished.path = Some(saved.target.path().display().to_string());
                }
                session.open_snapshot(Arc::clone(opened));
                self.saved = Some(saved);
                Some(Update::Opened)
            }
            Ok(Outcome::Opened(_)) => {
                Some(Update::Error("Project changed while opening the file. Open it again to replace the current project.".to_owned()))
            }
            #[cfg(not(target_arch = "wasm32"))]
            Ok(Outcome::Saved(saved, renamed)) => {
                if let Some(original) = renamed {
                    session.adopt_saved_metadata(&original, &saved.snapshot);
                }
                self.recent.remember_project(Arc::clone(&saved.snapshot), saved.target.path().to_owned());
                finished.path = Some(saved.target.path().display().to_string());
                let current = session.snapshot().is_some_and(|project| Arc::ptr_eq(&project, &saved.snapshot));
                let added_manifest = saved.target.added_manifest();
                self.saved = Some(saved);
                let text = if !current {
                    "Earlier version saved. Current edits are unsaved."
                } else if added_manifest {
                    "Project saved with a verified manifest"
                } else {
                    "Project saved"
                };
                Some(Update::Message(text.to_owned()))
            }
            #[cfg(target_arch = "wasm32")]
            Ok(Outcome::Downloaded) => Some(Update::Message("Project download requested".to_owned())),
            Err(error) => Some(Update::Error(error)),
        };
        if let Some(Update::Error(error)) = &update {
            finished.status = "failed";
            finished.error = Some(error.clone());
        }
        self.completed += 1;
        self.last = Some(finished);
        update
    }
}

#[cfg(test)]
mod name_tests {
    use super::download_file_name;

    /// Browser downloads are named after the project, as a desktop first save suggests, instead
    /// of a fixed `project.pc` (#164).
    #[test]
    fn browser_downloads_are_named_after_the_project() {
        assert_eq!(download_file_name("Dual LED demo"), "Dual LED demo.pc");
        assert_eq!(download_file_name("Power: 5V/3V3"), "Power- 5V-3V3.pc");
        assert_eq!(download_file_name(" .. "), "project.pc");
    }
}
