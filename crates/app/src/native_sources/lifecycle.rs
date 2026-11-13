//! Confirmed destructive source operations run outside the editor thread.
use super::*;
use crate::Dialog;
use pcbcraft_chrome::{controls, shell};
use serde_json::Value;

/// A confirmation captures the actual directory contents, source generation and registration.
#[derive(Clone, Debug)]
pub(crate) struct DeleteDraft {
    source: String,
    path: String,
    library: String,
    generation: u64,
    fingerprints: Vec<Value>,
}
/// One bounded operation slot covers inspection and trash, including disconnected workers.
#[derive(Debug)]
pub(super) struct Job {
    receive: Receiver<Result<Outcome, String>>,
}
#[derive(Debug)]
enum Outcome {
    #[cfg(not(target_arch = "wasm32"))]
    Prepared(DeleteDraft),
    #[cfg(not(target_arch = "wasm32"))]
    Deleted { source: String, path: String },
}

impl PcbApp {
    /// Source writes/deletion never overlap a source reader or another destructive operation.
    pub(crate) fn library_mutation_busy(&self) -> bool {
        self.library_sources.mutation.is_some()
    }
    /// Command and ribbon-caption acquisition choices use the standard menu surface.
    /// Pointer position is captured once so the menu remains anchored while it is open.
    pub(crate) fn add_library_dialog(&mut self, ctx: &egui::Context, source: Option<String>) {
        let id = egui::Id::unique("add_library_choices");
        let pointer = ctx.pointer_latest_pos().unwrap_or(ctx.content_rect().center());
        let at = ctx.data_mut(|data| *data.get_temp_mut_or_insert_with(id, || pointer));
        let mut open = true;
        let mut choice = None;
        egui::Popup::new(id, ctx.clone(), at, egui::LayerId::background())
            .kind(egui::PopupKind::Menu)
            .frame(pcbcraft_chrome::menu::frame())
            .open_bool(&mut open)
            .show(|ui| {
                let (folder, bundle) = pcbcraft_chrome::library_workspace::add_choices(ui);
                if folder || bundle {
                    choice = Some(folder);
                }
            });
        if let Some(folder) = choice {
            self.message = self.acquire_library(ctx, source, folder).err();
        } else if open {
            self.dialog = Dialog::AddLibrary(source);
            return;
        }
        ctx.data_mut(|data| data.remove::<egui::Pos2>(id));
    }
    /// Folder and bundle choices share availability validation before launching a picker.
    pub(crate) fn acquire_library(&mut self, ctx: &egui::Context, source: Option<String>, folder: bool) -> Result<(), String> {
        if self.library_sources.busy() || self.library_mutation_busy() || (source.is_none() && !self.library_sources.can_add(&self.session)) {
            return Err("Finish the current library operation before adding another library".into());
        }
        let source = source.unwrap_or_else(|| self.library_sources.next_source(&self.session));
        if folder {
            self.library_sources.pick_folder(source, ctx, self.session.libraries().generation())
        } else {
            self.library_sources.add_bundle(source, ctx, self.session.libraries().clone())
        }
    }
    /// Inspect the writable folder on a worker before naming its path/count in confirmation.
    pub(crate) fn begin_library_delete(&mut self, ctx: &egui::Context, source: Option<&str>) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, source);
            Err("Library deletion is unavailable in the browser".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let source = source.ok_or("Choose a library first")?;
            if self.library_sources.busy() || self.library_mutation_busy() {
                return Err("A library operation is already running".into());
            }
            let data = self.session.libraries().inspect();
            let data = data.as_array().into_iter().flatten().find(|s| s["source"] == source).ok_or("Library is unavailable")?;
            if data["builtin"] == true {
                return Err("Built-in libraries cannot be deleted".into());
            }
            let library = data["library"].as_str().ok_or("Library identity is unavailable")?.to_owned();
            if self.library_editors.unpublished_in(&library) {
                return Err("Publish or discard this library's editor changes before deleting it".into());
            }
            let registration =
                self.library_sources.registered.iter().find(|r| r.source == source).ok_or("Only registered folder libraries can be deleted")?.clone();
            let generation = self.session.libraries().generation();
            let (send, receive) = mpsc::channel();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("pcbcraft-library-delete-check".into())
                .spawn(move || {
                    let result = pcbcraft_storage::library::deletion_snapshot(std::path::Path::new(&registration.path)).map(|snapshot| {
                        Outcome::Prepared(DeleteDraft {
                            source: registration.source,
                            path: registration.path,
                            library,
                            generation,
                            fingerprints: snapshot.items().iter().map(|i| i.inspect()).collect(),
                        })
                    });
                    let _ = send.send(result);
                    ctx.request_repaint();
                })
                .map_err(|e| e.to_string())?;
            self.library_sources.mutation = Some(Job { receive });
            Ok(())
        }
    }
    /// Revalidate registration/catalog/drafts at confirmation; folder content is checked again
    /// on the worker before invoking system trash. Cancellation makes no source or project edit.
    pub(crate) fn delete_library_dialog(&mut self, ctx: &egui::Context, draft: DeleteDraft) {
        let mut submit = false;
        let mut cancel = false;
        let closing = shell::dialog(ctx, "delete_library", "Delete library", |ui| {
            ui.label(format!(
                "Move {} to the system trash? This folder contains {} items. Projects keep their pinned copies.",
                draft.path,
                draft.fingerprints.len()
            ));
            ui.horizontal(|ui| {
                submit = controls::button(ui, "Delete library").clicked();
                cancel = controls::button(ui, "Cancel").clicked();
            });
        });
        if submit {
            self.message = self.confirm_library_delete(ctx, draft).err();
        } else if !closing && !cancel {
            self.dialog = Dialog::DeleteLibrary(draft);
        }
    }
    /// A failed trash request retains the source; only a completed deletion forgets registration.
    fn confirm_library_delete(&mut self, ctx: &egui::Context, draft: DeleteDraft) -> Result<(), String> {
        if self.library_sources.busy()
            || self.library_mutation_busy()
            || self.session.libraries().generation() != draft.generation
            || !self.library_sources.registered.iter().any(|r| r.source == draft.source && r.path == draft.path)
            || self.library_editors.unpublished_in(&draft.library)
        {
            return Err("Library or editor changed; review deletion again".into());
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            Err("Library deletion is unavailable in the browser".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (send, receive) = mpsc::channel();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("pcbcraft-library-trash".into())
                .spawn(move || {
                    let result = (|| {
                        let path = std::path::Path::new(&draft.path);
                        let current = pcbcraft_storage::library::deletion_snapshot(path)?;
                        if current.items().iter().map(|i| i.inspect()).collect::<Vec<_>>() != draft.fingerprints {
                            return Err("Library folder changed; review deletion again".into());
                        }
                        pcbcraft_storage::library::trash_library(path)?;
                        Ok(Outcome::Deleted { source: draft.source, path: draft.path })
                    })();
                    let _ = send.send(result);
                    ctx.request_repaint();
                })
                .map_err(|e| e.to_string())?;
            self.library_sources.mutation = Some(Job { receive });
            Ok(())
        }
    }
    /// Poll without waiting or joining; disconnects retire the slot and report a local error.
    pub(crate) fn poll_library_mutation(&mut self, ctx: &egui::Context) {
        let result = self.library_sources.mutation.as_ref().map(|job| job.receive.try_recv());
        match result {
            Some(Ok(result)) => {
                self.library_sources.mutation = None;
                match result {
                    #[cfg(not(target_arch = "wasm32"))]
                    Ok(Outcome::Prepared(draft)) => {
                        if matches!(self.dialog, Dialog::None) && self.session.libraries().generation() == draft.generation {
                            self.dialog = Dialog::DeleteLibrary(draft);
                        } else {
                            self.message = Some("Library changed during deletion inspection; try again".into());
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    Ok(Outcome::Deleted { source, path }) => {
                        if self.library_sources.registered.iter().any(|r| r.source == source && r.path == path) {
                            self.library_sources.remove(&source, &mut self.session);
                        }
                    }
                    Err(error) => self.message = Some(error),
                }
                self.repaint_editors(ctx);
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.library_sources.mutation = None;
                self.message = Some("Library operation disconnected; inspect its folder before retrying".into());
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            None => {}
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    /// A registered original fixture gives deletion a real directory and stable source identity.
    #[allow(clippy::unwrap_used)]
    fn fixture() -> (tempfile::TempDir, PcbApp) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("library");
        let documents = sources::Documents::from([
            (
                "library.json".into(),
                json!({"format":"pcbcraft-library/1","id":"user","name":"Delete fixture","license":"MIT OR Apache-2.0"}).to_string(),
            ),
            ("symbols/resistor.json".into(), include_str!("../../../../libraries/starter/symbols/resistor.json").into()),
        ]);
        let bundle = serde_json::to_vec(&json!({"format":"pcbcraft-library-bundle/1","documents":documents})).unwrap();
        let source = pcbcraft_storage::library::install_bundle(&bundle, &path).unwrap();
        let mut app = PcbApp::default();
        app.run("project.new", &json!({}));
        app.session.install_library_source("deletable", Arc::new(source)).unwrap();
        app.library_sources.registered.push(Registration {
            source: "deletable".into(),
            path: path.to_str().unwrap().into(),
            name: "Delete fixture".into(),
            writable: true,
        });
        (root, app)
    }
    /// Dirty editor protection and stale confirmation both refuse before starting a trash worker.
    #[test]
    fn deletion_refuses_dirty_editors_and_stale_registration() {
        let (_root, mut app) = fixture();
        let ctx = egui::Context::default();
        let before = app.session.inspect();
        app.new_library_item(pcbcraft_engine::authoring::Kind::Symbol);
        assert!(app.begin_library_delete(&ctx, Some("deletable")).unwrap_err().contains("editor changes"));
        assert!(!app.library_mutation_busy());
        let draft = DeleteDraft {
            source: "deletable".into(),
            path: app.library_sources.registered[0].path.clone(),
            library: "user".into(),
            generation: 0,
            fingerprints: Vec::new(),
        };
        assert!(app.confirm_library_delete(&ctx, draft).is_err());
        assert_eq!(app.session.inspect(), before);
    }
    /// A disconnected operation releases its bounded slot without forgetting or editing the source.
    #[test]
    fn disconnected_deletion_retains_registration_and_project() {
        let (_root, mut app) = fixture();
        let before = app.session.inspect();
        let (send, receive) = mpsc::channel();
        drop(send);
        app.library_sources.mutation = Some(Job { receive });
        app.poll_library_mutation(&egui::Context::default());
        assert!(!app.library_mutation_busy());
        assert_eq!(app.library_sources.registered.len(), 1);
        assert_eq!(app.session.inspect(), before);
    }
    /// Opt-in platform test: trash only an owned synthetic folder while project pinning stays exact.
    #[test]
    #[ignore = "requires an isolated XDG_DATA_HOME and the desktop system trash service"]
    fn confirmed_system_trash_preserves_pinned_project() {
        let (_root, mut app) = fixture();
        let ctx = egui::Context::default();
        let definition = app.session.libraries().source_items("deletable")[0].symbol().unwrap();
        app.place_catalog_symbol(&definition, json!({"x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
        assert_eq!(app.session.project().unwrap().symbols().len(), 1);
        let before = app.session.inspect();
        let encoded = pcbcraft_engine::persistence::encode_documents(app.session.snapshot().unwrap()).unwrap();
        let path = app.library_sources.registered[0].path.clone();
        app.begin_library_delete(&ctx, Some("deletable")).unwrap();
        let start = std::time::Instant::now();
        while app.library_mutation_busy() {
            app.poll_library_mutation(&ctx);
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let Dialog::DeleteLibrary(draft) = std::mem::take(&mut app.dialog) else { panic!("{:?}", app.message) };
        app.confirm_library_delete(&ctx, draft).unwrap();
        while app.library_mutation_busy() {
            app.poll_library_mutation(&ctx);
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.message.is_none(), "{:?}", app.message);
        assert!(!std::path::Path::new(&path).exists());
        assert!(app.library_sources.registered.is_empty());
        assert_eq!(app.session.inspect(), before);
        assert_eq!(pcbcraft_engine::persistence::encode_documents(app.session.snapshot().unwrap()).unwrap(), encoded);
        assert_eq!(
            pcbcraft_engine::persistence::encode_documents(pcbcraft_engine::persistence::decode_documents(&encoded).unwrap()).unwrap(),
            encoded
        );
    }
}
