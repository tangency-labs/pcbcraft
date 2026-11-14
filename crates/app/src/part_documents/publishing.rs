//! Part documents use the drawing editors' destination form, worker and recovery slot.
use super::*;
#[cfg(not(target_arch = "wasm32"))]
use crate::library_editors::publishing::Capture;
#[cfg(not(target_arch = "wasm32"))]
use crate::library_editors::publishing::{Completion, Event, Request, refresh_publication, verify_publication};
use crate::library_editors::publishing::{Draft, NewLibrary};
#[cfg(not(target_arch = "wasm32"))]
use pcbcraft_engine::authoring::publishing as engine;

impl PcbApp {
    /// Capture document revision before asking for a native publication destination.
    pub(crate) fn begin_part_publish(&mut self, id: u64) -> Result<(), String> {
        if crate::IN_BROWSER {
            return Err("Publishing native libraries requires the desktop host.".into());
        }
        if self.library_editors.busy() || self.library_sources.busy() || self.library_mutation_busy() {
            return Err("Finish the current library operation first.".into());
        }
        let doc = self.library_editors.parts.get(&id).ok_or("Unknown part document.")?;
        if !doc.pending.is_empty() {
            return Err("Apply or reset typed fields before publishing.".into());
        }
        if self.library_editors.publisher.unconfirmed() != Some(id) {
            doc.session.record().validate(self.session.libraries())?;
        }
        let address = doc.session.record().reference();
        let destinations = self.writable_libraries();
        let loaded = self.session.libraries().inspect();
        let source = doc
            .source
            .as_ref()
            .filter(|source| destinations.iter().any(|(id, _)| id == *source))
            .cloned()
            .or_else(|| {
                destinations
                    .iter()
                    .find(|(source, _)| {
                        loaded.as_array().into_iter().flatten().any(|entry| entry["source"] == *source && entry["library"] == address.library)
                    })
                    .map(|(source, _)| source.clone())
            })
            .unwrap_or_default();
        self.dialog =
            Dialog::LibraryPublish(Draft { document: id, revision: doc.session.revision(), source, allow_older: false, copy: false, error: None });
        Ok(())
    }
    /// Compact destination controls share the existing Publish/New library lifecycle.
    pub(crate) fn part_publish_dialog(&mut self, ctx: &egui::Context, mut draft: Draft) {
        let Some(doc) = self.library_editors.parts.get(&draft.document) else { return };
        let address = doc.session.record().reference();
        let name = doc.session.record().name().to_owned();
        let license = match doc.session.record() {
            Record::Part(p) => p.license.clone(),
            Record::Orderable(p) => p.license.clone(),
        };
        let kind = doc.session.record().kind();
        let base = doc.session.published();
        let dirty = doc.dirty();
        let verifying = self.library_editors.publisher.unconfirmed() == Some(draft.document);
        let destinations = self.writable_libraries();
        let loaded = self.session.libraries().inspect();
        if !destinations.iter().any(|(id, _)| id == &draft.source) {
            draft.source.clear();
        }
        let library =
            loaded.as_array().into_iter().flatten().find(|entry| entry["source"] == draft.source).and_then(|entry| entry["library"].as_str());
        let different = address.library != "user" && library.is_some_and(|id| id != address.library);
        let latest = self.session.libraries().highest_revision(library.unwrap_or(&address.library), kind, &address.id);
        let older = !(draft.copy || different) && latest > 0 && base.as_ref().is_none_or(|base| latest > base.revision);
        let mut create = false;
        let action = pcbcraft_chrome::catalog::property_form_enabled(
            ctx,
            "part_publish",
            &format!("Publish {name}"),
            452.,
            if verifying {
                "Verify publication"
            } else if draft.copy || different {
                "Publish copy"
            } else {
                "Publish"
            },
            verifying || (!draft.source.is_empty() && (dirty || draft.copy || different) && (!older || draft.allow_older)),
            |ui| {
                let before = draft.source.clone();
                create = properties::choice_with_action(
                    ui,
                    "Library",
                    ui.scope_id().with("part_destination"),
                    &mut draft.source,
                    &destinations.iter().map(|(id, name)| (id.as_str(), name.clone())).collect::<Vec<_>>(),
                    "New library…",
                );
                if before != draft.source {
                    draft.allow_older = false;
                    draft.error = None;
                    ctx.request_repaint();
                }
                if address.library != "user" {
                    let mut mode = if draft.copy || different { "copy" } else { "revision" }.to_owned();
                    ui.add_enabled_ui(!different, |ui| {
                        properties::choice(
                            ui,
                            "Save as",
                            ui.scope_id().with("part_publish_mode"),
                            &mut mode,
                            &[("revision", "New revision".into()), ("copy", "New item".into())],
                        );
                    });
                    draft.copy = mode == "copy";
                }
                if older {
                    properties::section(ui, "Newer revision available");
                    ui.label(format!("Revision {latest} is available. Publishing this draft may omit intervening changes."));
                    controls::checkbox(ui, &mut draft.allow_older, "Publish without merging");
                }
                controls::collapsing(ui, "part_publish_details", "Details", false, None, |ui| {
                    properties::field(ui, "Item ID", |ui| {
                        ui.label(&address.id);
                    });
                    properties::field(ui, "License", |ui| {
                        ui.label(&license);
                    });
                });
                if let Some(error) = &draft.error {
                    ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
                }
            },
        );
        if create {
            self.dialog = Dialog::NewLibrary(NewLibrary::returning(license, draft));
        } else if action.primary {
            if let Err(error) = self.publish_part_document(ctx, draft.document, draft.revision, &draft.source, draft.allow_older, draft.copy) {
                draft.error = Some(error);
                self.dialog = Dialog::LibraryPublish(draft);
            }
        } else if !action.close {
            self.dialog = Dialog::LibraryPublish(draft);
        }
    }
    /// Validate on the editor thread, then reuse no-clobber disk publication and uncertain-save
    /// recovery. Completion never publishes a project edit or enables placement of an unresolved draft.
    pub(crate) fn publish_part_document(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        revision: u64,
        source: &str,
        allow_older: bool,
        copy: bool,
    ) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, id, revision, source, allow_older, copy);
            Err("Publishing native libraries requires the desktop host.".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.library_editors.busy() || self.library_sources.busy() || self.library_mutation_busy() {
                return Err("Finish the current library operation first.".into());
            }
            let doc = self.library_editors.parts.get(&id).ok_or("Unknown part document.")?;
            if doc.session.revision() != revision || !doc.pending.is_empty() {
                return Err("Part document changed or has unapplied fields; review before publishing.".into());
            }
            if let Some(recovery) = self.library_editors.publisher.recovery.clone() {
                if recovery.document != id {
                    return Err("Verify the preceding uncertain publication first.".into());
                }
                return self.library_editors.publisher.start(ctx, move |_| verify_publication(recovery));
            }
            if !self.writable_libraries().iter().any(|(id, _)| id == source) {
                return Err("Choose an available writable native library.".into());
            }
            let loaded = self.session.libraries().inspect();
            let entry = loaded.as_array().into_iter().flatten().find(|entry| entry["source"] == source).ok_or("Library unavailable.")?;
            let library = entry["library"].as_str().ok_or("Missing library identity.")?;
            let manifest = engine::new_library(library, entry["name"].as_str().unwrap_or(library), entry["license"].as_str().unwrap_or_default())?;
            let latest = self.session.libraries().highest_revision(library, doc.session.record().kind(), &doc.session.record().reference().id);
            doc.session.prepare(self.session.libraries(), &manifest, latest, allow_older, copy)?;
            let path =
                self.library_sources.registered.iter().find(|entry| entry.source == source).ok_or("Library registration missing.")?.path.clone();
            let catalog = self.session.libraries().clone();
            let generation = catalog.generation();
            let session = doc.session.clone();
            let source = source.to_owned();
            let destination = source.clone();
            self.library_editors.publisher.start(ctx, move |send| {
                let disk = pcbcraft_storage::library::publishing::read_manifest(std::path::Path::new(&path))?;
                if serde_json::from_str::<Value>(&disk).map_err(|e| e.to_string())?
                    != serde_json::from_str::<Value>(&manifest).map_err(|e| e.to_string())?
                {
                    return Err("Destination changed; refresh before publishing.".into());
                }
                let prepared = session.prepare(&catalog, &disk, latest, allow_older, copy)?;
                let request = Request { document: id, source, path, generation, prepared: Capture::Part(std::sync::Arc::new(prepared)) };
                send.send(Event::Prepared(request.clone())).map_err(|_| "Publisher owner disconnected before writing.")?;
                match pcbcraft_storage::library::publishing::publish(std::path::Path::new(&request.path), &request.prepared) {
                    Ok(()) => Ok(refresh_publication(request)),
                    Err(pcbcraft_storage::library::publishing::Failure::BeforeWrite(error)) => Ok(Completion::NotWritten(error)),
                    Err(pcbcraft_storage::library::publishing::Failure::Published(error)) => Ok(Completion::Unconfirmed(request, error)),
                }
            })?;
            self.library_editors.set_part_destination(id, destination);
            self.library_editors.notice = Some("Publishing…".into());
            Ok(())
        }
    }
    /// The captured document alone adopts durable identity; newer field edits remain dirty.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn confirm_part_publication(&mut self, id: u64, prepared: &Capture) {
        if let Capture::Part(prepared) = prepared
            && let Some(doc) = self.library_editors.parts.get_mut(&id)
        {
            doc.session.confirm(prepared);
        }
    }
}
