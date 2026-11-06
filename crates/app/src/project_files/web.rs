//! Browser import and download. Encoding yields between editor frames.

use std::sync::{Arc, mpsc::Sender};

use js_sys::{Promise, Uint8Array};
use pcbcraft_engine::{
    Project,
    persistence::{Documents, DocumentsEncoder, Encoding, MAX_ARCHIVE_BYTES, MAX_PROJECT_BYTES, PackedEncoder, decode_file, decode_packed},
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::{Outcome, Saved};

#[wasm_bindgen(inline_js = "
export function pickProject(jsonLimit, zipLimit) {
    return new Promise((resolve, reject) => {
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = '.pc,.zip,.json';
        input.oncancel = () => resolve(null);
        input.onchange = async () => {
            const file = input.files[0];
            if (!file) return resolve(null);
            try {
                if (file.size > zipLimit) throw new Error('Project ZIP exceeds the file size limit.');
                if (file.size > jsonLimit) {
                    const header = new DataView(await file.slice(0, 4).arrayBuffer()).getUint32(0);
                    if (![0x504b0304, 0x504b0506, 0x504b0708].includes(header)) {
                        throw new Error('Project JSON exceeds the file size limit.');
                    }
                }
                resolve(await file.arrayBuffer());
            } catch (error) { reject(error); }
        };
        input.click();
    });
}

let unsaved = false;
let warning = false;
export function setUnsaved(flag) {
    unsaved = flag;
    if (!warning) {
        warning = true;
        window.addEventListener('beforeunload', (event) => {
            if (unsaved) {
                event.preventDefault();
                event.returnValue = '';
            }
        });
    }
}

export function downloadProject(bytes, archive) {
    const url = URL.createObjectURL(new Blob([bytes], {type: archive ? 'application/zip' : 'application/json'}));
    const link = document.createElement('a');
    link.href = url;
    link.download = 'project.pc.zip';
    document.body.appendChild(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 30000);
}
")]
extern "C" {
    #[wasm_bindgen(js_name = pickProject)]
    fn pick_project(json_limit: u32, zip_limit: u32) -> Promise;

    /// Record whether leaving the page would lose unsaved changes, so the browser asks first.
    #[wasm_bindgen(js_name = setUnsaved)]
    pub(super) fn set_unsaved(unsaved: bool);

    #[wasm_bindgen(catch, js_name = downloadProject)]
    fn download_project(bytes: &[u8], archive: bool) -> Result<(), JsValue>;
}

/// Open the picker during the triggering interaction to retain browser permission.
pub(super) fn pick_file() -> Promise {
    pick_project(MAX_PROJECT_BYTES as u32, MAX_ARCHIVE_BYTES as u32)
}

/// Validate the bounded picked payload before returning it to the editor.
pub(super) async fn open(file: Promise) -> Result<Outcome, String> {
    let data = JsFuture::from(file).await.map_err(|error| format!("Could not open project: {}", error_message(&error)))?;
    if data.is_null() {
        return Ok(Outcome::Cancelled);
    }
    let bytes = Uint8Array::new(&data);
    if bytes.length() as usize > MAX_ARCHIVE_BYTES {
        return Err("project exceeds the native file size limit".to_owned());
    }
    let bytes: Arc<[u8]> = bytes.to_vec().into();
    let encoding = Encoding::detect(&bytes);
    let (snapshot, documents, needs_manifest, legacy) = if encoding == Encoding::Zip {
        let packed = decode_packed(&bytes)?;
        (packed.project, packed.documents.map(Arc::new), packed.needs_manifest, packed.legacy)
    } else {
        (decode_file(&bytes)?.0, None, false, true)
    };
    Ok(Outcome::Opened(Saved { snapshot, bytes, encoding, needs_manifest, legacy, documents }))
}

pub(super) struct Download {
    source: DownloadSource,
    result: Sender<Result<Outcome, String>>,
}

enum DownloadSource {
    Encode(DocumentsEncoder),
    Archive(PackedEncoder),
    Unchanged(Arc<[u8]>),
    Documents(Arc<Documents>),
}

impl std::fmt::Debug for Download {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Download").finish_non_exhaustive()
    }
}

impl Download {
    /// Prepare a download from a retained revision without encoding it in the input handler.
    pub(super) fn new(
        snapshot: Arc<Project>,
        unchanged: Option<Arc<[u8]>>,
        documents: Option<Arc<Documents>>,
        result: Sender<Result<Outcome, String>>,
    ) -> Result<Self, String> {
        let source = match unchanged {
            Some(bytes) => DownloadSource::Unchanged(bytes),
            None => documents.map(DownloadSource::Documents).unwrap_or_else(|| DownloadSource::Encode(DocumentsEncoder::new(snapshot))),
        };
        Ok(Self { source, result })
    }

    /// Encode a bounded batch or download the unchanged bytes. A request cannot confirm a write.
    pub(super) fn advance(&mut self) {
        let result = match &mut self.source {
            DownloadSource::Documents(documents) => match PackedEncoder::new((**documents).clone()) {
                Ok(archive) => {
                    self.source = DownloadSource::Archive(archive);
                    return;
                }
                Err(error) => Err(error),
            },
            DownloadSource::Unchanged(bytes) => send_download(bytes),
            DownloadSource::Encode(encoder) => match encoder.step(256) {
                Ok(Some(documents)) => match PackedEncoder::new(documents) {
                    Ok(archive) => {
                        self.source = DownloadSource::Archive(archive);
                        return;
                    }
                    Err(error) => Err(error),
                },
                Ok(None) => return,
                Err(error) => Err(error),
            },
            DownloadSource::Archive(encoder) => match encoder.step() {
                Ok(Some(bytes)) => send_download(&bytes),
                Ok(None) => return,
                Err(error) => Err(error),
            },
        };
        let _ = self.result.send(result);
    }
}

/// Hand the completed payload to the browser's download UI.
fn send_download(bytes: &[u8]) -> Result<Outcome, String> {
    download_project(bytes, true).map(|()| Outcome::Downloaded).map_err(|error| format!("Could not download project: {}", error_message(&error)))
}

/// Show the browser's error message or string rejection in the status bar.
fn error_message(error: &JsValue) -> String {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| error.as_string())
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "Unknown browser error".to_owned())
}
