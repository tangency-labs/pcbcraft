//! Measured native window size and requested client sizes for reproducible recordings.
use std::time::{Duration, Instant};

use egui::Vec2;
use serde::Deserialize;
use serde_json::{Value, json};

use super::SCHEMA;
use crate::{PcbApp, editor_layout::HostId, editor_windows::viewport};

/// How long a size request may remain unmatched before it is reported as constrained.
pub(super) const RESIZE_SETTLE: Duration = Duration::from_secs(2);

/// A native window's client size and state as reported for its last accepted paint.
/// The state flags are `None` where the platform does not report them.
#[derive(Clone, Debug)]
pub(super) struct Measured {
    client: Vec2,
    maximized: Option<bool>,
    fullscreen: Option<bool>,
    minimized: Option<bool>,
    monitor: Option<Vec2>,
}

/// A requested client size and what the window did with it: `pending` until a paint
/// measures the requested size (`applied`) or the settle time passes (`constrained`).
#[derive(Debug)]
pub(super) struct Resize {
    requested: Vec2,
    pub(super) since: Instant,
    pub(super) state: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowQuery {
    window: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeRequest {
    window: u64,
    width: f32,
    height: f32,
}

impl super::Automation {
    /// Whether `host` waits for a requested size; its geometry is stale until then.
    pub(super) fn resize_pending(&self, host: HostId) -> bool {
        self.resizes.get(&host).is_some_and(|resize| resize.state == "pending")
    }
}

impl PcbApp {
    /// Measure this host's window for its accepted paint, and settle a pending size request:
    /// `applied` once the client matches it to a physical pixel, else `constrained` after
    /// the settle time. Repaints continue while it is pending.
    pub(super) fn automation_measure(&mut self, ctx: &egui::Context, host: HostId) -> Measured {
        // The pinned egui-winit reads macOS maximized/minimized only when the window opens,
        // so later values there are unknown rather than measured.
        let current = !cfg!(target_os = "macos");
        let window = ctx.input(|input| {
            let info = input.viewport();
            Measured {
                client: input.viewport_rect().size(),
                maximized: info.maximized.filter(|_| current),
                fullscreen: info.fullscreen,
                minimized: info.minimized.filter(|_| current),
                monitor: info.monitor_size,
            }
        });
        if let Some(resize) = self.automation.resizes.get_mut(&host).filter(|resize| resize.state == "pending") {
            // Native sizes are whole physical pixels, rounded from logical points as winit does.
            let physical = |size: Vec2| (size * ctx.pixels_per_point()).round();
            if physical(window.client) == physical(resize.requested) {
                resize.state = "applied";
            } else if resize.since.elapsed() >= RESIZE_SETTLE {
                resize.state = "constrained";
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
        self.automation.resizes.retain(|host, _| self.views.hosts.contains_key(host));
        window
    }

    /// `ui.window` reports a native window's measured client size and state from its last
    /// accepted paint, with the latest size request kept separately so a request never reads
    /// as a measurement. `ui.resize` requests a new client size.
    pub(super) fn automation_window(&mut self, ctx: &egui::Context, id: &str, params: &Value) -> Result<Value, String> {
        if id == "ui.resize" {
            return self.automation_resize(ctx, params);
        }
        let query: WindowQuery = serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
        let host = HostId(query.window);
        if !self.views.hosts.contains_key(&host) {
            return Err("Unknown editor window".into());
        }
        self.repaint_editors(ctx);
        let snapshot = self.automation.snapshots.get(&host).ok_or("UI geometry is not ready; wait for a paint and retry")?;
        let window = &snapshot.window;
        let size = |size: Vec2| json!({"width":size.x,"height":size.y});
        let resize = self.automation.resizes.get(&host).map(|resize| json!({"state":resize.state,"requested":size(resize.requested)}));
        Ok(json!({"schema":SCHEMA,"window":host.0,"layout":snapshot.layout,"client":size(window.client),"scale":snapshot.scale,
            "maximized":window.maximized,"fullscreen":window.fullscreen,"minimized":window.minimized,"focused":snapshot.focused,
            "monitor":window.monitor.map(size),"resize":resize}))
    }

    /// Ask the existing native host for a client size in logical points. Sizes below the
    /// window's minimum, beyond its monitor, or for a maximized, fullscreen, minimized or
    /// embedded window are refused unchanged. An accepted request cancels queued input,
    /// retires the current layout token and holds new input until it is applied or constrained.
    fn automation_resize(&mut self, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
        let request: SizeRequest = serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
        let host = HostId(request.window);
        if !self.views.hosts.contains_key(&host) {
            return Err("Unknown editor window".into());
        }
        if host != HostId::ROOT && ctx.embed_viewports() {
            return Err("Detached editors are embedded in the main window on this platform and have no native size".into());
        }
        let snapshot = self.automation.snapshots.get(&host).ok_or("UI geometry is not ready; wait for a paint and retry")?;
        let window = &snapshot.window;
        if [window.maximized, window.fullscreen, window.minimized].contains(&Some(true)) {
            return Err("Restore the window from maximized, fullscreen or minimized before resizing it".into());
        }
        let size = egui::vec2(request.width, request.height);
        let min = if host == HostId::ROOT { crate::MAIN_WINDOW_MIN_SIZE } else { crate::editor_windows::DETACHED_WINDOW_MIN_SIZE };
        if !(size.x >= min[0] && size.y >= min[1]) {
            return Err(format!("This window's client size must be at least {}×{} logical points", min[0], min[1]));
        }
        // Without a reported monitor, the largest texture side bounds the request.
        let max = window.monitor.unwrap_or(egui::vec2(16384.0, 16384.0)).min(egui::vec2(16384.0, 16384.0));
        if size.x > max.x || size.y > max.y {
            return Err(format!("The requested size exceeds the {}×{} logical-point display", max.x, max.y));
        }
        self.automation_cancel("Window resize requested");
        self.automation.next_layout = self.automation.next_layout.saturating_add(1);
        if let Some(snapshot) = self.automation.snapshots.get_mut(&host) {
            snapshot.layout = self.automation.next_layout;
        }
        self.automation.resizes.insert(host, Resize { requested: size, since: Instant::now(), state: "pending" });
        ctx.send_viewport_cmd_to(viewport(host), egui::ViewportCommand::InnerSize(size));
        ctx.request_repaint_of(viewport(host));
        Ok(json!({"schema":SCHEMA,"window":host.0,"state":"pending","requested":{"width":size.x,"height":size.y}}))
    }
}
