//! One bounded picker/write job owns its immutable snapshot until completion.
use pcbcraft_engine::authoring::{Item, MAX_BYTES};
use std::sync::{
    Arc,
    mpsc::{self, Receiver, TryRecvError},
};

#[derive(Debug)]
pub(super) enum Outcome {
    Opened(Box<Item>),
    #[cfg(not(target_arch = "wasm32"))]
    Saved(u64, Arc<Item>),
    Exported,
    Cancelled,
}
#[derive(Debug, Default)]
pub(super) struct Files {
    pending: Option<Receiver<Result<Outcome, String>>>,
}
impl Files {
    /// Supply one controlled result to exercise disconnect and overlapping-save ownership.
    #[cfg(test)]
    pub(super) fn stand_in(&mut self) -> mpsc::SyncSender<Result<Outcome, String>> {
        let (send, receive) = mpsc::sync_channel(1);
        self.pending = Some(receive);
        send
    }
    /// A pending picker or publication retains its owner and prevents shutdown or another job.
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    /// Poll without blocking; a lost worker releases the pending slot and reports failure.
    pub(super) fn poll(&mut self) -> Option<Result<Outcome, String>> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(value) => value,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("Library file worker stopped before reporting its result.".into()),
        };
        self.pending = None;
        Some(result)
    }
    /// Start one explicit file operation. Save-copy never overwrites an existing destination;
    /// native export and browser download do not mark the editable document durably saved.
    pub(super) fn start(&mut self, ctx: &egui::Context, save: Option<(u64, Arc<Item>, bool)>) -> Result<(), String> {
        if self.busy() {
            return Err("Finish the current library file operation first.".into());
        }
        let (send, receive) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("library-file".into())
            .spawn(move || {
                let result = native(save);
                let _ = send.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        #[cfg(target_arch = "wasm32")]
        {
            let picker = save.is_none().then(pick_item);
            wasm_bindgen_futures::spawn_local(async move {
                let result = web(save, picker).await;
                let _ = send.send(result);
                ctx.request_repaint();
            });
        }
        self.pending = Some(receive);
        Ok(())
    }
}

/// Use regular bounded input files; a FIFO or device must not block the worker indefinitely.
#[cfg(not(target_arch = "wasm32"))]
fn read(path: &std::path::Path) -> Result<Item, String> {
    use std::io::Read;
    if !std::fs::symlink_metadata(path).map_err(|e| e.to_string())?.file_type().is_file() {
        return Err("Choose a regular library item file.".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular library item file.".into());
    }
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Item::decode(&bytes)
}

/// Publish only to a new name. Atomic no-clobber publication protects source definitions and
/// unrelated existing files, including a destination created while the picker was open.
#[cfg(not(target_arch = "wasm32"))]
fn publish(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Choose a destination directory.")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(bytes).and_then(|_| temp.as_file().sync_all()).map_err(|e| e.to_string())?;
    temp.persist_noclobber(path).map_err(|e| format!("Could not publish library copy; choose a new filename: {}", e.error))?;
    // The visible file is complete even if directory sync is unavailable; don't claim a durable
    // save when that final confirmation fails. A retry must use a new name.
    #[cfg(unix)]
    std::fs::File::open(parent).and_then(|file| file.sync_all()).map_err(|e| format!("Copy was published but directory sync failed: {e}"))?;
    Ok(())
}

/// Pick on the worker, then validate or publish the captured bytes without touching project I/O.
#[cfg(not(target_arch = "wasm32"))]
fn native(save: Option<(u64, Arc<Item>, bool)>) -> Result<Outcome, String> {
    match save {
        None => match rfd::FileDialog::new().set_title("Open library item").add_filter("PCBcraft library item", &["json"]).pick_file() {
            Some(path) => read(&path).map(|item| Outcome::Opened(Box::new(item))),
            None => Ok(Outcome::Cancelled),
        },
        Some((id, item, export)) => {
            let bytes = if export {
                let mut bytes = serde_json::to_vec_pretty(&item.export_native()?).map_err(|e| e.to_string())?;
                bytes.push(b'\n');
                bytes
            } else {
                item.encode()?
            };
            let name = format!("{}-r{}{}.json", item.id, item.revision, if export { ".native" } else { ".pcitem" });
            let Some(path) = rfd::FileDialog::new()
                .set_title(if export { "Export native definition to a new file" } else { "Save library copy to a new file" })
                .set_file_name(name)
                .save_file()
            else {
                return Ok(Outcome::Cancelled);
            };
            publish(&path, &bytes)?;
            Ok(if export { Outcome::Exported } else { Outcome::Saved(id, item) })
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = "
export function pickLibraryItem(){return new Promise((resolve,reject)=>{const input=document.createElement('input');input.type='file';input.accept='.json';input.oncancel=()=>resolve(null);input.onchange=async()=>{try{const file=input.files[0];if(!file)return resolve(null);if(file.size>1048576)throw new Error('Library item exceeds 1 MiB.');resolve(await file.arrayBuffer());}catch(error){reject(error);}};input.click();});}
export function downloadLibraryItem(bytes,name){const url=URL.createObjectURL(new Blob([bytes],{type:'application/json'}));const a=document.createElement('a');a.href=url;a.download=name;document.body.appendChild(a);a.click();a.remove();setTimeout(()=>URL.revokeObjectURL(url),30000);}
")]
extern "C" {
    #[wasm_bindgen(js_name=pickLibraryItem)]
    fn pick_item() -> js_sys::Promise;
    #[wasm_bindgen(catch,js_name=downloadLibraryItem)]
    fn download(bytes: &[u8], name: &str) -> Result<(), JsValue>;
}
/// Browser downloads cannot confirm that a file was kept, so leave the document unsaved.
#[cfg(target_arch = "wasm32")]
async fn web(save: Option<(u64, Arc<Item>, bool)>, picker: Option<js_sys::Promise>) -> Result<Outcome, String> {
    if let Some((_, item, export)) = save {
        let bytes = if export { serde_json::to_vec_pretty(&item.export_native()?).map_err(|e| e.to_string())? } else { item.encode()? };
        download(&bytes, &format!("{}-r{}{}.json", item.id, item.revision, if export { ".native" } else { ".pcitem" }))
            .map_err(|e| format!("Library download failed: {e:?}"))?;
        return Ok(Outcome::Exported);
    }
    let picker = picker.ok_or("Library picker was not started.")?;
    let data = wasm_bindgen_futures::JsFuture::from(picker).await.map_err(|e| format!("Library open failed: {e:?}"))?;
    if data.is_null() {
        return Ok(Outcome::Cancelled);
    }
    let bytes = js_sys::Uint8Array::new(&data);
    if bytes.length() as usize > MAX_BYTES {
        return Err("Library item exceeds 1 MiB.".into());
    }
    Item::decode(&bytes.to_vec()).map(|item| Outcome::Opened(Box::new(item)))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    /// A racing or existing destination is untouched; successful output reopens deterministically.
    #[test]
    fn save_copy_never_overwrites_sources_or_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("item.pcitem.json");
        let item = Item::new(pcbcraft_engine::authoring::Kind::Footprint).unwrap();
        let bytes = item.encode().unwrap();
        publish(&path, &bytes).unwrap();
        assert_eq!(read(&path).unwrap(), item);
        assert!(publish(&path, b"different").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(publish(&dir.path().join("absent/item.json"), &bytes).is_err());
        assert!(read(dir.path()).is_err());
    }
}
