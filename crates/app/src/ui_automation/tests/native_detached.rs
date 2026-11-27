//! Opt-in native proof that detached editor windows take queued UI input through the same
//! authenticated transport, and that the main window shares the resulting history.
use super::native::{Sized, capture, input};
use super::*;
use pcbcraft_mcp::{Backend, live::Remote};
use std::sync::{Arc, Mutex, mpsc};

/// One live command through the transport.
fn call(remote: &mut Remote, id: &str, params: Value) -> Result<Value, String> {
    remote.call("engine.execute", json!({"command":id,"params":params})).map_err(|failure| failure.to_string())
}

/// Poll until `done` holds, with a bounded deadline.
fn wait(what: &str, mut done: impl FnMut() -> Result<bool, String>) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done()? {
        if Instant::now() > deadline {
            return Err(format!("Timed out waiting for {what}"));
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}

/// Ask for OS focus on one window and wait until its measured paint reports it.
fn focus(remote: &mut Remote, ctx: &egui::Context, window: u64) -> Result<(), String> {
    ctx.send_viewport_cmd_to(crate::editor_windows::viewport(HostId(window)), egui::ViewportCommand::Focus);
    ctx.request_repaint();
    wait(&format!("focus on window {window}"), || Ok(call(remote, "ui.handles", json!({"window":window,"prefix":"canvas:"}))?["focused"] == true))
}

/// Run a window command that opens a native window, then return its measured, focused ID.
fn open(remote: &mut Remote, ctx: &egui::Context, id: &str, params: Value) -> Result<u64, String> {
    let ready = |remote: &mut Remote| -> Result<Vec<u64>, String> {
        Ok(call(remote, "ui.inspect", json!({}))?["ready_windows"]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default())
    };
    let before = ready(remote)?;
    call(remote, id, params)?;
    let mut opened = None;
    wait(&format!("a window opened by {id}"), || {
        opened = ready(remote)?.into_iter().find(|id| !before.contains(id));
        Ok(opened.is_some())
    })?;
    let window = opened.unwrap_or_default();
    focus(remote, ctx, window)?;
    Ok(window)
}

/// Drag the item at model point `at` in `window`'s focused view through ordinary pointer input.
fn drag(remote: &mut Remote, window: u64, at: (i64, i64)) -> Result<(), String> {
    let editor = remote.call("editor.inspect", json!({})).map_err(|failure| failure.to_string())?;
    let view = editor["windows"].as_array().and_then(|windows| windows.iter().find(|w| w["id"] == window)).map(|w| w["focused_view"].clone());
    let point = call(remote, "ui.project", json!({"view":view.ok_or("Window has no focused view")?,"at":{"x_nm":at.0,"y_nm":at.1}}))?;
    if point["window"] != window || point["uncovered"] != true {
        return Err(format!("The item is not uncovered in window {window}: {point}"));
    }
    let (x, y) = (point["x"].as_f64().unwrap_or_default(), point["y"].as_f64().unwrap_or_default());
    input(remote, "ui.drag", json!({"window":window,"layout":point["layout"],"from":{"x":x,"y":y},"to":{"x":x+40.64,"y":y},"steps":8}))
}

/// The authored symbols and footprints, compared as whole records.
fn placements(remote: &mut Remote) -> Result<Value, String> {
    let project = remote.call("project.inspect", json!({})).map_err(|failure| failure.to_string())?;
    Ok(json!([project["project"]["schematic"]["symbols"], project["project"]["board"]["footprints"]]))
}

/// Run on an isolated X11 display with a window manager that focuses new and activated windows.
#[test]
#[ignore = "requires isolated X11 with a window manager, a GPU/software adapter and local sockets"]
fn native_detached_windows_edit_the_shared_project() {
    use winit::platform::x11::EventLoopBuilderExtX11;
    let result = Arc::new(Mutex::new(None));
    let outcome = result.clone();
    let (send, done) = mpsc::sync_channel(1);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("PCBcraft detached input test").with_inner_size([1400.0, 900.0]).with_active(true),
        persist_window: false,
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_x11().with_any_thread(true);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "PCBcraft detached input test",
        options,
        Box::new(move |cc| {
            pcbcraft_chrome::install(&cc.egui_ctx);
            let mut app = PcbApp::default();
            app.start_new_project_in(crate::Workspace::Board);
            app.views.active.switch(crate::Workspace::Schematic, &app.session);
            let revision = app.session.revision();
            app.session
                .apply(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision}))
                .map_err(|e| e.to_string())?;
            let symbol = app.session.project().map(|p| p.symbols()[0].clone()).ok_or("No symbol was placed")?;
            let revision = app.session.revision();
            app.session
                .apply(
                    pcbcraft_engine::PLACE_FOOTPRINT,
                    &json!({"symbol":symbol.id(),"definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0,"expected_revision":revision}),
                )
                .map_err(|e| e.to_string())?;
            let (symbol, footprint) = (symbol.position, (0, 0));
            app.automation.observing = true;
            let control = crate::live_control::LiveControl::start(&cc.egui_ctx)?;
            let (port, token) = (control.port(), control.secret().to_owned());
            app.live_control = Some(control);
            let (ctx, view) = (cc.egui_ctx.clone(), app.views.active.id().0);
            std::thread::spawn(move || {
                let run = || -> Result<(), String> {
                    let mut remote = Remote::new(port, &token)?;
                    wait("window 0", || Ok(call(&mut remote, "ui.window", json!({"window":0})).is_ok()))?;
                    let before = placements(&mut remote)?;
                    let schematic = open(&mut remote, &ctx, "view.open_window", json!({"view":view}))?;
                    drag(&mut remote, schematic, (symbol.x_nm, symbol.y_nm))?;
                    capture("detached-schematic")?;
                    let board = open(&mut remote, &ctx, "app.set_workspace", json!({"view":view,"workspace":"board","placement":"window"}))?;
                    drag(&mut remote, board, footprint)?;
                    capture("detached-board")?;
                    let after = placements(&mut remote)?;
                    if after[0] == before[0] || after[1] == before[1] {
                        return Err(format!("Detached drags did not edit both documents: {after}"));
                    }
                    focus(&mut remote, &ctx, 0)?;
                    // Undo reverses the footprint drag first; Redo reapplies the symbol drag first.
                    let symbol_only = json!([after[0], before[1]]);
                    for (key, expected) in [("Z", &symbol_only), ("Z", &before), ("Y", &symbol_only), ("Y", &after)] {
                        input(&mut remote, "ui.key", json!({"window":0,"key":key,"modifiers":{"command":true}}))?;
                        let now = placements(&mut remote)?;
                        if &now != expected {
                            return Err(format!("Command+{key} in window 0 left {now}, expected {expected}"));
                        }
                    }
                    Ok(())
                };
                let _ = send.send(run());
            });
            Ok(Box::new(Sized { app, done, result: outcome, closing: false }))
        }),
    )
    .unwrap();
    result.lock().unwrap().take().expect("native window closed without a result").unwrap();
}
