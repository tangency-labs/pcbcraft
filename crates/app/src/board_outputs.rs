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

/// Label the common archive publisher so assembly and fabrication saves keep their own wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Purpose {
    Fabrication,
    Assembly,
}

impl Purpose {
    /// Return the title shown by the native file picker.
    #[cfg(all(not(target_arch = "wasm32"), not(test)))]
    fn title(self) -> &'static str {
        match self {
            Self::Fabrication => "Save fabrication outputs",
            Self::Assembly => "Save assembly outputs",
        }
    }

    /// Return a concise success notice for the saved archive.
    pub(super) fn success_message(self) -> &'static str {
        match self {
            Self::Fabrication => "Saved fabrication outputs",
            Self::Assembly => "Saved assembly outputs",
        }
    }
}

/// The single export slot.
#[derive(Debug, Default)]
pub(crate) struct Exports {
    pending: Option<Receiver<Result<(Purpose, Outcome), String>>>,
    #[cfg(test)]
    purpose: Option<Purpose>,
    /// Tests capture the archive instead of opening a save dialog.
    #[cfg(test)]
    pub(super) captured: Option<(String, Vec<u8>)>,
    #[cfg(test)]
    completion: Option<mpsc::SyncSender<Result<(Purpose, Outcome), String>>>,
}

impl Exports {
    /// Whether an export's dialog or write is still running.
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Take a finished export's result without waiting. A worker that stopped without reporting
    /// is an error, and the slot is released either way.
    #[cfg(test)]
    pub(super) fn poll(&mut self) -> Option<Result<Outcome, String>> {
        self.poll_with_purpose().map(|result| result.map(|(_, outcome)| outcome))
    }

    /// Take a completed result together with the owner that selected its save wording.
    pub(super) fn poll_with_purpose(&mut self) -> Option<Result<(Purpose, Outcome), String>> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("The output export stopped before reporting its result.".into()),
        };
        self.pending = None;
        Some(result)
    }

    /// Save the captured archive under a suggested name. Refuses while another export runs.
    #[cfg(test)]
    pub(super) fn start(&mut self, ctx: &egui::Context, name: String, bytes: Vec<u8>) -> Result<(), String> {
        self.start_for(ctx, Purpose::Fabrication, name, bytes)
    }

    /// Save one archive for a specific output workflow, using the same bounded publisher.
    pub(super) fn start_for(&mut self, ctx: &egui::Context, purpose: Purpose, name: String, bytes: Vec<u8>) -> Result<(), String> {
        if self.busy() {
            return Err("Finish the current export first.".into());
        }
        let (send, receive) = mpsc::sync_channel(1);
        #[cfg(test)]
        {
            let _ = ctx;
            self.captured = Some((name, bytes));
            self.completion = Some(send);
            self.purpose = Some(purpose);
        }
        #[cfg(not(test))]
        {
            let ctx = ctx.clone();
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::Builder::new()
                .name("pcbcraft-output-export".into())
                .spawn(move || {
                    let _ = send.send(native(&name, &bytes, purpose));
                    ctx.request_repaint();
                })
                .map_err(|e| e.to_string())?;
            #[cfg(target_arch = "wasm32")]
            {
                let _ = send.send(download(&bytes, &name).map(|_| (purpose, Outcome::Saved)).map_err(|e| format!("Download failed: {e:?}")));
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
        let purpose = self.purpose.ok_or("No captured export purpose is pending.")?;
        self.completion
            .take()
            .ok_or("No captured export is pending.")?
            .send(result.map(|outcome| (purpose, outcome)))
            .map_err(|error| error.to_string())
    }
}

/// Ask where to save, then publish the archive atomically: a complete temporary file in the
/// destination folder replaces the target only after it is written and synced.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn native(name: &str, bytes: &[u8], purpose: Purpose) -> Result<(Purpose, Outcome), String> {
    let Some(path) = rfd::FileDialog::new().set_title(purpose.title()).set_file_name(name).add_filter("Zip archive", &["zip"]).save_file() else {
        return Ok((purpose, Outcome::Cancelled));
    };
    publish_archive(&path, bytes)?;
    Ok((purpose, Outcome::Saved))
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

/// Suggest a portable archive name that records the explicit variant selection.
pub(super) fn assembly_archive_name(project: &str, variant: Option<&str>) -> String {
    let project = archive_stem(project);
    let variant = variant.map(archive_stem).unwrap_or_else(|| "base".into());
    format!("{project}-{variant}-assembly.zip")
}

/// Keep archive name components portable without deriving identity from their text.
fn archive_stem(text: &str) -> String {
    let cleaned: String = text.chars().take(80).map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    if cleaned.trim_matches('_').is_empty() { "board".into() } else { cleaned }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
