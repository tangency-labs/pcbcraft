//! Opt-in verification of eframe's real OS clipboard between two independent native processes.
#![allow(clippy::unwrap_used)]
use super::*;
use std::{
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use winit::platform::wayland::EventLoopBuilderExtWayland as _;

struct ClipboardLogger;
impl log::Log for ClipboardLogger {
    /// Keep failure diagnostics confined to clipboard backends.
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().contains("clipboard")
    }
    /// Report backend failures without printing clipboard payloads.
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{} {} {}", record.level(), record.target(), record.args());
        }
    }
    fn flush(&self) {}
}
static CLIPBOARD_LOGGER: ClipboardLogger = ClipboardLogger;
struct Peer {
    app: PcbApp,
    root: PathBuf,
    sender: bool,
    frames: u64,
    requested: Option<u64>,
    started: Instant,
    passed: Arc<AtomicBool>,
}
impl eframe::App for Peer {
    /// Let eframe publish/read the actual OS clipboard, keeping its owner process alive until the
    /// independent receiver has decoded and committed the fragment through the real app handler.
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        if self.frames == 1 {
            println!("Peer first native frame, sender={}", self.sender);
        }
        assert!(self.started.elapsed() < Duration::from_secs(30), "native clipboard peer timed out");
        self.app.frame(ui);
        if self.sender {
            if self.requested.is_none() && self.started.elapsed() > Duration::from_secs(3) && ui.input(|i| i.focused) {
                println!(
                    "Sender copying {} selected objects, native focused={}",
                    self.app.views.active.schematic.selection.len(),
                    ui.input(|i| i.focused)
                );
                let id = self.app.session.project().unwrap().symbols()[0].id();
                self.app.select(pcbcraft_engine::selection::Selection::one_symbol(id));
                self.app.copy_schematic_clipboard(ui.ctx(), false);
                self.requested = Some(self.frames);
                println!("Sender queued OS clipboard output, status={:?}, outputs={}", self.app.message, ui.ctx().output(|o| o.commands.len()));
                std::fs::write(self.root.join("ready"), b"ready").unwrap();
            }
            if self.root.join("done").exists() {
                self.passed.store(true, Ordering::SeqCst);
                self.app.close_allowed = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else {
            if self.requested.is_none() && self.started.elapsed() > Duration::from_secs(3) && ui.input(|i| i.focused) {
                println!("Receiver requesting native clipboard, focused={}", ui.input(|i| i.focused));
                self.app.request_schematic_clipboard(ui.ctx(), false);
                self.requested = Some(self.frames);
            }
            if let Some(placement) = self.app.views.active.schematic.clipboard.paste.clone() {
                self.app.commit_schematic_clipboard(&placement, Point { x_nm: 12_700_000, y_nm: 12_700_000 }).unwrap();
                assert_eq!(self.app.session.project().unwrap().symbols().len(), 1);
                assert_eq!(self.app.session.project().unwrap().components().next().unwrap().reference(), "R1");
                std::fs::write(self.root.join("done"), b"passed").unwrap();
                self.passed.store(true, Ordering::SeqCst);
                self.app.close_allowed = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
        if self.requested.is_none() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(40));
    }
}

/// Child entry point for the process regression; callers must supply the temporary rendezvous path.
#[test]
#[ignore = "helper for native_clipboard_between_processes"]
fn native_clipboard_peer() -> eframe::Result {
    let Some(root) = std::env::var_os("PCBCRAFT_CLIPBOARD_TEST_ROOT") else {
        return Ok(());
    };
    let _ = log::set_logger(&CLIPBOARD_LOGGER);
    log::set_max_level(log::LevelFilter::Trace);
    let sender = std::env::var("PCBCRAFT_CLIPBOARD_TEST_ROLE").unwrap() == "sender";
    let passed = Arc::new(AtomicBool::new(false));
    let evidence = passed.clone();
    let mut app = PcbApp::default();
    app.session.apply("project.new", &json!({"name":if sender {"Clipboard sender"}else{"Clipboard receiver"}})).unwrap();
    if sender {
        app.session
            .apply(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}))
            .unwrap();
        let id = app.session.project().unwrap().symbols()[0].id();
        app.views.sync(&app.session);
        app.select(pcbcraft_engine::selection::Selection::one_symbol(id));
    }
    println!("Peer starting native window, sender={sender}");
    eframe::run_native(
        if sender { "PCBcraft clipboard sender" } else { "PCBcraft clipboard receiver" },
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([900., 700.]).with_visible(true),
            persist_window: false,
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_any_thread(true);
            })),
            ..Default::default()
        },
        Box::new(move |cc| {
            pcbcraft_chrome::fonts::install(&cc.egui_ctx);
            Ok(Box::new(Peer { app, root: PathBuf::from(root), sender, frames: 0, requested: None, started: Instant::now(), passed }))
        }),
    )?;
    assert!(evidence.load(Ordering::SeqCst), "native clipboard peer closed before verification");
    Ok(())
}

/// Run on an isolated Wayland session: two real windows/processes exchange OS clipboard text.
/// The sender must remain alive because Wayland clipboard ownership is tied to the native client.
#[test]
#[ignore = "opens two native windows and temporarily replaces the system clipboard; use an isolated Wayland display"]
fn native_clipboard_between_processes() {
    let root = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let launch = |role: &str| {
        Command::new(&exe)
            .args(["--ignored", "--exact", "clipboard::native_tests::native_clipboard_peer", "--nocapture"])
            .env("PCBCRAFT_CLIPBOARD_TEST_ROOT", root.path())
            .env("PCBCRAFT_CLIPBOARD_TEST_ROLE", role)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let mut sender = launch("sender");
    let started = Instant::now();
    while !root.path().join("ready").exists() && started.elapsed() < Duration::from_secs(20) {
        if sender.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    if !root.path().join("ready").exists() {
        let _ = sender.kill();
        let output = sender.wait_with_output().unwrap();
        panic!("sender failed: {} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
    let receiver = finish_peer(launch("receiver"));
    if !receiver.status.success() {
        let _ = sender.kill();
    }
    let sender = finish_peer(sender);
    assert!(
        receiver.status.success(),
        "receiver: {} {} sender: {} {}",
        String::from_utf8_lossy(&receiver.stdout),
        String::from_utf8_lossy(&receiver.stderr),
        String::from_utf8_lossy(&sender.stdout),
        String::from_utf8_lossy(&sender.stderr)
    );
    assert!(sender.status.success(), "sender: {} {}", String::from_utf8_lossy(&sender.stdout), String::from_utf8_lossy(&sender.stderr));
}

/// Bound cleanup even if a platform clipboard call stalls outside the native frame loop.
fn finish_peer(mut child: Child) -> Output {
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() && started.elapsed() < Duration::from_secs(35) {
        std::thread::sleep(Duration::from_millis(40));
    }
    if child.try_wait().unwrap().is_none() {
        let _ = child.kill();
    }
    child.wait_with_output().unwrap()
}
