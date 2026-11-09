//! Fabrication output export: one archive of Gerber and drill files, saved off the editor loop.
//!
//! The archive bytes are built from the current snapshot on the UI thread, which is quick and
//! deterministic; only the save dialog and filesystem write run on a worker. One export runs at
//! a time. Dropping the application never joins the worker, and a worker that stops reports a
//! disconnect instead of leaving the slot busy forever.
#[cfg(not(test))]
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, TryRecvError};

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
        #[cfg(test)]
        {
            let _ = ctx;
            self.captured = Some((name, bytes));
            Ok(())
        }
        #[cfg(not(test))]
        {
            let (send, receive) = mpsc::sync_channel(1);
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
            self.pending = Some(receive);
            Ok(())
        }
    }
}

/// Ask where to save, then publish the archive atomically: a complete temporary file in the
/// destination folder replaces the target only after it is written and synced.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn native(name: &str, bytes: &[u8]) -> Result<Outcome, String> {
    use std::io::Write;
    let Some(path) = rfd::FileDialog::new().set_title("Save fabrication outputs").set_file_name(name).add_filter("Zip archive", &["zip"]).save_file()
    else {
        return Ok(Outcome::Cancelled);
    };
    let parent = path.parent().ok_or("Choose a destination folder.")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).and_then(|_| file.as_file().sync_all()).map_err(|e| e.to_string())?;
    file.persist(&path).map_err(|e| e.error.to_string())?;
    Ok(Outcome::Saved)
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
