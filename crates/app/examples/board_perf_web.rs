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
