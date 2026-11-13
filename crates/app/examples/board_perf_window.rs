//! Replay canonical inputs through a real native window and GPU, with bounded readback.
#[cfg(not(target_arch = "wasm32"))]
/// Run one isolated diagnostic window; incomplete replays never publish a report.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use pcbcraft_app::performance::{Environment, rendered::RenderedApp};
    use std::{
        io::Write,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };
    struct DiagnosticLog;
    impl log::Log for DiagnosticLog {
        /// Keep diagnostics limited to the renderer and replay, excluding unrelated service logs.
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Info
                && (metadata.target().starts_with("pcbcraft_app::performance") || metadata.target().starts_with("egui_wgpu"))
        }
        /// Emit selected diagnostic records without including endpoint credentials.
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                eprintln!("{}: {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: DiagnosticLog = DiagnosticLog;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    let args: Vec<_> = std::env::args().skip(1).collect();
    let preserve_stall = args.last().is_some_and(|arg| arg == "--preserve-on-stall");
    let args = if preserve_stall { &args[..args.len() - 1] } else { &args[..] };
    let (files, idle) =
        if args.len() == 6 && args[4] == "--idle-seconds" { (&args[..4], Duration::from_secs(args[5].parse()?)) } else { (args, Duration::ZERO) };
    let [artifact, metadata, iterations, output] = files else {
        return Err("usage: board_perf_window ARTIFACT ENVIRONMENT_JSON ITERATIONS REPORT [--idle-seconds N] [--preserve-on-stall]".into());
    };
    if std::path::Path::new(output).exists() {
        return Err("report already exists".into());
    }
    let artifact = std::fs::read_to_string(artifact)?;
    let mut environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
    use sha2::{Digest, Sha256};
    let executable_sha256 = format!("{:x}", Sha256::digest(std::fs::read(std::env::current_exe()?)?));
    environment.notes.push_str(&format!(" Executable SHA-256: {executable_sha256}."));
    let iterations = iterations.parse()?;
    let (send, receive) = mpsc::sync_channel(1);
    let failed = Arc::new(AtomicBool::new(false));
    let watchdog_failed = failed.clone();
    let watchdog = pcbcraft_app::performance::watchdog::Watchdog::start(Duration::from_secs(30), move || {
        watchdog_failed.store(true, Ordering::Release);
        eprintln!("Error: native replay made no UI progress before the independent watchdog deadline; no report written");
        if preserve_stall {
            eprintln!(
                "Diagnostic PID {} preserved for read-only stacks/protocol investigation; any later recovery remains a failed run",
                std::process::id()
            );
        } else {
            std::process::exit(1);
        }
    })?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_decorations(!cfg!(target_os = "linux"))
            .with_resizable(false)
            .with_active(true)
            .with_always_on_top(),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "PCBcraft performance replay",
        options,
        Box::new(move |cc| {
            let renderer =
                cc.wgpu_render_state.as_ref().map(|state| format!("{:?}", state.adapter.get_info())).unwrap_or_else(|| "unknown renderer".into());
            let device = cc.wgpu_render_state.as_ref().ok_or("native replay requires WGPU")?.device.clone();
            Ok(Box::new(
                RenderedApp::new(&cc.egui_ctx, &artifact, environment, iterations, Duration::from_secs(30), send, renderer)?
                    .with_idle_between_iterations(idle)?
                    .with_gpu_device(device)
                    .with_watchdog(watchdog),
            ))
        }),
    )
    .map_err(|e| e.to_string())?;
    if failed.load(Ordering::Acquire) {
        return Err("independent watchdog expired; recovered or partial results are not baseline evidence".into());
    }
    let report = receive.try_recv().map_err(|_| "window closed before replay completed")??;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
    file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
