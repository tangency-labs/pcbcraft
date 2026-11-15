//! Opt-in native-window proof, using the same authenticated transport as a demonstration.
use super::*;
use pcbcraft_mcp::{Backend, live::Remote};
use std::sync::{Arc, Mutex, mpsc};

/// Execute one accepted input exactly once, then poll its completion with a bounded deadline.
fn input(remote: &mut Remote, id: &str, params: Value) -> Result<(), String> {
    let accepted = remote.call("engine.execute", json!({"command":id,"params":params}))?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        let status = remote.call("engine.execute", json!({"command":"ui.inspect","params":{}}))?;
        let operation = &status["operation"];
        if operation["ticket"] == accepted["ticket"] {
            match operation["state"].as_str() {
                Some("completed") => return Ok(()),
                Some("cancelled") => return Err(status.to_string()),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    Err(format!("Native input did not complete: {id}"))
}

/// Capture only this isolated test display when an evidence destination was explicitly supplied.
fn capture(name: &str) -> Result<(), String> {
    if let Some(root) = std::env::var_os("PCBCRAFT_UI_EVIDENCE") {
        // A completed ticket acknowledges UI processing, not compositor presentation.
        std::thread::sleep(Duration::from_millis(500));
        let destination = std::path::PathBuf::from(root).join(format!("native-{name}.png"));
        let status = std::process::Command::new("import").args(["-window", "root"]).arg(destination).status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("Native test capture failed".into());
        }
    }
    Ok(())
}

struct Window {
    app: PcbApp,
    done: mpsc::Receiver<Result<(), String>>,
    result: Arc<Mutex<Option<Result<(), String>>>>,
    saw_wire: bool,
    closing: bool,
}

impl eframe::App for Window {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut RawInput) {
        self.app.automation_input(ctx, raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.closing {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.app.frame(ui);
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title("PCBcraft UI automation test".into()));
        self.saw_wire |= matches!(self.app.views.active.schematic.tool, Some(crate::Tool::Wire { .. }));
        if let Ok(result) = self.done.try_recv() {
            let result = result.and_then(|()| {
                if !self.saw_wire {
                    return Err("Native button click did not arm Wire".into());
                }
                if self.app.session.project().is_none_or(|p| p.name != "Native recorded circuit") {
                    return Err("Native text input did not rename the project".into());
                }
                Ok(())
            });
            *self.result.lock().unwrap() = Some(result);
            self.closing = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

/// Run on an isolated X11 display; no user settings, projects or ordinary app windows are used.
#[test]
#[ignore = "requires isolated X11, a GPU/software adapter and local sockets"]
fn native_window_input_uses_real_controls() {
    use winit::platform::x11::EventLoopBuilderExtX11;
    let result = Arc::new(Mutex::new(None));
    let outcome = result.clone();
    let (send, done) = mpsc::sync_channel(1);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("PCBcraft UI automation test").with_inner_size([1600.0, 1000.0]).with_active(true),
        persist_window: false,
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_x11().with_any_thread(true);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "PCBcraft UI automation test",
        options,
        Box::new(move |cc| {
            pcbcraft_chrome::install(&cc.egui_ctx);
            let mut app = PcbApp::default();
            app.session.apply("project.new", &json!({})).map_err(|e| e.to_string())?;
            app.session
                .apply(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}))
                .map_err(|e| e.to_string())?;
            app.views.active.bind_opened_project(&app.session);
            app.automation.observing = true;
            let control = crate::live_control::LiveControl::start(&cc.egui_ctx)?;
            let (port, token) = (control.port(), control.secret().to_owned());
            app.live_control = Some(control);
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            std::thread::spawn(move || {
                let run = || -> Result<(), String> {
                    let mut remote = Remote::new(port, &token)?;
                    let deadline = Instant::now() + Duration::from_secs(10);
                    loop {
                        let handles = remote.call("engine.execute", json!({"command":"ui.handles","params":{"window":0}}));
                        if handles.as_ref().is_ok_and(|h| {
                            h["focused"] == true
                                && h["handles"].as_array().is_some_and(|targets| targets.iter().any(|t| t["name"] == "toolbar:Connect:Wire"))
                        }) {
                            break;
                        }
                        if Instant::now() > deadline {
                            return Err(format!("Native targets did not become ready/focused: {handles:?}"));
                        }
                        std::thread::sleep(Duration::from_millis(16));
                    }
                    input(&mut remote, "ui.click", json!({"window":0,"target":{"handle":"toolbar:Connect:Wire"}}))?;
                    capture("wire")?;
                    input(&mut remote, "ui.key", json!({"window":0,"key":"Escape"}))?;
                    input(&mut remote, "ui.click", json!({"window":0,"target":{"handle":"menu:File"}}))?;
                    input(&mut remote, "ui.click", json!({"window":0,"target":{"handle":"menu:Rename Project…"}}))?;
                    input(&mut remote, "ui.key", json!({"window":0,"key":"A","modifiers":{"command":true}}))?;
                    input(&mut remote, "ui.text", json!({"window":0,"text":"Native recorded circuit"}))?;
                    capture("dialog")?;
                    input(&mut remote, "ui.click", json!({"window":0,"target":{"handle":"button:OK"}}))?;
                    capture("completed")?;
                    Ok(())
                };
                let _ = send.send(run());
            });
            Ok(Box::new(Window { app, done, result: outcome, saw_wire: false, closing: false }))
        }),
    )
    .unwrap();
    result.lock().unwrap().take().expect("native window closed without a result").unwrap();
}
