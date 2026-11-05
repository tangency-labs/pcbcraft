//! Browser import and download. Encoding yields between editor frames.

use std::sync::{Arc, mpsc::Sender};

use js_sys::{Promise, Uint8Array};
use pcbcraft_engine::{
    Project,
    persistence::{ArchiveEncoder, Encoder, Encoding, MAX_ARCHIVE_BYTES, MAX_PROJECT_BYTES, decode_file},
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::{Outcome, Saved};

#[wasm_bindgen(inline_js = "
export function pickProject(jsonLimit, zipLimit) {
    return new Promise((resolve, reject) => {
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = '.pc,.json';
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

export function downloadProject(bytes, archive) {
    const url = URL.createObjectURL(new Blob([bytes], {type: archive ? 'application/zip' : 'application/json'}));
    const link = document.createElement('a');
    link.href = url;
    link.download = 'project.pc';
    document.body.appendChild(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 30000);
}
")]
extern "C" {
    #[wasm_bindgen(js_name = pickProject)]
    fn pick_project(json_limit: u32, zip_limit: u32) -> Promise;

    #[wasm_bindgen(catch, js_name = downloadProject)]
    fn download_project(bytes: &[u8], archive: bool) -> Result<(), JsValue>;
}

/// Open the picker during the triggering interaction to retain browser permission.
pub(super) fn pick_file() -> Promise {
    pick_project(MAX_PROJECT_BYTES as u32, MAX_ARCHIVE_BYTES as u32)
}

/// Validate the bounded picked payload before returning it to the editor.
pub(super) async fn open(file: Promise) -> Result<Outcome, String> {
    let data = JsFuture::from(file).await.map_err(|error| format!("Could not open project: {error:?}"))?;
    if data.is_null() {
        return Ok(Outcome::Cancelled);
    }
    let bytes = Uint8Array::new(&data);
    if bytes.length() as usize > MAX_ARCHIVE_BYTES {
        return Err("project exceeds the native file size limit".to_owned());
    }
    let bytes: Arc<[u8]> = bytes.to_vec().into();
    let (snapshot, encoding) = decode_file(&bytes)?;
    Ok(Outcome::Opened(Saved { snapshot, bytes, encoding }))
}

pub(super) struct Download {
    source: DownloadSource,
    encoding: Encoding,
    result: Sender<Result<Outcome, String>>,
}

enum DownloadSource {
    Encode(Encoder),
    Archive(ArchiveEncoder),
    Unchanged(Arc<[u8]>),
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
        encoding: Encoding,
        result: Sender<Result<Outcome, String>>,
    ) -> Result<Self, String> {
        let source = match unchanged {
            Some(bytes) => DownloadSource::Unchanged(bytes),
            None => DownloadSource::Encode(Encoder::new(snapshot)?),
        };
        Ok(Self { source, encoding, result })
    }

    /// Encode a bounded batch or download the unchanged bytes. A request cannot confirm a write.
    pub(super) fn advance(&mut self) {
        let result = match &mut self.source {
            DownloadSource::Unchanged(bytes) => send_download(bytes, self.encoding),
            DownloadSource::Encode(encoder) => match encoder.step(256) {
                Ok(Some(bytes)) if self.encoding == Encoding::Zip => match ArchiveEncoder::new(bytes) {
                    Ok(archive) => {
                        self.source = DownloadSource::Archive(archive);
                        return;
                    }
                    Err(error) => Err(error),
                },
                Ok(Some(bytes)) => send_download(&bytes, self.encoding),
                Ok(None) => return,
                Err(error) => Err(error),
            },
            DownloadSource::Archive(encoder) => match encoder.step() {
                Ok(Some(bytes)) => send_download(&bytes, self.encoding),
                Ok(None) => return,
                Err(error) => Err(error),
            },
        };
        let _ = self.result.send(result);
    }
}

/// Hand the completed payload to the browser's download UI.
fn send_download(bytes: &[u8], encoding: Encoding) -> Result<Outcome, String> {
    download_project(bytes, encoding == Encoding::Zip).map(|()| Outcome::Downloaded).map_err(|error| format!("Could not download project: {error:?}"))
}
