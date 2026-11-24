//! A pending file operation belongs to the view that requested it.
use super::Opened;
use pcbcraft_engine::board::fabrication::review::{MAX_ZIP_BYTES, Package};
use std::sync::{
    Arc, Weak,
    mpsc::{self, Receiver, TryRecvError},
};

type Completion = Result<Option<Arc<Opened>>, String>;

#[derive(Debug)]
struct Job {
    owner: crate::ViewId,
    binding: Weak<()>,
    result: Receiver<Completion>,
}

/// Keep the picker slot until the worker finishes, even if its pane is closed.
#[derive(Debug, Default)]
pub(crate) struct Files(Option<Job>);

impl Files {
    /// Only one picker or read can be active at a time.
    pub(super) fn busy(&self) -> bool {
        self.0.is_some()
    }

    /// Report work only for the pane binding that requested it.
    pub(super) fn reading(&self, owner: crate::ViewId, binding: &Arc<()>) -> bool {
        self.0.as_ref().is_some_and(|job| job.owner == owner && Weak::ptr_eq(&job.binding, &Arc::downgrade(binding)))
    }

    /// One worker owns the picker, bounded read, and parser. A finished job never waits for UI.
    pub(super) fn open(
        &mut self,
        ctx: &egui::Context,
        owner: crate::ViewId,
        binding: &Arc<()>,
        path: Option<std::path::PathBuf>,
        directory: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("Finish opening the current fabrication package first.".into());
        }
        let (send, result) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("fabrication-review".into())
            .spawn(move || {
                let _ = send.send(native(path, directory.as_deref()));
                ctx.request_repaint();
            })
            .map_err(|error| format!("Could not start opening the fabrication package: {error}"))?;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, directory);
            let file = pick_zip(MAX_ZIP_BYTES as u32);
            wasm_bindgen_futures::spawn_local(async move {
                let _ = send.send(browser(file).await);
                ctx.request_repaint();
            });
        }
        self.0 = Some(Job { owner, binding: Arc::downgrade(binding), result });
        Ok(())
    }

    /// Deliver at most one result. A switched, closed or replaced pane cannot adopt it.
    pub(super) fn poll(&mut self) -> Option<(crate::ViewId, Arc<()>, Completion)> {
        let job = self.0.as_ref()?;
        let completion = match job.result.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("Opening the fabrication package stopped before it completed.".into()),
        };
        let job = self.0.take()?;
        Some((job.owner, job.binding.upgrade()?, completion))
    }
}

/// Native paths, supplied by the picker or a live command, go through the same bounded reader.
#[cfg(not(target_arch = "wasm32"))]
fn native(path: Option<std::path::PathBuf>, directory: Option<&std::path::Path>) -> Completion {
    use std::io::Read;
    let Some(path) = path.or_else(|| {
        let mut dialog = rfd::FileDialog::new().set_title("Open fabrication ZIP").add_filter("Fabrication ZIP", &["zip"]);
        if let Some(directory) = directory {
            dialog = dialog.set_directory(directory);
        }
        dialog.pick_file()
    }) else {
        return Ok(None);
    };
    if !std::fs::metadata(&path).map_err(|error| error.to_string())?.is_file() {
        return Err("Choose a regular fabrication ZIP file.".into());
    }
    let file = open_zip_file(&path)?;
    let name = path.file_name().map_or_else(|| "Fabrication ZIP".into(), |name| name.to_string_lossy().into_owned());
    let mut bytes = Vec::new();
    file.take(MAX_ZIP_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    load(name, &bytes)
}

/// Check the opened descriptor before reading. On Unix, a path replaced with a FIFO cannot
/// block the worker while it opens the descriptor.
#[cfg(not(target_arch = "wasm32"))]
fn open_zip_file(path: &std::path::Path) -> Result<std::fs::File, String> {
    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags, open};
        open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map(std::fs::File::from)
    };
    #[cfg(not(unix))]
    let file = std::fs::File::open(path);
    let file = file.map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_ZIP_BYTES as u64 {
        return Err("Choose a regular fabrication ZIP file up to 64 MiB.".into());
    }
    Ok(file)
}

/// The reader validates the archive and builds the plot from its actual member bytes.
fn load(name: String, bytes: &[u8]) -> Completion {
    let package = Package::read(bytes).map_err(|error| format!("{name}: {error}"))?;
    Ok(Some(Arc::new(Opened { name, package })))
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = "
export function pickFabricationZip(limit) {
    return new Promise((resolve, reject) => {
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = '.zip';
        input.oncancel = () => resolve(null);
        input.onchange = async () => {
            const file = input.files[0];
            if (!file) return resolve(null);
            try {
                if (file.size > limit) throw new Error('Fabrication ZIP exceeds 64 MiB.');
                resolve([file.name, await file.arrayBuffer()]);
            } catch (error) { reject(error); }
        };
        input.click();
    });
}
")]
extern "C" {
    #[wasm_bindgen(js_name = pickFabricationZip)]
    fn pick_zip(limit: u32) -> js_sys::Promise;
}

/// Start the chooser during the browser gesture and bound the selected bytes before parsing.
#[cfg(target_arch = "wasm32")]
async fn browser(file: js_sys::Promise) -> Completion {
    let result = wasm_bindgen_futures::JsFuture::from(file).await.map_err(|error| format!("Could not open fabrication ZIP: {error:?}"))?;
    if result.is_null() {
        return Ok(None);
    }
    let file = js_sys::Array::from(&result);
    let bytes = js_sys::Uint8Array::new(&file.get(1));
    if bytes.length() as usize > MAX_ZIP_BYTES {
        return Err("Fabrication ZIP exceeds 64 MiB.".into());
    }
    load(file.get(0).as_string().unwrap_or_else(|| "Fabrication ZIP".into()), &bytes.to_vec())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::open_zip_file;
    use rustix::fs::{CWD, Mode, mkfifoat};
    use std::{sync::mpsc, time::Duration};

    /// Exercise the descriptor check when the path was replaced after the initial type check.
    #[test]
    fn manufacturing_fifo_open_returns_without_a_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fabrication.zip");
        mkfifoat(CWD, &path, Mode::RUSR | Mode::WUSR).unwrap();
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || send.send(open_zip_file(&path)).unwrap());
        let error = receive.recv_timeout(Duration::from_secs(5)).expect("opening a FIFO must not wait for a writer").unwrap_err();
        assert!(error.contains("Choose a regular fabrication ZIP file"), "{error}");
    }
}
