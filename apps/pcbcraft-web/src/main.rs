//! PCBCraft in the browser: the same app as the desktop, through eframe's web runner (WebGPU where
//! the browser has it, WebGL2 otherwise). Build with `trunk build --release` from this directory.
//! The output goes to `dist/web`.

#[cfg(target_arch = "wasm32")]
fn main() {
    // The same module is imported by an isolated fill worker; it must not bootstrap a UI there.
    if web_sys::window().is_none() {
        return;
    }
    use eframe::wasm_bindgen::JsCast as _;

    const CANVAS_ID: &str = "pcbcraft_canvas";
    const LOADING_ID: &str = "pcbcraft_loading";

    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let started =
            eframe::WebRunner::new().start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(pcbcraft_app::PcbApp::new(cc))))).await;
        if let Some(loading) = document.get_element_by_id(LOADING_ID) {
            match started {
                Ok(()) => loading.remove(),
                Err(e) => {
                    log::error!("failed to start: {e:?}");
                    loading.set_text_content(Some("PCBCraft could not start in this browser. It needs WebGPU or WebGL2."));
                }
            }
        }
    });
}

/// Forward completed-zone counts while calculation stays in the isolated worker.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn pcbcraft_fill_worker(request: &str, progress: &js_sys::Function) -> String {
    pcbcraft_engine::board::fill::worker::execute_reporting(request, &mut |completed| {
        let _ = progress.call1(&wasm_bindgen::JsValue::NULL, &wasm_bindgen::JsValue::from(completed as u32));
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("pcbcraft-web only runs in a browser. Build it with `trunk build --release` from apps/pcbcraft-web.");
}
