//! Item deletion captures a source-local revision group and shares the source-operation slot.
#[cfg(not(target_arch = "wasm32"))]
use super::lifecycle::{Job, Outcome};
use super::*;
#[cfg(not(target_arch = "wasm32"))]
use crate::Dialog;
#[cfg(not(target_arch = "wasm32"))]
use pcbcraft_engine::library::sources::ItemRemoval;
use pcbcraft_engine::library::sources::Key;
#[cfg(not(target_arch = "wasm32"))]
use pcbcraft_engine::part_authoring::Record;

/// The confirmed path and snapshot remain host-owned; live inspection exposes only a token
/// and item metadata, never filesystem authority supplied by a caller.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub(crate) struct Draft {
    source: String,
    path: String,
    pub(super) generation: u64,
    plan: Arc<pcbcraft_storage::library::deletion::Plan>,
    error: Option<String>,
}
impl PcbApp {
    /// Block known library references and open authoring documents without affecting project pins.
    #[cfg(not(target_arch = "wasm32"))]
    fn check_item_delete_documents(&self, removal: &ItemRemoval) -> Result<(), String> {
        self.session.libraries().check_item_removal(removal)?;
        for doc in self.library_editors.parts.values() {
            let record = doc.session.record();
            let dependent = matches!(record,Record::Orderable(record) if removal.includes(sources::Kind::Part,&record.part));
            if removal.includes(record.kind(), &record.reference()) || dependent {
                return Err("Close this part's open authoring document and any dependent orderable drafts before deleting it.".into());
            }
        }
        Ok(())
    }
    /// Review actual native files on a worker. The GUI and live command enter this same path;
    /// preparing or cancelling never changes a record, registration or project history.
    pub(crate) fn begin_library_item_delete(&mut self, ctx: &egui::Context, source: &str, key: &Key) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, source, key);
            Err("Part deletion requires a qualified desktop host.".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !cfg!(unix) {
                return Err("Safe per-part deletion is not available on this platform.".into());
            }
            if self.library_sources.busy() || self.library_mutation_busy() {
                return Err("Finish the current library operation first.".into());
            }
            if !self.writable_libraries().iter().any(|(id, _)| id == source) {
                return Err("Choose a writable external folder library.".into());
            }
            let removal = self.session.libraries().item_removal(source, key)?;
            self.check_item_delete_documents(&removal)?;
            let registration =
                self.library_sources.registered.iter().find(|r| r.source == source).ok_or("Library registration disappeared.")?.clone();
            let generation = self.session.libraries().generation();
            let key = key.clone();
            let ctx = ctx.clone();
            let (send, receive) = mpsc::channel();
            std::thread::Builder::new()
                .name("pcbcraft-part-delete-check".into())
                .spawn(move || {
                    let result = pcbcraft_storage::library::deletion::prepare(std::path::Path::new(&registration.path), &key).and_then(|plan| {
                        if plan.removal() != &removal {
                            return Err("Part revisions changed on disk; refresh before deleting.".into());
                        }
                        Ok(Outcome::ItemPrepared(Box::new(Draft {
                            source: registration.source,
                            path: registration.path,
                            generation,
                            plan: Arc::new(plan),
                            error: None,
                        })))
                    });
                    let _ = send.send(result);
                    ctx.request_repaint();
                })
                .map_err(|e| e.to_string())?;
            self.library_sources.item_delete_pending = true;
            self.library_sources.item_delete_canceled = false;
            self.library_sources.mutation = Some(Job { receive });
            Ok(())
        }
    }
    /// The token and catalog generation authorize exactly the reviewed snapshot, not the
    /// current browser selection. Disk validation and reversible staging run off the UI thread.
    #[cfg(not(target_arch = "wasm32"))]
    fn confirm_library_item_delete(&mut self, ctx: &egui::Context, draft: &Draft, token: &str) -> Result<(), String> {
        if token != draft.plan.token()
            || self.session.libraries().generation() != draft.generation
            || self.library_sources.busy()
            || self.library_mutation_busy()
            || !self.library_sources.registered.iter().any(|r| r.source == draft.source && r.path == draft.path && r.writable)
        {
            return Err("Library changed or deletion token is stale; review deletion again.".into());
        }
        self.check_item_delete_documents(draft.plan.removal())?;
        let draft = draft.clone();
        let ctx = ctx.clone();
        let (send, receive) = mpsc::channel();
        std::thread::Builder::new()
            .name("pcbcraft-part-trash".into())
            .spawn(move || {
                let path = std::path::Path::new(&draft.path);
                let (result, error) = match pcbcraft_storage::library::deletion::execute(path, &draft.plan) {
                    Ok(source) => (Ok(Arc::new(source)), None),
                    Err(error) => (pcbcraft_storage::library::load(path).map(Arc::new), Some(error)),
                };
                let _ = send.send(Ok(Outcome::ItemFinished { source: draft.source, path: draft.path, result, error }));
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        self.library_sources.item_delete_pending = true;
        self.library_sources.mutation = Some(Job { receive });
        Ok(())
    }
    /// Use the shared confirmation form, keeping dependency/stale errors beside the captured item.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn delete_library_item_dialog(&mut self, ctx: &egui::Context, mut draft: Draft) {
        let action = pcbcraft_chrome::catalog::property_form_enabled(
            ctx,
            "delete_library_part",
            "Delete part",
            520.,
            "Delete part",
            !self.library_mutation_busy(),
            |ui| {
                let count = draft.plan.removal().revisions.len();
                let revisions = if count == 1 { "revision" } else { "revisions" };
                ui.label(format!(
                    "Move “{}” to system trash? This removes {count} published {revisions} from this library folder.",
                    draft.plan.removal().name
                ));
                ui.label("Existing projects keep their pinned copies. Symbols, footprints and categories are kept.");
                pcbcraft_chrome::controls::collapsing(ui, "delete_part_details", "Details", false, None, |ui| {
                    ui.label(&draft.path);
                    ui.label(format!("{}/{}", draft.plan.removal().library, draft.plan.removal().id));
                });
                if let Some(error) = &draft.error {
                    ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
                }
            },
        );
        if action.primary {
            match self.confirm_library_item_delete(ctx, &draft, draft.plan.token()) {
                Ok(()) => {}
                Err(error) => {
                    draft.error = Some(error);
                    self.dialog = Dialog::DeleteLibraryItem(Box::new(draft));
                }
            }
        } else if !action.close {
            self.dialog = Dialog::DeleteLibraryItem(Box::new(draft));
        } else {
            self.library_sources.item_delete_canceled = true;
        }
    }
    /// Live clients discover a confirmation token only after disk inspection completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn inspect_library_item_deletion(&self) -> serde_json::Value {
        if let Dialog::DeleteLibraryItem(draft) = &self.dialog {
            json!({"phase":"review","source":draft.source,"library":draft.plan.removal().library,"id":draft.plan.removal().id,"kind":draft.plan.removal().kind,"revisions":draft.plan.removal().revisions,"token":draft.plan.token(),"error":draft.error})
        } else {
            json!({"phase":if self.library_sources.item_delete_pending {"working"} else {"idle"}})
        }
    }
    /// Explicit live confirmation/cancellation shares GUI validation; a missing token cannot delete.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn library_item_delete_phase(&mut self, ctx: &egui::Context, phase: &str, token: Option<&str>) -> Result<(), String> {
        match phase {
            "cancel" => {
                self.library_sources.item_delete_canceled = true;
                if matches!(self.dialog, Dialog::DeleteLibraryItem(_)) {
                    self.dialog = Dialog::None;
                }
                Ok(())
            }
            "confirm" => {
                let Dialog::DeleteLibraryItem(draft) = &self.dialog else { return Err("Prepare and inspect a part deletion first.".into()) };
                let draft = draft.clone();
                self.confirm_library_item_delete(ctx, &draft, token.ok_or("A reviewed deletion token is required.")?)?;
                self.dialog = Dialog::None;
                Ok(())
            }
            _ => Err("Choose prepare, confirm or cancel.".into()),
        }
    }
}

#[cfg(all(test, unix, not(target_arch = "wasm32")))]
pub(crate) mod tests {
    use super::*;
    use pcbcraft_engine::{
        authoring::publishing::Publication,
        library::sources::Kind,
        part_authoring::{Record, Session},
    };
    use std::time::{Duration, Instant};
    /// A published original native record has a writable real folder and no open authoring tab.
    #[allow(clippy::unwrap_used, clippy::panic)]
    pub(crate) fn fixture() -> (tempfile::TempDir, PcbApp, Key) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let manifest = pcbcraft_engine::authoring::publishing::new_library("item-delete", "Item delete", "MIT OR Apache-2.0").unwrap();
        pcbcraft_storage::library::publishing::create(&root, &manifest).unwrap();
        let record =
            Session::new(Record::new_part(None).unwrap(), false).unwrap().prepare(&sources::Catalog::bundled(), &manifest, 0, false, false).unwrap();
        pcbcraft_storage::library::publishing::publish(&root, &record).unwrap();
        let address = record.reference();
        let key = Key { library: address.library, id: address.id, revision: 1, kind: Kind::Part };
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({})).unwrap();
        app.session.install_library_source("deletable-item", Arc::new(pcbcraft_storage::library::load(&root).unwrap())).unwrap();
        app.library_sources.registered.push(Registration {
            source: "deletable-item".into(),
            path: root.to_str().unwrap().into(),
            name: "Item delete".into(),
            writable: true,
        });
        app.place_library_workspace(&egui::Context::default(), pcbcraft_chrome::workspace::Placement::Here).unwrap();
        app.run_window_command(
            &egui::Context::default(),
            "app.library_browse",
            &json!({"view":app.views.active.id().0,"source":"deletable-item","item":key.id}),
        )
        .unwrap();
        (temp, app, key)
    }
    /// Worker inspection must retire its single source-operation slot without joining the UI.
    pub(crate) fn finish(app: &mut PcbApp, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.library_sources.mutating() && Instant::now() < deadline {
            app.poll_library_mutation(ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!app.library_sources.mutating());
    }
    /// Ribbon deletion opens the reviewed form and Cancel preserves every native record and
    /// project snapshot. Item deletion is distinct from the whole-library source action.
    #[test]
    fn item_delete_ribbon_review_cancel_preserves_files_and_project() {
        use egui_kittest::{Harness, kittest::Queryable};
        let (_temp, app, key) = fixture();
        let snapshot = app.session.snapshot().unwrap();
        let mut h =
            Harness::builder().with_max_steps(64).with_size(egui::vec2(1800., 1000.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        h.get_by_label("Delete selected part").click();
        h.run();
        let ctx = h.ctx.clone();
        finish(h.state_mut(), &ctx);
        h.run();
        let inspection = h.state().inspect_library_item_deletion();
        assert_eq!(inspection["phase"], "review");
        assert_eq!(inspection["revisions"], json!([1]));
        h.get_by_label("Cancel").click();
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(h.state().session.libraries().item(&key).is_ok());
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
    }
    /// Live confirmation needs the exact reviewed token and generation. Known open owner and
    /// dependent drafts block deletion before a file worker is launched.
    #[test]
    fn tokens_open_documents_and_catalog_changes_are_rechecked() {
        let (_temp, mut app, key) = fixture();
        let ctx = egui::Context::default();
        let record = app.session.libraries().item(&key).unwrap().part().unwrap();
        let id = app.library_editors.open_part(Record::Part((*record).clone()), true, Some("deletable-item".into())).unwrap();
        assert!(app.begin_library_item_delete(&ctx, "deletable-item", &key).unwrap_err().contains("Close"));
        assert!(!app.library_sources.mutating());
        app.library_editors.close_part(id, false).unwrap();
        app.run_window_command(&ctx, "app.library_delete_part", &json!({"view":app.views.active.id().0,"phase":"prepare"})).unwrap();
        finish(&mut app, &ctx);
        let token = app.inspect_library_item_deletion()["token"].as_str().unwrap().to_owned();
        assert!(
            app.run_window_command(&ctx, "app.library_delete_part", &json!({"view":app.views.active.id().0,"phase":"confirm","token":"wrong"}))
                .unwrap_err()
                .contains("stale")
        );
        let manifest = pcbcraft_engine::authoring::publishing::new_library("changed", "Changed", "MIT OR Apache-2.0").unwrap();
        app.session
            .install_library_source("changed", Arc::new(NativeSource::load(&sources::Documents::from([("library.json".into(), manifest)])).unwrap()))
            .unwrap();
        assert!(
            app.run_window_command(&ctx, "app.library_delete_part", &json!({"view":app.views.active.id().0,"phase":"confirm","token":token}))
                .unwrap_err()
                .contains("stale")
        );
        app.run_window_command(&ctx, "app.library_delete_part", &json!({"view":app.views.active.id().0,"phase":"cancel"})).unwrap();
        assert!(matches!(app.dialog, Dialog::None));
        assert!(app.session.libraries().item(&key).is_ok());
    }
    /// Cancellation of a pending inspection does not reopen its confirmation after completion.
    #[test]
    fn canceled_inspection_does_not_restore_a_stale_dialog() {
        let (_temp, mut app, key) = fixture();
        let ctx = egui::Context::default();
        app.begin_library_item_delete(&ctx, "deletable-item", &key).unwrap();
        app.library_item_delete_phase(&ctx, "cancel", None).unwrap();
        finish(&mut app, &ctx);
        assert!(matches!(app.dialog, Dialog::None));
        assert!(app.session.libraries().item(&key).is_ok());
    }
    /// The worker's refreshed snapshot removes stale row selection without touching project
    /// history, and refresh failure makes the registration unavailable rather than showing old data.
    #[test]
    fn completion_refreshes_rows_and_failed_refresh_is_unavailable() {
        use egui_kittest::Harness;
        let (_temp, app, key) = fixture();
        let snapshot = app.session.snapshot().unwrap();
        let mut h =
            Harness::builder().with_max_steps(64).with_size(egui::vec2(1800., 1000.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let path = h.state().library_sources.registered[0].path.clone();
        let documents =
            sources::Documents::from([("library.json".into(), std::fs::read_to_string(std::path::Path::new(&path).join("library.json")).unwrap())]);
        let loaded = Arc::new(NativeSource::load(&documents).unwrap());
        let (send, receive) = mpsc::channel();
        send.send(Ok(Outcome::ItemFinished { source: "deletable-item".into(), path: path.clone(), result: Ok(loaded), error: None })).unwrap();
        h.state_mut().library_sources.mutation = Some(Job { receive });
        let ctx = h.ctx.clone();
        h.state_mut().poll_library_mutation(&ctx);
        h.run();
        assert!(h.state().session.libraries().item(&key).is_err());
        assert!(h.state().inspect_library_workspace(h.state().views.active.library.as_ref().unwrap())["selected"].is_null());
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
        let (send, receive) = mpsc::channel();
        send.send(Ok(Outcome::ItemFinished {
            source: "deletable-item".into(),
            path,
            result: Err("Refresh failed".into()),
            error: Some("Trash uncertain".into()),
        }))
        .unwrap();
        h.state_mut().library_sources.mutation = Some(Job { receive });
        h.state_mut().poll_library_mutation(&ctx);
        assert_eq!(h.state().library_sources.statuses["deletable-item"], "Refresh failed");
        assert_eq!(h.state().message.as_deref(), Some("Trash uncertain"));
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
    }
}
