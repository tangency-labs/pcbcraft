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

#[derive(Debug, Default)]
pub(super) struct ProjectFiles {
    saved: Option<Saved>,
    job: Option<Job>,
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
    #[cfg(not(target_arch = "wasm32"))]
    Saved(Saved),
    #[cfg(target_arch = "wasm32")]
    Downloaded,
}

pub(super) enum Update {
    Opened,
    Message(String),
    Error(String),
}

impl ProjectFiles {
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
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("pcbcraft-save".to_owned())
                .spawn(move || {
                    let _ = send.send(native::save(snapshot, previous, encoding));
                    ctx.request_repaint();
                })
                .map_err(|error| format!("Could not start saving the project: {error}"))?;
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
            Ok(Outcome::Saved(saved)) => {
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
