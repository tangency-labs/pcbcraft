//! Opt-in native regression for the local eframe patch. Uses real OS windows; run on an
//! isolated display with `cargo test -p pcbcraft --test native_window_lifecycle -- --ignored`.
#![cfg(any(target_os = "linux", target_os = "windows"))]
#![allow(clippy::unwrap_used)]

use eframe::egui::{self, ViewportCommand, ViewportId};
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};
#[derive(Default)]
struct Evidence {
    ids: HashSet<WindowId>,
    created: usize,
    child_frames: usize,
    complete: bool,
}
struct Verify {
    root: Weak<Window>,
    evidence: Arc<Mutex<Evidence>>,
    stage: usize,
    due: Instant,
    visible_root: bool,
    child: bool,
    frames_before: usize,
    cancelled: bool,
    quitting: bool,
    restore_single: bool,
}
impl eframe::App for Verify {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let child = ViewportId::from_hash_of("surviving editor");
        if std::mem::take(&mut self.restore_single) {
            self.child = false;
            self.visible_root = true;
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.quitting && self.child {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                frame.retire_root_window();
                self.visible_root = false;
                println!("Root close requested; retaining child viewport {child:?}");
            } else {
                self.evidence.lock().unwrap().complete = true;
                return;
            }
        }
        if self.visible_root {
            ui.label("Same shared document");
        }
        if self.child {
            ctx.show_viewport_immediate(
                child,
                egui::ViewportBuilder::default().with_title("Same shared document").with_inner_size([800., 600.]),
                |ui, _| {
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        if self.visible_root {
                            self.child = false;
                            return;
                        }
                        ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                        if self.cancelled {
                            self.quitting = true;
                            ui.ctx().send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
                        } else {
                            self.cancelled = true;
                        }
                    }
                    ui.label("Same shared document");
                    self.evidence.lock().unwrap().child_frames += 1;
                },
            );
        }
        if Instant::now() < self.due {
            return;
        }
        self.due = Instant::now() + Duration::from_secs(1);
        println!("Checkpoint {}", self.stage);
        match self.stage {
            0 => {
                assert_eq!(self.evidence.lock().unwrap().ids.len(), 2);
                self.frames_before = self.evidence.lock().unwrap().child_frames;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            1 => {
                assert!(self.root.upgrade().is_none(), "original native Window still alive");
                assert_eq!(self.evidence.lock().unwrap().created, 2, "survivor was recreated");
                assert_eq!(self.evidence.lock().unwrap().ids.len(), 1);
                assert!(self.evidence.lock().unwrap().child_frames > self.frames_before);
                assert!(frame.winit_window().is_none());
                println!("PASS original native window destroyed; original child still rendering");
                // Restore the root as an arrangement switch does.
                self.visible_root = true;
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            }
            2 => {
                assert!(frame.winit_window().is_some(), "root was not recreated");
                assert_eq!(self.evidence.lock().unwrap().created, 3);
                println!("PASS root recreated without replacing child");
                ctx.send_viewport_cmd_to(child, ViewportCommand::Close);
            }
            3 => {
                assert!(!self.child);
                self.child = true;
                self.root = Arc::downgrade(frame.winit_window().unwrap());
            }
            4 => {
                assert_eq!(self.evidence.lock().unwrap().created, 4);
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            5 => {
                assert!(self.root.upgrade().is_none());
                assert_eq!(self.evidence.lock().unwrap().ids.len(), 1);
                // Restore a single-window arrangement: there must be no gap with no event driver.
                self.restore_single = true;
            }
            6 => {
                assert!(frame.winit_window().is_some());
                assert_eq!(self.evidence.lock().unwrap().ids.len(), 1);
                assert_eq!(self.evidence.lock().unwrap().created, 5);
                self.root = Arc::downgrade(frame.winit_window().unwrap());
                self.child = true;
            }
            7 => {
                assert_eq!(self.evidence.lock().unwrap().created, 6);
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            8 => {
                assert!(self.root.upgrade().is_none());
                self.frames_before = self.evidence.lock().unwrap().child_frames;
                ctx.send_viewport_cmd_to(child, ViewportCommand::Close);
            }
            9 => {
                assert!(self.cancelled && !self.quitting && self.child);
                assert_eq!(self.evidence.lock().unwrap().created, 6, "cancel recreated the last window");
                assert!(self.evidence.lock().unwrap().child_frames > self.frames_before);
                ctx.send_viewport_cmd_to(child, ViewportCommand::Close);
            }
            _ => {}
        }
        self.stage += 1;
    }
}
struct Observe<'a> {
    app: eframe::EframeWinitApplication<'a>,
    evidence: Arc<Mutex<Evidence>>,
}
impl ApplicationHandler<eframe::UserEvent> for Observe<'_> {
    fn resumed(&mut self, e: &ActiveEventLoop) {
        self.app.resumed(e)
    }
    fn window_event(&mut self, e: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        {
            let mut evidence = self.evidence.lock().unwrap();
            if matches!(event, WindowEvent::Destroyed) {
                evidence.ids.remove(&id);
                println!("Native window destroyed {id:?}");
            } else if evidence.ids.insert(id) {
                evidence.created += 1;
                println!("Native window observed {id:?}");
            }
        }
        self.app.window_event(e, id, event)
    }
    fn new_events(&mut self, e: &ActiveEventLoop, c: StartCause) {
        self.app.new_events(e, c)
    }
    fn user_event(&mut self, e: &ActiveEventLoop, v: eframe::UserEvent) {
        self.app.user_event(e, v)
    }
    fn device_event(&mut self, e: &ActiveEventLoop, id: DeviceId, v: DeviceEvent) {
        self.app.device_event(e, id, v)
    }
    fn about_to_wait(&mut self, e: &ActiveEventLoop) {
        self.app.about_to_wait(e)
    }
    fn suspended(&mut self, e: &ActiveEventLoop) {
        self.app.suspended(e)
    }
    fn exiting(&mut self, e: &ActiveEventLoop) {
        self.app.exiting(e)
    }
}
/// Native identities and rendering survive both close orders, root recreation, Cancel and
/// a final-window exit initiated inside an immediate child callback.
#[test]
#[ignore = "opens native windows; run with an isolated Wayland or X11 display"]
fn native_window_lifecycle() -> eframe::Result {
    let evidence = Arc::new(Mutex::new(Evidence::default()));
    let done = Arc::new(AtomicBool::new(false));
    #[cfg(target_os = "linux")]
    use winit::platform::wayland::EventLoopBuilderExtWayland as _;
    #[cfg(target_os = "windows")]
    use winit::platform::windows::EventLoopBuilderExtWindows as _;
    let event_loop = EventLoop::<eframe::UserEvent>::with_user_event().with_any_thread(true).build()?;
    let ev = evidence.clone();
    let stop = done.clone();
    let app = eframe::create_native(
        "Native lifecycle regression",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([900., 700.]),
            persist_window: false,
            ..Default::default()
        },
        Box::new(move |cc| {
            let root = Arc::downgrade(cc.winit_window().unwrap());
            println!("Created root {:?}", cc.winit_window().unwrap().id());
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                    ctx.request_repaint();
                }
            });
            Ok(Box::new(Verify {
                root,
                evidence: ev,
                stage: 0,
                due: Instant::now() + Duration::from_secs(2),
                visible_root: true,
                child: true,
                frames_before: 0,
                cancelled: false,
                quitting: false,
                restore_single: false,
            }))
        }),
        &event_loop,
    );
    let mut observer = Observe { app, evidence: evidence.clone() };
    event_loop.run_app(&mut observer)?;
    done.store(true, Ordering::Relaxed);
    assert!(evidence.lock().unwrap().complete, "regression did not finish");
    println!("PASS native lifecycle regression");
    Ok(())
}
