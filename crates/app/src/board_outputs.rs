//! Fabrication output export: the existing Gerber/drill archive and an integrity sidecar.
//!
//! The UI thread captures deterministic ZIP bytes and member records. A native worker selects
//! the destination, serializes the filename-bound sidecar and writes the files; browser builds
//! request downloads instead. The editor allows one active export request; it does not lock
//! destinations or track browser downloads. Dropping the application never joins the worker,
//! and a worker that stops reports a disconnect instead of leaving the slot busy.
use pcbcraft_engine::board::fabrication::Archive;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// How an export request ended. On desktop `Saved` means both writes completed, not that the
/// files were checked as a matching pair. The browser path receives no completion or
/// cancellation signal from the browser, and UI tests inject the outcome.
#[derive(Debug)]
#[cfg_attr(any(test, target_arch = "wasm32"), allow(dead_code))]
pub(super) enum Outcome {
    Saved,
    #[cfg(any(test, target_arch = "wasm32"))]
    DownloadsRequested,
    Cancelled,
}

/// The single export slot.
#[derive(Debug, Default)]
pub(crate) struct Exports {
    pending: Option<Receiver<Result<Outcome, String>>>,
    /// Tests capture the archive instead of opening a save dialog.
    #[cfg(test)]
    pub(super) captured: Option<(String, Archive)>,
    #[cfg(test)]
    completion: Option<mpsc::SyncSender<Result<Outcome, String>>>,
}

impl Exports {
    /// Whether an export's dialog or write is still running.
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Take a finished export's result without waiting. A worker that stopped without reporting
    /// is an error, and the slot is released either way.
    pub(super) fn poll(&mut self) -> Option<Result<Outcome, String>> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("The fabrication export stopped before reporting its result.".into()),
        };
        self.pending = None;
        Some(result)
    }

    /// Start desktop publication or request browser downloads from the capture. Refuses while
    /// this editor's export slot is busy; neither path rereads the design.
    pub(super) fn start(&mut self, ctx: &egui::Context, name: String, archive: Archive) -> Result<(), String> {
        if self.busy() {
            return Err("Finish the current fabrication export first.".into());
        }
        let (send, receive) = mpsc::sync_channel(1);
        #[cfg(test)]
        {
            let _ = ctx;
            self.captured = Some((name, archive));
            self.completion = Some(send);
        }
        #[cfg(not(test))]
        {
            let ctx = ctx.clone();
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::Builder::new()
                .name("fabrication-export".into())
                .spawn(move || {
                    let _ = send.send(native(&name, &archive));
                    ctx.request_repaint();
                })
                .map_err(|e| e.to_string())?;
            #[cfg(target_arch = "wasm32")]
            {
                let _ = send.send(download_archive(&name, &archive));
                ctx.request_repaint();
            }
        }
        self.pending = Some(receive);
        Ok(())
    }

    /// Complete a captured test export through the same channel the native save worker uses.
    /// Refuses if no captured request owns a completion sender.
    #[cfg(test)]
    pub(super) fn complete_captured(&mut self, result: Result<Outcome, String>) -> Result<(), String> {
        self.completion.take().ok_or("No captured export is pending.")?.send(result).map_err(|error| error.to_string())
    }
}

/// Ask where to save, then publish ZIP and sidecar sequentially, not as an atomic pair.
/// Cancellation writes neither; any publication failure is reported rather than success.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn native(name: &str, archive: &Archive) -> Result<Outcome, String> {
    let Some(path) = rfd::FileDialog::new().set_title("Save fabrication outputs").set_file_name(name).add_filter("Zip archive", &["zip"]).save_file()
    else {
        return Ok(Outcome::Cancelled);
    };
    publish_package(&path, archive)?;
    Ok(Outcome::Saved)
}

/// Check and stage the unselected companion before replacing the selected ZIP. Never replace
/// a companion, even if it appears after the check; use a fresh destination for a new pair.
#[cfg(not(target_arch = "wasm32"))]
fn publish_package(path: &std::path::Path, archive: &Archive) -> Result<(), String> {
    publish_package_with_sync(path, archive, sync_directory)
}

/// Publish the staged ZIP followed by a no-clobber companion. Directory sync runs after each
/// installation. Errors distinguish a failed companion install from an installed but unconfirmed
/// durable entry. There is no rollback or coordination with other writers of the ZIP.
#[cfg(not(target_arch = "wasm32"))]
fn publish_package_with_sync(
    path: &std::path::Path,
    archive: &Archive,
    mut sync: impl FnMut(&std::path::Path) -> std::io::Result<()>,
) -> Result<(), String> {
    use std::{fs, io::ErrorKind};
    let name = path.file_name().and_then(|name| name.to_str()).ok_or("The archive filename must be valid UTF-8 for its manifest.")?;
    let companion_name = format!("{name}.manifest.json");
    let sidecar = path.with_file_name(&companion_name);
    let refusal = |reason: &str| {
        format!("Cannot write manifest {}: {reason}. Choose a shorter or unused ZIP filename, or move the existing companion.", sidecar.display())
    };
    // Bound the Windows basename explicitly; path lookup can report an oversized name as absent.
    #[cfg(windows)]
    if companion_name.encode_utf16().count() > 255 {
        return Err(refusal("the companion filename exceeds 255 UTF-16 code units"));
    }
    match fs::symlink_metadata(&sidecar) {
        Ok(_) => return Err(refusal("the companion path already exists and will not be replaced")),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(refusal(&error.to_string())),
    }
    let manifest = archive.manifest_json(name)?;
    let parent = output_directory(path);
    let staged = stage_output(parent, &manifest, None).map_err(|error| refusal(&error.to_string()))?;
    publish_archive_with_sync(path, archive.bytes(), &mut sync)
        .map_err(|error| format!("Manifest not installed; archive publication failed (the ZIP may already be replaced): {error}"))?;
    staged
        .persist_noclobber(&sidecar)
        .map_err(|error| format!("ZIP published, but its manifest failed to install: {}", refusal(&error.error.to_string())))?;
    sync(parent).map_err(|error| format!("ZIP and manifest installed, but syncing the manifest directory failed; durability is unconfirmed: {error}"))
}

/// Stage, flush and rename before invoking the directory-sync boundary. Any earlier failure
/// drops the staging file and leaves the destination intact; a later sync failure is propagated.
#[cfg(not(target_arch = "wasm32"))]
fn publish_archive_with_sync(path: &std::path::Path, bytes: &[u8], sync: impl FnOnce(&std::path::Path) -> std::io::Result<()>) -> Result<(), String> {
    let permissions = match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
        Ok(_) => return Err("The archive destination is not a regular file.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let parent = output_directory(path);
    stage_output(parent, bytes, permissions).map_err(|error| error.to_string())?.persist(path).map_err(|error| error.error.to_string())?;
    sync(parent).map_err(|error| format!("Archive published, but syncing its directory failed: {error}"))
}

/// Use the destination directory for staging, including the current directory for a bare name.
#[cfg(not(target_arch = "wasm32"))]
fn output_directory(path: &std::path::Path) -> &std::path::Path {
    path.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| std::path::Path::new("."))
}

/// Stage and sync complete bytes without changing the destination. New files use ordinary
/// umask-filtered permissions on Unix; replacements stage privately and inherit existing modes.
#[cfg(not(target_arch = "wasm32"))]
fn stage_output(parent: &std::path::Path, bytes: &[u8], permissions: Option<std::fs::Permissions>) -> std::io::Result<tempfile::NamedTempFile> {
    use std::io::Write;
    #[cfg(unix)]
    let mut file = {
        use std::{fs::Permissions, os::unix::fs::PermissionsExt};
        if permissions.is_some() {
            tempfile::NamedTempFile::new_in(parent)
        } else {
            tempfile::Builder::new().permissions(Permissions::from_mode(0o666)).tempfile_in(parent)
        }
    }?;
    #[cfg(not(unix))]
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    if let Some(permissions) = permissions {
        file.as_file().set_permissions(permissions)?;
    }
    file.as_file().sync_all()?;
    Ok(file)
}

/// Flush the rename's directory entry on Unix. Other platforms retain their existing native
/// rename behavior; this helper does not claim a portable power-loss durability guarantee.
#[cfg(not(target_arch = "wasm32"))]
fn sync_directory(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(all(target_arch = "wasm32", not(test)))]
use wasm_bindgen::prelude::*;
#[cfg(all(target_arch = "wasm32", not(test)))]
#[wasm_bindgen(inline_js = "
export function downloadFabrication(bytes,name,type){
    const url=URL.createObjectURL(new Blob([bytes],{type}));
    let a;
    let requested=false;
    try{
        a=document.createElement('a');
        a.href=url;
        a.download=name;
        document.body.appendChild(a);
        a.click();
        requested=true;
    }finally{
        try{a?.remove();}
        finally{
            if(requested){setTimeout(()=>URL.revokeObjectURL(url),30000);}
            else{URL.revokeObjectURL(url);}
        }
    }
}
")]
extern "C" {
    #[wasm_bindgen(catch, js_name = downloadFabrication)]
    fn download(bytes: &[u8], name: &str, media_type: &str) -> Result<(), JsValue>;
}

/// Request both browser downloads from the capture. Browsers can silently block multiple
/// downloads, so a successful request is not a confirmed filesystem save.
#[cfg(all(target_arch = "wasm32", not(test)))]
fn download_archive(name: &str, archive: &Archive) -> Result<Outcome, String> {
    let manifest = archive.manifest_json(name)?;
    download(archive.bytes(), name, "application/zip").map_err(|error| format!("ZIP download failed; manifest not requested: {error:?}"))?;
    download(&manifest, &format!("{name}.manifest.json"), "application/json")
        .map_err(|error| format!("ZIP download requested, but manifest download failed: {error:?}"))?;
    Ok(Outcome::DownloadsRequested)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
