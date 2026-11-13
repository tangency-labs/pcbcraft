//! Browser canvas readback after the frame callback submits its work, without relying on renderer-specific
//! screenshot mapping. The next input waits until the canvas copy completes.
use super::{Result, clock_ms, digest};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(
    inline_js = "export async function read_replay_canvas(canvas) { await Promise.resolve(); const copy = document.createElement('canvas'); copy.width = canvas.width; copy.height = canvas.height; const ctx = copy.getContext('2d', {willReadFrequently:true}); if (!ctx) throw new Error('Canvas readback context unavailable'); ctx.drawImage(canvas, 0, 0); return ctx.getImageData(0, 0, copy.width, copy.height).data; }"
)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn read_replay_canvas(canvas: web_sys::HtmlCanvasElement) -> std::result::Result<JsValue, JsValue>;
}

/// Copy the submitted canvas before the browser can discard its drawing buffer and retain a digest plus nonblank/dimension evidence. All
/// browser readback overhead is included; this cannot observe compositor presentation.
pub(super) async fn capture(canvas: web_sys::HtmlCanvasElement) -> Result<super::rendered::Capture> {
    let size = [canvas.width() as usize, canvas.height() as usize];
    let pixels = read_replay_canvas(canvas).await.map_err(|e| format!("canvas readback failed: {e:?}"))?;
    let bytes = js_sys::Uint8Array::new(&pixels).to_vec();
    let at_ms = clock_ms();
    if bytes.len() != size[0] * size[1] * 4 {
        return Err("canvas readback size changed".into());
    }
    let varied = bytes.as_chunks::<4>().0.first().is_some_and(|first| bytes.as_chunks::<4>().0.iter().any(|pixel| pixel != first));
    Ok(super::rendered::Capture { at_ms, size, image_sha256: digest(&bytes), varied })
}
