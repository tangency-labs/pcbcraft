//! Fabrication output export: one ZIP containing Gerber/drill files and their manifest.
//!
//! The UI thread captures the complete ZIP bytes. A native worker selects and writes the
//! destination; browser builds request one download instead. The editor allows one active
//! export request; it does not lock destinations or track browser downloads. Dropping the
//! application never joins the worker, and a worker that stops reports a disconnect.
use pcbcraft_engine::board::fabrication::Archive;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// How an export request ended. `Saved` means desktop publication completed. A browser
/// request gets no completion or cancellation signal, and UI tests inject the outcome.
#[derive(Debug)]
#[cfg_attr(any(test, target_arch = "wasm32"), allow(dead_code))]
pub(super) enum Outcome {
    Saved,
    #[cfg(any(test, target_arch = "wasm32"))]
    DownloadRequested,
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

    /// Start desktop publication or request a browser download from the capture. Refuses while
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

/// Ask where to save, then publish the captured ZIP at the selected path.
/// Cancellation writes nothing; publication errors are reported rather than success.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn native(name: &str, archive: &Archive) -> Result<Outcome, String> {
    let Some(path) = rfd::FileDialog::new().set_title("Save fabrication outputs").set_file_name(name).add_filter("Zip archive", &["zip"]).save_file()
    else {
        return Ok(Outcome::Cancelled);
    };
    publish_archive_with_sync(&path, archive.bytes(), sync_directory)?;
    Ok(Outcome::Saved)
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

/// Request the captured ZIP through the browser bridge.
#[cfg(all(target_arch = "wasm32", not(test)))]
fn download_archive(name: &str, archive: &Archive) -> Result<Outcome, String> {
    request_zip_download(name, archive, download)
}

/// Hand the full capture to a single download request, without claiming a filesystem save.
#[cfg(any(all(test, not(target_arch = "wasm32")), all(target_arch = "wasm32", not(test))))]
fn request_zip_download<E: std::fmt::Debug>(
    name: &str,
    archive: &Archive,
    request: impl FnOnce(&[u8], &str, &str) -> Result<(), E>,
) -> Result<Outcome, String> {
    request(archive.bytes(), name, "application/zip").map_err(|error| format!("ZIP download request failed: {error:?}"))?;
    Ok(Outcome::DownloadRequested)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
