//! Opt-in real-window replay. GPU readback is an observed completion boundary,
//! not compositor presentation or physical input-to-photon latency.

use super::{
    Artifact, Environment, Metric, Result, clock_ms, digest, load_artifact,
    replay::{Driver, WARMUP},
};
use crate::PcbApp;
#[cfg(not(target_arch = "wasm32"))]
use egui::ViewportCommand;
use egui::{RawInput, Ui};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    time::Duration,
};

/// Completion distinguishes complete evidence from failure; partial reports are never returned.
pub type Completion = std::result::Result<RenderedReport, String>;

/// Browser host polling can detect lost animation callbacks even when no app timer runs.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone)]
pub struct Liveness(std::rc::Rc<std::cell::Cell<f64>>);

#[cfg(any(target_arch = "wasm32", test))]
impl Default for Liveness {
    /// Allow 30 seconds for startup/accepted UI progress in the browser's monotonic clock.
    fn default() -> Self {
        Self(std::rc::Rc::new(std::cell::Cell::new(clock_ms() + 30_000.0)))
    }
}

#[cfg(any(target_arch = "wasm32", test))]
impl Liveness {
    /// Check without advancing the deadline; host polling cannot disguise an absent UI.
    pub fn expired(&self) -> bool {
        clock_ms() >= self.0.get()
    }

    /// Accepted progress or an explicit idle hold renews the same monotonic deadline.
    fn renew(&self, remaining_ms: f64) {
        self.0.set(clock_ms() + remaining_ms);
    }
}

/// Observed host visibility, before synthetic focus is applied to deterministic input.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WindowObservation {
    pub at_ms: f64,
    pub focused: bool,
    pub visible: Option<bool>,
    pub minimized: Option<bool>,
    pub occluded: Option<bool>,
}

/// Rendered evidence retains raw timings and explicitly identifies the measured clock/boundary.
#[derive(Debug, Serialize)]
pub struct RenderedReport {
    pub schema: String,
    pub environment: Environment,
    pub target: String,
    pub artifact_sha256: String,
    pub project_sha256: String,
    pub recipe: String,
    pub trace_id: String,
    pub input_policy: String,
    pub iterations: usize,
    pub warmup_frames: usize,
    pub idle_between_iterations_ms: f64,
    pub elapsed_ms: f64,
    pub debug_assertions: bool,
    pub renderer: String,
    pub window_observations: Vec<WindowObservation>,
    pub metrics: Vec<Metric>,
    pub captures: usize,
    pub image_size: [usize; 2],
    pub last_image_sha256: String,
    pub unmeasured: Vec<String>,
}

/// Timestamped completion from the renderer's one-shot GPU screenshot callback.
pub(super) struct Capture {
    pub(super) at_ms: f64,
    pub(super) size: [usize; 2],
    pub(super) image_sha256: String,
    pub(super) varied: bool,
}

/// One real editor with at most one injected action and screenshot in flight.
/// Dropping the runner disconnects its bounded callbacks; no callback waits for the UI.
pub struct RenderedApp {
    app: PcbApp,
    driver: Driver,
    artifact: Artifact,
    artifact_sha256: String,
    environment: Environment,
    iterations: usize,
    iteration: usize,
    step: usize,
    warmup: usize,
    next_input_ms: f64,
    run_started_ms: f64,
    idle_between_iterations_ms: f64,
    started_ms: f64,
    deadline_ms: f64,
    samples: BTreeMap<String, Vec<f64>>,
    active: Option<f64>,
    pending_input: Vec<egui::Event>,
    capture: Option<Receiver<Capture>>,
    completion: Option<SyncSender<Completion>>,
    captures: usize,
    size: Option<[usize; 2]>,
    last_image_sha256: String,
    renderer: String,
    observations: Vec<WindowObservation>,
    #[cfg(target_arch = "wasm32")]
    canvas: Option<web_sys::HtmlCanvasElement>,
    #[cfg(target_arch = "wasm32")]
    liveness: Option<Liveness>,
    #[cfg(not(target_arch = "wasm32"))]
    remote: Option<super::remote::RemoteEdit>,
    #[cfg(not(target_arch = "wasm32"))]
    device: Option<eframe::wgpu::Device>,
    #[cfg(not(target_arch = "wasm32"))]
    poll_stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    #[cfg(not(target_arch = "wasm32"))]
    poll_error: Option<Receiver<String>>,
    #[cfg(not(target_arch = "wasm32"))]
    watchdog: Option<super::watchdog::Watchdog>,
}

impl RenderedApp {
    /// Validate artifacts and metadata before opening a diagnostic window. The timeout is
    /// per screenshot, bounded to 1–120 seconds; a lost callback cannot produce a passing report.
    pub fn new(
        ctx: &egui::Context,
        artifact: &str,
        environment: Environment,
        iterations: usize,
        timeout: Duration,
        completion: SyncSender<Completion>,
        renderer: String,
    ) -> Result<Self> {
        environment.validate()?;
        if !(1..=10_000).contains(&iterations) || !(1.0..=120.0).contains(&timeout.as_secs_f64()) {
            return Err("iterations must be 1..=10000 and screenshot timeout 1..=120 seconds".into());
        }
        let loaded = load_artifact(artifact)?;
        if loaded.trace_id != super::replay::STANDARD_TRACE_ID {
            return Err("rendered replay requires the versioned standard trace".into());
        }
        if !loaded.trace.last().is_some_and(|step| step.capture_after()) {
            return Err("rendered trace must end with a capturable release, move or edit".into());
        }
        let (app, driver) = Self::prepare(ctx, &loaded)?;
        pcbcraft_chrome::install(ctx);
        // One input sample corresponds to one accepted UI pass. Avoid consuming a one-shot
        // capture in a discarded egui sizing pass; warmup establishes layout first.
        ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::MIN);
        Ok(Self {
            app,
            driver,
            artifact: loaded,
            artifact_sha256: digest(artifact.as_bytes()),
            environment,
            iterations,
            iteration: 0,
            step: 0,
            warmup: WARMUP,
            next_input_ms: 0.0,
            run_started_ms: clock_ms(),
            idle_between_iterations_ms: 0.0,
            started_ms: clock_ms(),
            deadline_ms: timeout.as_secs_f64() * 1000.0,
            samples: BTreeMap::new(),
            active: None,
            pending_input: Vec::new(),
            capture: None,
            completion: Some(completion),
            captures: 0,
            size: None,
            last_image_sha256: String::new(),
            renderer,
            observations: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            canvas: None,
            #[cfg(target_arch = "wasm32")]
            liveness: None,
            #[cfg(not(target_arch = "wasm32"))]
            remote: None,
            #[cfg(not(target_arch = "wasm32"))]
            device: None,
            #[cfg(not(target_arch = "wasm32"))]
            poll_stop: None,
            #[cfg(not(target_arch = "wasm32"))]
            poll_error: None,
            #[cfg(not(target_arch = "wasm32"))]
            watchdog: None,
        })
    }

    /// Retain the actual native renderer device for nonblocking screenshot-completion polling.
    /// Native wgpu mapping callbacks require polling; waiting for another UI submission would
    /// include arbitrary event-loop delay and can strand a request on an otherwise idle window.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_gpu_device(mut self, device: eframe::wgpu::Device) -> Self {
        self.device = Some(device);
        self
    }

    /// Monitor native UI progress independently of event-loop/frame callbacks. The example
    /// exits on expiry without writing partial evidence, including when in-app timers stop.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_watchdog(mut self, watchdog: super::watchdog::Watchdog) -> Self {
        self.watchdog = Some(watchdog);
        self
    }

    /// Poll only while one capture is outstanding. A bounded detached worker never waits on
    /// the UI, and cancellation or app drop releases its device clone within one poll interval.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_device(&mut self, ctx: egui::Context) -> Result<()> {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let device = self.device.clone().ok_or("native replay has no GPU device")?;
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        self.poll_stop = Some(stop);
        let (send, receive) = mpsc::sync_channel(1);
        self.poll_error = Some(receive);
        std::thread::Builder::new()
            .name("pcbcraft-perf-gpu-poll".into())
            .spawn(move || {
                let start = std::time::Instant::now();
                while !cancelled.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(120) {
                    if let Err(error) = device.poll(eframe::wgpu::PollType::Poll) {
                        let _ = send.try_send(format!("GPU completion polling failed: {error}"));
                        ctx.request_repaint();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Stop a capture's native polling without joining or waiting on graphics-driver work.
    fn stop_polling(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.poll_error = None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(stop) = self.poll_stop.take() {
            stop.store(true, std::sync::atomic::Ordering::Release);
        }
    }

    /// Hold the same process/window between complete traces for idle/exposure testing.
    /// Each hold is bounded to 24 hours; the report records the actual elapsed duration.
    pub fn with_idle_between_iterations(mut self, idle: Duration) -> Result<Self> {
        if idle.as_secs_f64() > 86_400.0 {
            return Err("idle interval must not exceed 24 hours".into());
        }
        self.idle_between_iterations_ms = idle.as_secs_f64() * 1000.0;
        Ok(self)
    }

    /// Use browser canvas readback after the current animation frame has submitted. This
    /// avoids backend-specific screenshot mapping and still fails on blank or missing pixels.
    #[cfg(target_arch = "wasm32")]
    pub fn with_canvas_capture(mut self, canvas: web_sys::HtmlCanvasElement) -> Self {
        self.canvas = Some(canvas);
        self
    }

    /// Share UI progress with the browser host, which can reject stalled animation callbacks
    /// from its polling timer instead of waiting forever for an in-app timeout.
    #[cfg(target_arch = "wasm32")]
    pub fn with_liveness(mut self, liveness: Liveness) -> Self {
        self.liveness = Some(liveness);
        self
    }

    /// Use an isolated opt-in endpoint only in this diagnostic window. Credentials stay in
    /// memory and the endpoint is dropped on iteration reset, failure or window shutdown.
    fn prepare(ctx: &egui::Context, artifact: &Artifact) -> Result<(PcbApp, Driver)> {
        let (app, driver) = Driver::prepare(artifact)?;
        #[cfg(not(target_arch = "wasm32"))]
        let mut app = app;
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Match the desktop launcher's Linux decoration/titlebar integration.
            app = app.with_integrated_titlebar(cfg!(target_os = "linux"));
            app.live_control = Some(crate::live_control::LiveControl::start(ctx).map_err(|e| e.to_string())?);
        }
        let _ = ctx;
        Ok((app, driver))
    }

    /// Complete once; errors and success both retire callbacks without blocking or publishing files.
    fn complete(&mut self, ctx: &egui::Context, result: Completion) {
        let _ = ctx;
        if let Some(send) = self.completion.take() {
            let _ = send.try_send(result);
        }
        self.stop_polling();
        self.capture = None;
        self.active = None;
        self.pending_input.clear();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.remote = None;
            self.app.live_control = None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    /// Require nonblank, stable-size rendered images, record callback arrival, and restore
    /// the immutable source for the next iteration rather than accumulating shared edits.
    fn poll_capture(&mut self, ctx: &egui::Context) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(error) = self.poll_error.as_ref().and_then(|receive| receive.try_recv().ok()) {
            return Err(error);
        }
        let Some(receiver) = &self.capture else { return Ok(()) };
        let image = match receiver.try_recv() {
            Ok(image) => image,
            Err(TryRecvError::Empty) => {
                if clock_ms() - self.started_ms > self.deadline_ms {
                    return Err("GPU screenshot callback timed out".into());
                }
                return Ok(());
            }
            Err(TryRecvError::Disconnected) => return Err("GPU screenshot callback disconnected".into()),
        };
        if image.size.contains(&0) || !image.varied || self.size.is_some_and(|size| size != image.size) {
            return Err("rendered capture is blank or changed dimensions during replay".into());
        }
        self.size = Some(image.size);
        self.last_image_sha256 = image.image_sha256;
        let id = &self.artifact.trace[self.step].id;
        self.samples.entry(format!("rendered.{id}.input_to_gpu_readback")).or_default().push(image.at_ms - self.started_ms);
        self.samples.entry(format!("rendered.{id}.readback_delivery")).or_default().push(clock_ms() - image.at_ms);
        self.captures += 1;
        self.stop_polling();
        self.capture = None;
        self.step += 1;
        if self.step == self.artifact.trace.len() {
            self.driver.finish(&self.app)?;
            self.iteration += 1;
            if self.iteration == self.iterations {
                let report = RenderedReport {
                    schema: "pcbcraft-rendered-performance-report/1".into(),
                    environment: self.environment.clone(),
                    target: if cfg!(target_arch = "wasm32") { "browser-rendered" } else { "native-rendered" }.into(),
                    artifact_sha256: self.artifact_sha256.clone(),
                    project_sha256: self.artifact.project_sha256.clone(),
                    recipe: self.artifact.recipe.clone(),
                    trace_id: self.artifact.trace_id.clone(),
                    input_policy: "atomic press/release pairs; retained until UI acceptance; at most 60 Hz; real framework clock".into(),
                    iterations: self.iterations,
                    warmup_frames: WARMUP,
                    idle_between_iterations_ms: self.idle_between_iterations_ms,
                    elapsed_ms: clock_ms() - self.run_started_ms,
                    debug_assertions: cfg!(debug_assertions),
                    renderer: self.renderer.clone(),
                    window_observations: std::mem::take(&mut self.observations),
                    metrics: std::mem::take(&mut self.samples).into_iter().map(|(id, samples)| Metric::new(&id, samples)).collect(),
                    captures: self.captures,
                    image_size: image.size,
                    last_image_sha256: self.last_image_sha256.clone(),
                    unmeasured: vec![
                        "physical input receipt, compositor presentation and input-to-photon latency".into(),
                        "walkaround/shove/ERC/index/worker metrics require their owning features".into(),
                        "long-duration Wayland exposure, lock, display sleep/wake and monitor transitions".into(),
                    ],
                };
                self.complete(ctx, Ok(report));
            } else {
                (self.app, self.driver) = Self::prepare(ctx, &self.artifact)?;
                self.step = 0;
                self.warmup = WARMUP;
                self.next_input_ms = clock_ms() + self.idle_between_iterations_ms;
                self.started_ms = clock_ms() + self.idle_between_iterations_ms;
                #[cfg(target_arch = "wasm32")]
                if let Some(liveness) = &self.liveness {
                    liveness.renew(self.idle_between_iterations_ms + self.deadline_ms);
                }
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(watchdog) = &self.watchdog {
                    watchdog.renew(Duration::from_secs_f64((self.idle_between_iterations_ms + self.deadline_ms) / 1000.0));
                }
            }
        }
        Ok(())
    }

    /// Inject trace events at the framework input hook without replacing its real clock.
    /// Pace actions at no more than 60 Hz so idle steps preserve double-click semantics;
    /// changing RawInput time would invalidate the integration's repaint scheduling.
    fn input(&mut self, ctx: &egui::Context, input: &mut RawInput) -> Result<()> {
        let viewport = input.viewports.get(&egui::ViewportId::ROOT);
        let observation = WindowObservation {
            at_ms: clock_ms(),
            focused: input.focused,
            visible: viewport.and_then(|view| view.visible()),
            minimized: viewport.and_then(|view| view.minimized),
            occluded: viewport.and_then(|view| view.occluded),
        };
        if !self.observations.last().is_some_and(|last| {
            last.focused == observation.focused
                && last.visible == observation.visible
                && last.minimized == observation.minimized
                && last.occluded == observation.occluded
        }) {
            if self.observations.len() >= 4096 {
                return Err("too many window-state transitions for bounded diagnostic report".into());
            }
            self.observations.push(observation);
        }
        input.events.clear();
        input.focused = true;
        self.poll_capture(ctx)?;
        // eframe may call the raw hook for a logic-only pass and save its unconsumed
        // RawInput for later. Keep this one batch until App::ui actually accepts it;
        // clearing events on the next hook would silently discard synthetic clicks.
        if self.active.is_some() {
            input.events.clone_from(&self.pending_input);
            return Ok(());
        }
        if self.completion.is_none() || self.warmup > 0 || self.capture.is_some() || clock_ms() < self.next_input_ms {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(remote) = &self.remote {
                if let Some(elapsed) = remote.poll()? {
                    self.samples.entry("mcp.roundtrip".into()).or_default().push(elapsed);
                    self.remote = None;
                    self.active = Some(self.started_ms);
                }
                return Ok(());
            }
            if let Some(request) = self.driver.remote_request(&self.app, &self.artifact, self.step)? {
                let control = self.app.live_control.as_ref().ok_or("missing diagnostic endpoint")?;
                self.started_ms = clock_ms();
                self.remote = Some(super::remote::RemoteEdit::start(control.port(), control.secret(), request, ctx.clone())?);
                return Ok(());
            }
        }
        log::info!(
            "at_ms={} inject {}: tool={}, routing={}",
            clock_ms(),
            self.artifact.trace[self.step].id,
            self.app.views.active.board.tracks.tool,
            self.app.views.active.board.tracks.routing()
        );
        self.started_ms = clock_ms();
        self.next_input_ms = self.started_ms + 1000.0 / 60.0;
        input.events = self.driver.input(&mut self.app, &self.artifact, self.step, &mut self.samples)?;
        if !self.artifact.trace[self.step].capture_after() {
            self.step += 1;
            input.events.extend(self.driver.input(&mut self.app, &self.artifact, self.step, &mut self.samples)?);
        }
        self.active = Some(self.started_ms);
        self.pending_input.clone_from(&input.events);
        Ok(())
    }
}

impl eframe::App for RenderedApp {
    /// Hidden windows still need a deadline and callback polling; a stalled or obscured
    /// surface is a failed run, never a successful zero-latency sample.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.completion.is_none() {
            return;
        }
        if let Err(error) = self.poll_capture(ctx) {
            self.complete(ctx, Err(error));
        }
        if self.warmup > 0 && clock_ms() - self.started_ms > self.deadline_ms {
            self.complete(ctx, Err("window did not complete rendered warmup before deadline".into()));
        }
        let delay = if self.warmup == 0 { (self.next_input_ms - clock_ms()).max(5.0) } else { 5.0 };
        if delay <= 5.0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(delay / 1000.0));
        }
    }

    /// Keep framework polling responsive while awaiting asynchronous GPU readback.
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut RawInput) {
        if let Err(error) = self.input(ctx, input) {
            self.complete(ctx, Err(error));
        }
    }

    /// Paint the production editor, assert the action's state, then request exactly one
    /// screenshot. Callback timing includes GPU transfer overhead and browser frame delay.
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if self.completion.is_none() {
            #[cfg(not(target_arch = "wasm32"))]
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        let ctx = ui.ctx().clone();
        let before = clock_ms();
        self.app.frame(ui);
        let after = clock_ms();
        #[cfg(target_arch = "wasm32")]
        if let Some(liveness) = &self.liveness {
            liveness.renew((self.next_input_ms - after).max(0.0) + self.deadline_ms);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(watchdog) = &self.watchdog {
            watchdog.renew(Duration::from_secs_f64(((self.next_input_ms - after).max(0.0) + self.deadline_ms) / 1000.0));
        }
        if self.warmup > 0 {
            self.warmup -= 1;
        } else if let Some(start) = self.active.take() {
            self.pending_input.clear();
            let id = &self.artifact.trace[self.step].id;
            self.samples.entry(format!("rendered.{id}.ui")).or_default().push(after - before);
            self.samples.entry(format!("rendered.{id}.input_to_ui")).or_default().push(after - start);
            log::info!(
                "at_ms={} after {}: tool={}, routing={}, pointer={:?}",
                clock_ms(),
                id,
                self.app.views.active.board.tracks.tool,
                self.app.views.active.board.tracks.routing(),
                ctx.input(|i| (i.focused, i.pointer.hover_pos(), i.pointer.primary_pressed(), i.pointer.primary_released(), i.events.clone()))
            );
            if let Err(error) = self.driver.verify(&self.app, &self.artifact, self.step) {
                self.complete(&ctx, Err(error));
                return;
            }
            let (send, receive) = mpsc::sync_channel(1);
            self.capture = Some(receive);
            let wake = ctx.clone();
            #[cfg(target_arch = "wasm32")]
            if let Some(canvas) = self.canvas.clone() {
                wasm_bindgen_futures::spawn_local(async move {
                    match super::browser_capture::capture(canvas).await {
                        Ok(capture) => {
                            let _ = send.try_send(capture);
                        }
                        Err(error) => log::error!("{error}"),
                    }
                    wake.request_repaint();
                });
            } else {
                self.complete(&ctx, Err("browser replay has no capture canvas".into()));
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(error) = self.poll_device(ctx.clone()) {
                self.complete(&ctx, Err(error));
                return;
            }
            #[cfg(not(target_arch = "wasm32"))]
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                let at_ms = clock_ms();
                let bytes: Vec<_> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
                let capture = Capture {
                    at_ms,
                    size: image.size,
                    image_sha256: digest(&bytes),
                    varied: image.pixels.first().is_some_and(|first| image.pixels.iter().any(|pixel| pixel != first)),
                };
                let _ = send.try_send(capture);
                wake.request_repaint();
            })));
        }
        let delay = if self.warmup == 0 { (self.next_input_ms - clock_ms()).max(5.0) } else { 5.0 };
        if delay <= 5.0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(delay / 1000.0));
        }
    }

    /// Diagnostic windows never persist or overwrite the person's application settings.
    fn persist_egui_memory(&self) -> bool {
        false
    }
}

impl Drop for RenderedApp {
    /// Window close and browser teardown cancel only diagnostic work, without waiting on it.
    fn drop(&mut self) {
        self.stop_polling();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Polling alone must not renew an expired deadline, while actual progress may do so.
    #[test]
    fn host_polling_cannot_hide_missing_ui_progress() {
        let liveness = Liveness::default();
        assert!(!liveness.expired());
        liveness.0.set(-1.0);
        assert!(liveness.expired());
        assert!(liveness.expired());
        liveness.renew(30_000.0);
        assert!(!liveness.expired());
    }

    /// Lost callbacks and invisible/blank targets must fail rather than yielding latency evidence.
    #[test]
    fn failed_capture_cannot_produce_a_passing_report() {
        let artifact = super::super::generate("reference-v1", "test").unwrap();
        let environment = Environment {
            revision: "test".into(),
            hardware: "test".into(),
            os: "test".into(),
            runtime: "test".into(),
            compiler: "test".into(),
            profile: "test".into(),
            power: "unknown".into(),
            notes: String::new(),
        };
        let (send, _) = mpsc::sync_channel(1);
        let ctx = egui::Context::default();
        let mut app = RenderedApp::new(&ctx, &artifact, environment, 1, Duration::from_secs(1), send, "test renderer".into()).unwrap();
        // A logic-only hook must not erase input before a later accepted UI pass.
        app.warmup = 0;
        let mut input = RawInput::default();
        app.input(&ctx, &mut input).unwrap();
        let events = input.events.clone();
        let started = app.active;
        assert!(!events.is_empty());
        input.events.clear();
        app.input(&ctx, &mut input).unwrap();
        assert_eq!(input.events, events);
        assert_eq!(app.active, started);
        assert_eq!(app.step, 1);
        let (send, receive) = mpsc::sync_channel(1);
        app.capture = Some(receive);
        app.started_ms = clock_ms() - 2000.0;
        assert!(app.poll_capture(&ctx).unwrap_err().contains("timed out"));
        drop(send);
        assert!(app.poll_capture(&ctx).unwrap_err().contains("disconnected"));
        let (send, receive) = mpsc::sync_channel(1);
        app.capture = Some(receive);
        send.try_send(Capture { at_ms: clock_ms(), size: [1400, 900], image_sha256: "test".into(), varied: false }).unwrap();
        assert!(app.poll_capture(&ctx).unwrap_err().contains("blank"));
        assert_eq!(app.captures, 0);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (send, receive) = mpsc::sync_channel(1);
            app.poll_error = Some(receive);
            send.try_send("GPU completion polling failed: device lost".into()).unwrap();
            assert!(app.poll_capture(&ctx).unwrap_err().contains("device lost"));
            let (send, receive) = mpsc::sync_channel(1);
            app.watchdog = Some(
                super::super::watchdog::Watchdog::start(Duration::from_secs(60), move || {
                    let _ = send.try_send(());
                })
                .unwrap(),
            );
            app.complete(&ctx, Err("test failure".into()));
            // The event loop must remain monitored until actual teardown, even after
            // a result is queued: a lost Close command must not strand the executable.
            assert!(app.watchdog.is_some());
            drop(app);
            assert!(matches!(receive.recv_timeout(Duration::from_secs(5)), Err(mpsc::RecvTimeoutError::Disconnected)));
        }
    }
}
