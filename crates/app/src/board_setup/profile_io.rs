//! One application-owned profile picker/worker survives form dismissal without transferring results.
use pcbcraft_engine::board::setup::profile::{MAX_PROFILE_BYTES, SetupProfile};
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Debug)]
pub(super) enum Outcome {
    Imported(Box<SetupProfile>),
    Exported,
    Cancelled,
}
#[derive(Debug, Default)]
pub(crate) struct Files {
    pending: Option<(u64, Receiver<Result<Outcome, String>>)>,
    next: u64,
}
impl Files {
    /// A live picker/worker retains the only file slot across form dismissals.
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    /// Release a completed/disconnected slot without waiting. The ticket prevents a reopened form
    /// from adopting an old import; callers discard results whose captured owner is gone.
    pub(super) fn poll(&mut self) -> Option<(u64, Result<Outcome, String>)> {
        let (ticket, receive) = self.pending.as_ref()?;
        let result = match receive.try_recv() {
            Ok(value) => value,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("Profile file worker stopped before reporting its result.".into()),
        };
        let ticket = *ticket;
        self.pending = None;
        Some((ticket, result))
    }
    /// Capture immutable export bytes or start a bounded import. One pending job blocks further
    /// starts even after its form closes; dropping the application never joins blocking I/O.
    pub(super) fn start(&mut self, ctx: &egui::Context, save: Option<(String, Vec<u8>)>) -> Result<u64, String> {
        if self.busy() {
            return Err("Finish the current profile file operation first.".into());
        }
        let ticket = self.next.checked_add(1).ok_or("Profile request limit reached")?;
        let (send, receive) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("board-setup-profile".into())
            .spawn(move || {
                let result = native(save);
                let _ = send.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        #[cfg(target_arch = "wasm32")]
        {
            let picker = save.is_none().then(pick_profile);
            wasm_bindgen_futures::spawn_local(async move {
                let result = web(save, picker).await;
                let _ = send.send(result);
                ctx.request_repaint();
            });
        }
        self.next = ticket;
        self.pending = Some((ticket, receive));
        Ok(ticket)
    }
    /// Replace the file slot with a controlled completion channel for lifecycle regressions.
    #[cfg(test)]
    pub(super) fn stand_in(&mut self) -> (u64, mpsc::SyncSender<Result<Outcome, String>>) {
        let (send, receive) = mpsc::sync_channel(1);
        self.next += 1;
        self.pending = Some((self.next, receive));
        (self.next, send)
    }
}

/// Import regular files only and bound reads before decoding. No profile content is executed.
#[cfg(not(target_arch = "wasm32"))]
fn read(path: &std::path::Path) -> Result<SetupProfile, String> {
    use std::io::Read;
    if !std::fs::symlink_metadata(path).map_err(|e| e.to_string())?.file_type().is_file() {
        return Err("Choose a regular setup profile file.".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular setup profile file.".into());
    }
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_PROFILE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    SetupProfile::decode(&bytes)
}

/// Publish a complete new file atomically; existing or racing destinations remain untouched.
#[cfg(not(target_arch = "wasm32"))]
fn publish(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Choose a destination directory.")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).and_then(|_| file.as_file().sync_all()).map_err(|e| e.to_string())?;
    file.persist_noclobber(path).map_err(|e| format!("Choose a new filename; the profile was not overwritten: {}", e.error))?;
    #[cfg(unix)]
    std::fs::File::open(parent).and_then(|file| file.sync_all()).map_err(|e| format!("Profile was published but directory sync failed: {e}"))?;
    Ok(())
}

/// Native pickers and filesystem work run off the editor loop, owning only captured bytes.
#[cfg(not(target_arch = "wasm32"))]
fn native(save: Option<(String, Vec<u8>)>) -> Result<Outcome, String> {
    if let Some((name, bytes)) = save {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export setup profile to a new file")
            .set_file_name(name)
            .add_filter("PCBcraft setup profile", &["json"])
            .save_file()
        else {
            return Ok(Outcome::Cancelled);
        };
        publish(&path, &bytes)?;
        Ok(Outcome::Exported)
    } else {
        let Some(path) = rfd::FileDialog::new().set_title("Import setup profile").add_filter("PCBcraft setup profile", &["json"]).pick_file() else {
            return Ok(Outcome::Cancelled);
        };
        read(&path).map(|profile| Outcome::Imported(Box::new(profile)))
    }
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = "
export function pickSetupProfile(){return new Promise((resolve,reject)=>{const input=document.createElement('input');input.type='file';input.accept='.json';input.oncancel=()=>resolve(null);input.onchange=async()=>{try{const file=input.files[0];if(!file)return resolve(null);if(file.size>1048576)throw new Error('Setup profile exceeds 1 MiB.');resolve(await file.arrayBuffer());}catch(error){reject(error);}};input.click();});}
export function downloadSetupProfile(bytes,name){const url=URL.createObjectURL(new Blob([bytes],{type:'application/json'}));const a=document.createElement('a');a.href=url;a.download=name;document.body.appendChild(a);a.click();a.remove();setTimeout(()=>URL.revokeObjectURL(url),30000);}
")]
extern "C" {
    #[wasm_bindgen(js_name=pickSetupProfile)]
    fn pick_profile() -> js_sys::Promise;
    #[wasm_bindgen(catch,js_name=downloadSetupProfile)]
    fn download(bytes: &[u8], name: &str) -> Result<(), JsValue>;
}
/// Start the picker in the click's browser activation; imports validate before returning to a draft.
#[cfg(target_arch = "wasm32")]
async fn web(save: Option<(String, Vec<u8>)>, picker: Option<js_sys::Promise>) -> Result<Outcome, String> {
    if let Some((name, bytes)) = save {
        download(&bytes, &name).map_err(|e| format!("Profile download failed: {e:?}"))?;
        return Ok(Outcome::Exported);
    }
    let data = wasm_bindgen_futures::JsFuture::from(picker.ok_or("Profile picker was not started")?)
        .await
        .map_err(|e| format!("Profile import failed: {e:?}"))?;
    if data.is_null() {
        return Ok(Outcome::Cancelled);
    }
    let bytes = js_sys::Uint8Array::new(&data);
    if bytes.length() as usize > MAX_PROFILE_BYTES {
        return Err("Setup profile exceeds 1 MiB.".into());
    }
    SetupProfile::decode(&bytes.to_vec()).map(|profile| Outcome::Imported(Box::new(profile)))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    /// Publication cannot overwrite a shared profile or leave a partial visible destination.
    #[test]
    fn profile_export_is_atomic_and_never_clobbers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("starter.pcsetup.json");
        publish(&path, b"original").unwrap();
        assert!(publish(&path, b"replacement").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"original");
        assert!(read(dir.path()).is_err());
    }
}
