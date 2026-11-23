//! Disposable same-build browser workers. Traps, timeouts and cancellation retire an instance.
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
const workers = new Map();
let serial = 0;
export function zone_worker_start(payload) {
    if (workers.size >= 2) throw 'Zone fill worker capacity reached';
    const preload = [...document.querySelectorAll('link[rel="preload"][type="application/wasm"]')];
    const scripts = [...document.querySelectorAll('link[rel="modulepreload"]')];
    const wasm = preload.find(link => /_bg\.wasm$/.test(new URL(link.href).pathname));
    if (!wasm) throw 'Zone fill worker WASM entry is unavailable';
    const moduleUrl = wasm.href.replace(/_bg\.wasm$/, '.js');
    if (!scripts.some(link => link.href === moduleUrl)) throw 'Zone fill worker module does not match the packaged WASM entry';
    const code = `import init, {pcbcraft_fill_worker} from ${JSON.stringify(moduleUrl)};
        self.onmessage = async event => {
            try { await init({module_or_path: ${JSON.stringify(wasm.href)}});
                self.postMessage({ok:true, value:pcbcraft_fill_worker(event.data)});
            } catch (error) { self.postMessage({ok:false, value:String(error)}); }
        };`;
    const url = URL.createObjectURL(new Blob([code], {type:'text/javascript'}));
    let worker;
    try { worker = new Worker(url, {type:'module'}); }
    catch(error) { URL.revokeObjectURL(url); throw String(error); }
    const id = ++serial;
    const state = {worker,url,result:null,timer:null};
    workers.set(id,state);
    const finish = (ok,value) => {
        if (!workers.has(id) || state.result !== null) return;
        clearTimeout(state.timer);worker.terminate();URL.revokeObjectURL(url);
        state.result = (ok?'O':'E') + value;
    };
    worker.onmessage = event => finish(event.data.ok, event.data.value);
    worker.onerror = event => { event.preventDefault();finish(false,event.message || 'Zone fill worker failed'); };
    state.timer=setTimeout(()=>finish(false,'Zone fill worker timed out after 30 seconds'),30000);
    try { worker.postMessage(payload); } catch(error) { finish(false,String(error)); }
    return id;
}
export function zone_worker_poll(id) {
    const state=workers.get(id);
    if (!state) return 'EZone fill worker was retired';
    if (state.result === null) return null;
    workers.delete(id);return state.result;
}
export function zone_worker_cancel(id) {
    const state=workers.get(id);if(!state)return;
    clearTimeout(state.timer);state.worker.terminate();URL.revokeObjectURL(state.url);workers.delete(id);
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    fn zone_worker_start(payload: &str) -> Result<u32, JsValue>;
    fn zone_worker_poll(id: u32) -> Option<String>;
    fn zone_worker_cancel(id: u32);
}

/// Start only an isolated worker; inability to start is an error, never foreground fallback.
pub(super) fn start(payload: &str) -> Result<u32, String> {
    zone_worker_start(payload).map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
}
/// Pending work is nonblocking; every terminal result consumes and retires its worker handle.
pub(super) fn poll(id: u32) -> Option<Result<String, String>> {
    zone_worker_poll(id).map(|response| match response.strip_prefix('O') {
        Some(result) => Ok(result.to_owned()),
        None => Err(response.strip_prefix('E').unwrap_or(&response).to_owned()),
    })
}
/// Immediate host cancellation needs no acknowledgement or join from an executing kernel.
pub(super) fn cancel(id: u32) {
    zone_worker_cancel(id);
}
