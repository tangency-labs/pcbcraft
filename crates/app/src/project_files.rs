//! File job lifecycle shared by desktop and browser.

use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

use pcbcraft_engine::{Project, Session};

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
    pub(super) fn open(&mut self, revision: u64, ctx: &egui::Context) -> Result<(), String> {
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("pcbcraft-open".to_owned())
            .spawn(move || {
                let _ = send.send(native::open());
                ctx.request_repaint();
            })
            .map_err(|error| format!("Could not start opening the project: {error}"))?;
        #[cfg(target_arch = "wasm32")]
        {
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

    /// Capture an immutable snapshot and start saving it without blocking editor commands.
    pub(super) fn save(&mut self, session: &Session, save_as: bool, ctx: &egui::Context) -> Result<(), String> {
        let snapshot = session.snapshot().ok_or("no project open")?;
        let (send, receive) = mpsc::channel();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let previous = self
                .saved
                .as_ref()
                .filter(|saved| !save_as && saved.snapshot.same_document(&snapshot))
                .map(|saved| (Arc::clone(&saved.snapshot), saved.target.clone()));
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("pcbcraft-save".to_owned())
                .spawn(move || {
                    let _ = send.send(native::save(snapshot, previous));
                    ctx.request_repaint();
                })
                .map_err(|error| format!("Could not start saving the project: {error}"))?;
        }
        #[cfg(target_arch = "wasm32")]
        let download = {
            let _ = save_as;
            let unchanged = self.saved.as_ref().filter(|saved| Arc::ptr_eq(&saved.snapshot, &snapshot)).map(|saved| Arc::clone(&saved.bytes));
            web::Download::new(snapshot, unchanged, send)?
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
                self.saved = Some(saved);
                let text = if current { "Project saved" } else { "Earlier version saved. Current edits are unsaved." };
                Some(Update::Message(text.to_owned()))
            }
            #[cfg(target_arch = "wasm32")]
            Ok(Outcome::Downloaded) => Some(Update::Message("Project download requested".to_owned())),
            Err(error) => Some(Update::Error(error)),
        }
    }
}
