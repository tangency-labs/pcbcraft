//! File job lifecycle shared by desktop and browser.

use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

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

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

/// Turn the browser's leave-page warning on or off.
#[cfg(target_arch = "wasm32")]
pub(super) fn warn_before_leaving(unsaved: bool) {
    web::set_unsaved(unsaved);
}

#[derive(Debug, Default)]
pub(super) struct ProjectFiles {
    saved: Option<Saved>,
    job: Option<Job>,
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
    #[cfg(not(target_arch = "wasm32"))]
    target: native::Target,
    #[cfg(target_arch = "wasm32")]
    bytes: Arc<[u8]>,
    #[cfg(target_arch = "wasm32")]
    encoding: Encoding,
    #[cfg(target_arch = "wasm32")]
    needs_manifest: bool,
    #[cfg(target_arch = "wasm32")]
    legacy: bool,
    #[cfg(target_arch = "wasm32")]
    documents: Option<Arc<pcbcraft_engine::persistence::Documents>>,
}

#[derive(Debug)]
struct Job {
    receiver: Receiver<Result<Outcome, String>>,
    revision: u64,
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

    /// Whether a file operation is outstanding. Edits may continue during a save.
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Whether replacing the current project would discard edits since its last known saved snapshot.
    pub(super) fn dirty(&self, session: &Session) -> bool {
        session.snapshot().is_some_and(|project| !self.saved.as_ref().is_some_and(|saved| Arc::ptr_eq(&saved.snapshot, &project)))
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
        #[cfg(not(target_arch = "wasm32"))]
        {
            let previous = self
                .saved
                .as_ref()
                .filter(|saved| saved.snapshot.same_document(&snapshot))
                .map(|saved| (Arc::clone(&saved.snapshot), saved.target.clone()));
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
                .filter(|saved| saved.encoding == encoding && Arc::ptr_eq(&saved.snapshot, &snapshot) && !saved.legacy && !saved.needs_manifest)
                .map(|saved| Arc::clone(&saved.bytes));
            let documents = self.saved.as_ref().filter(|saved| Arc::ptr_eq(&saved.snapshot, &snapshot)).and_then(|saved| saved.documents.clone());
            web::Download::new(snapshot, unchanged, documents, send)?
        };
        self.job = Some(Job {
            receiver: receive,
            revision: session.revision(),
            #[cfg(target_arch = "wasm32")]
            download: Some(download),
        });
        ctx.request_repaint();
        Ok(())
    }

    /// Describe migration or manifest generation where it affects the next save.
    pub(super) fn open_notice(&self) -> String {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.saved.as_ref().map_or_else(|| "Project opened".to_owned(), |saved| saved.target.open_notice())
        }
        #[cfg(target_arch = "wasm32")]
        {
            match &self.saved {
                Some(saved) if saved.needs_manifest => "Project opened without a manifest · Download will include one".to_owned(),
                Some(saved) if saved.legacy => "Project opened · Download will use the split .pc.zip format".to_owned(),
                _ => "Project opened".to_owned(),
            }
        }
    }

    /// Accept completed jobs on the editor thread. Keep edits made after the saved revision.
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
        self.job = None;
        match result {
            Ok(Outcome::Cancelled) => None,
            Ok(Outcome::Opened(saved)) if session.revision() == revision => {
                session.open_snapshot(Arc::clone(&saved.snapshot));
                self.saved = Some(saved);
                Some(Update::Opened)
            }
            Ok(Outcome::Opened(_)) => {
                Some(Update::Error("Project changed while opening the file. Open it again to replace the current project.".to_owned()))
            }
            #[cfg(not(target_arch = "wasm32"))]
            Ok(Outcome::Saved(saved, renamed)) => {
                if let Some(original) = renamed {
                    session.adopt_saved_name(&original, &saved.snapshot);
                }
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
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
impl ProjectFiles {
    /// Send later saves to a stand-in the test finishes, instead of a worker with a native picker.
    pub(super) fn use_stand_in(&mut self) {
        self.stand_in = Some(StandIn::default());
    }

    /// Deliver the real loader's result through the ordinary completion path without an OS picker.
    pub(super) fn open_path_for_test(&mut self, path: &std::path::Path, revision: u64) {
        let (send, receiver) = mpsc::channel();
        let _ = send.send(native::load(path).map(Outcome::Opened));
        self.job = Some(Job { receiver, revision });
    }

    /// Whether a save is waiting for the test to finish it.
    pub(super) fn stand_in_waiting(&self) -> bool {
        self.stand_in.as_ref().is_some_and(|stand_in| stand_in.pending.is_some())
    }

    /// Finish the waiting save as the worker would: in place when it has a destination, otherwise
    /// as though the person picked `path` as a new project folder.
    pub(super) fn finish_stand_in(&mut self, path: std::path::PathBuf) {
        let Some(StandInSave { snapshot, previous, format, send }) = self.stand_in.as_mut().and_then(|stand_in| stand_in.pending.take()) else {
            return;
        };
        let result = match previous {
            Some(previous) if format.is_none() && !previous.1.legacy() => native::save(snapshot, Some(previous), None),
            previous => native::save_new(snapshot, previous.as_ref(), format.unwrap_or_default(), path),
        };
        let _ = send.send(result);
    }

    /// End the waiting save as a dismissed picker (`None`) or a failure with that message.
    pub(super) fn end_stand_in(&mut self, error: Option<&str>) {
        let Some(StandInSave { send, .. }) = self.stand_in.as_mut().and_then(|stand_in| stand_in.pending.take()) else { return };
        let _ = send.send(error.map_or(Ok(Outcome::Cancelled), |error| Err(error.to_owned())));
    }
}
