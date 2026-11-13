//! Browser CPU diagnostics using the same artifacts, engine and editor replay as native.

/// Run the shared harness in an actual browser Window. This synchronous diagnostic deliberately
/// reports CPU work; its return does not signal a rendered or presented frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run_performance(artifact: &str, environment: &str, iterations: u32) -> Result<String, wasm_bindgen::JsValue> {
    let execute = || {
        let environment = serde_json::from_str(environment).map_err(|e| e.to_string())?;
        let report = pcbcraft_app::performance::run(artifact, environment, iterations as usize)?;
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
    };
    execute().map_err(|error: String| wasm_bindgen::JsValue::from_str(&error))
}

/// Start an asynchronous GPU-backed replay in its own canvas. Retain the returned handle
/// until a report or error arrives, then destroy it to release browser resources.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn start_rendered(
    canvas: web_sys::HtmlCanvasElement,
    artifact: String,
    environment: String,
    iterations: u32,
    backend: String,
) -> Result<RenderedHandle, wasm_bindgen::JsValue> {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let environment = serde_json::from_str(&environment).map_err(|e| wasm_bindgen::JsValue::from_str(&format!("invalid environment: {e}")))?;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let runner = eframe::WebRunner::new();
    let liveness = pcbcraft_app::performance::rendered::Liveness::default();
    let app_liveness = liveness.clone();
    let mut options = eframe::WebOptions::default();
    match backend.as_str() {
        "default" => {}
        "gl" => {
            if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
                setup.instance_descriptor.backends = eframe::wgpu::Backends::GL;
            }
        }
        _ => return Err(wasm_bindgen::JsValue::from_str("backend must be default or gl")),
    }
    runner
        .start(
            canvas.clone(),
            options,
            Box::new(move |cc| {
                let renderer =
                    cc.wgpu_render_state.as_ref().map(|state| format!("{:?}", state.adapter.get_info())).unwrap_or_else(|| "unknown renderer".into());
                Ok(Box::new(
                    pcbcraft_app::performance::rendered::RenderedApp::new(
                        &cc.egui_ctx,
                        &artifact,
                        environment,
                        iterations as usize,
                        std::time::Duration::from_secs(30),
                        send,
                        renderer,
                    )?
                    .with_canvas_capture(canvas.clone())
                    .with_liveness(app_liveness),
                ))
            }),
        )
        .await
        .inspect_err(|_| runner.destroy())?;
    Ok(RenderedHandle { runner, receive, liveness })
}

/// Own the browser runner and its single bounded result, without polling on the UI thread.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub struct RenderedHandle {
    runner: eframe::WebRunner,
    receive: std::sync::mpsc::Receiver<pcbcraft_app::performance::rendered::Completion>,
    liveness: pcbcraft_app::performance::rendered::Liveness,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
impl RenderedHandle {
    /// Return the complete report, null while running, or a visible failure. Partial runs
    /// never become baseline reports, including callback failure and runner shutdown.
    pub fn poll(&self) -> Result<Option<String>, wasm_bindgen::JsValue> {
        match self.receive.try_recv() {
            Ok(Ok(report)) => serde_json::to_string_pretty(&report).map(Some).map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string())),
            Ok(Err(error)) => Err(wasm_bindgen::JsValue::from_str(&error)),
            Err(std::sync::mpsc::TryRecvError::Empty) if self.liveness.expired() => {
                self.runner.destroy();
                Err(wasm_bindgen::JsValue::from_str("rendered runner made no UI progress before deadline"))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(wasm_bindgen::JsValue::from_str("rendered runner disconnected")),
        }
    }

    /// Release animation callbacks, textures and event listeners after success or cancellation.
    pub fn destroy(&self) {
        self.runner.destroy();
    }
}
