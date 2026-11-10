//! Fabrication output export: one archive of Gerber and drill files, saved off the editor loop.
//!
//! The archive bytes are built deterministically from the current snapshot on the UI thread;
//! only the save dialog and filesystem write run on a worker. One export runs at
//! a time. Dropping the application never joins the worker, and a worker that stops reports a
//! disconnect instead of leaving the slot busy forever.
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// How an export finished. Tests capture the archive instead of saving, and browser downloads
/// cannot be cancelled, so those builds never construct every variant.
#[derive(Debug)]
#[cfg_attr(any(test, target_arch = "wasm32"), allow(dead_code))]
pub(super) enum Outcome {
    Saved,
    Cancelled,
}

/// The single export slot.
#[derive(Debug, Default)]
pub(crate) struct Exports {
    pending: Option<Receiver<Result<Outcome, String>>>,
    /// Tests capture the archive instead of opening a save dialog.
    #[cfg(test)]
    pub(super) captured: Option<(String, Vec<u8>)>,
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

    /// Save the captured archive under a suggested name. Refuses while another export runs.
    pub(super) fn start(&mut self, ctx: &egui::Context, name: String, bytes: Vec<u8>) -> Result<(), String> {
        if self.busy() {
            return Err("Finish the current fabrication export first.".into());
        }
        let (send, receive) = mpsc::sync_channel(1);
        #[cfg(test)]
        {
            let _ = ctx;
            self.captured = Some((name, bytes));
            self.completion = Some(send);
        }
        #[cfg(not(test))]
        {
            let ctx = ctx.clone();
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::Builder::new()
                .name("fabrication-export".into())
                .spawn(move || {
                    let _ = send.send(native(&name, &bytes));
                    ctx.request_repaint();
                })
                .map_err(|e| e.to_string())?;
            #[cfg(target_arch = "wasm32")]
            {
                let _ = send.send(download(&bytes, &name).map(|_| Outcome::Saved).map_err(|e| format!("Download failed: {e:?}")));
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

/// Ask where to save, then publish the archive atomically: a complete temporary file in the
/// destination folder replaces the target only after it is written and synced.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn native(name: &str, bytes: &[u8]) -> Result<Outcome, String> {
    let Some(path) = rfd::FileDialog::new().set_title("Save fabrication outputs").set_file_name(name).add_filter("Zip archive", &["zip"]).save_file()
    else {
        return Ok(Outcome::Cancelled);
    };
    publish_archive(&path, bytes)?;
    Ok(Outcome::Saved)
}

/// Publish complete bytes with ordinary umask-filtered creation permissions on Unix, retaining
/// an existing file's permissions on replacement. Sync failures after rename report that the
/// new archive is already present but its directory entry's durability is unconfirmed.
#[cfg(not(target_arch = "wasm32"))]
fn publish_archive(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    publish_archive_with_sync(path, bytes, sync_directory)
}

/// Stage, flush and rename before invoking the directory-sync boundary. Any earlier failure
/// drops the staging file and leaves the destination intact; a later sync failure is propagated.
#[cfg(not(target_arch = "wasm32"))]
fn publish_archive_with_sync(path: &std::path::Path, bytes: &[u8], sync: impl FnOnce(&std::path::Path) -> std::io::Result<()>) -> Result<(), String> {
    use std::{fs, io::Write};
    let permissions = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
        Ok(_) => return Err("The archive destination is not a regular file.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let parent = path.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| std::path::Path::new("."));
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::PermissionsExt;
        if permissions.is_some() {
            // Do not expose a private overwritten file through a wider staging mode.
            tempfile::NamedTempFile::new_in(parent)
        } else {
            tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o666)).tempfile_in(parent)
        }
    }
    .map_err(|error| error.to_string())?;
    #[cfg(not(unix))]
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    if let Some(permissions) = permissions {
        file.as_file().set_permissions(permissions).map_err(|error| error.to_string())?;
    }
    file.as_file().sync_all().map_err(|error| error.to_string())?;
    file.persist(path).map_err(|error| error.error.to_string())?;
    sync(parent).map_err(|error| format!("Archive published, but syncing its directory failed: {error}"))
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
export function downloadFabrication(bytes,name){const url=URL.createObjectURL(new Blob([bytes],{type:'application/zip'}));const a=document.createElement('a');a.href=url;a.download=name;document.body.appendChild(a);a.click();a.remove();setTimeout(()=>URL.revokeObjectURL(url),30000);}
")]
extern "C" {
    #[wasm_bindgen(catch, js_name = downloadFabrication)]
    fn download(bytes: &[u8], name: &str) -> Result<(), JsValue>;
}

/// A file name for the archive from the project name, keeping only portable characters.
pub(super) fn archive_name(project: &str) -> String {
    let cleaned: String = project.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let base = if cleaned.trim_matches('_').is_empty() { "board".to_owned() } else { cleaned };
    format!("{base}-fabrication.zip")
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
