//! Bounded navigation diagnostics over an unchanged native project snapshot.

use super::{Environment, Metric, Result, timed};
use crate::{PcbApp, editor_view::Workspace};
use egui::{Event, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};
use pcbcraft_engine::{Project, persistence::encode_packed};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// Exercise continuously changing zoom sizes in every populated sheet and the board at a
/// large desktop viewport. Unique scales expose cold text/layout work hidden by alternating
/// between two cached sizes. Reports distinguish this trace from the navigation smoke.
pub fn zoom_all(project: Arc<Project>, environment: Environment) -> Result<Value> {
    environment.validate()?;
    let original = encode_packed(project.clone())?;
    let mut targets: Vec<_> = project
        .sheets()
        .filter(|sheet| project.for_sheet(sheet.target).is_some_and(|p| !p.symbols().is_empty() || !p.sheet_symbols().is_empty()))
        .map(|sheet| (Workspace::Schematic, Some(sheet.target), sheet.name.to_owned()))
        .collect();
    targets.push((Workspace::Board, None, "PCB".to_owned()));
    let mut results = Vec::new();
    for (workspace, sheet, name) in targets {
        let mut app = PcbApp::default();
        app.session.open_snapshot(project.clone());
        app.views.active.bind_opened_project(&app.session);
        if let Some(sheet) = sheet {
            app.views.active.open_document(crate::editor_view::DocumentTarget::Sheet(sheet), &app.session);
        } else {
            app.views.active.switch(workspace, &app.session);
        }
        app.views.active.camera.request_fit();
        app.home = false;
        let ctx = egui::Context::default();
        let mut samples = BTreeMap::new();
        let mut texture_upload_bytes = BTreeMap::<String, Vec<usize>>::new();
        for number in 0..78 {
            let pointer = pos2(1400.0, 800.0);
            let (id, events) = match number {
                0..8 => ("warmup", vec![]),
                8..18 => ("idle", vec![Event::PointerMoved(pointer)]),
                18..48 => ("zoom_in", vec![Event::PointerMoved(pointer), Event::Zoom(1.033)]),
                _ => ("zoom_out", vec![Event::PointerMoved(pointer), Event::Zoom(1.0 / 1.027)]),
            };
            let zoom_before = app.views.active.camera.zoom();
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(2560.0, 1600.0))),
                time: Some(number as f64 / 60.0),
                focused: true,
                events,
                ..Default::default()
            };
            let mut output = timed(&mut samples, &format!("{id}.ui"), || ctx.run_ui(input, |ui| app.frame(ui)));
            let texture_bytes: usize =
                output.textures_delta.set.values().flatten().map(|delta| delta.image.size().iter().product::<usize>() * 4).sum();
            texture_upload_bytes.entry(id.to_owned()).or_default().push(texture_bytes);
            output.textures_delta.clear();
            timed(&mut samples, &format!("{id}.tessellate"), || ctx.tessellate(output.shapes, output.pixels_per_point));
            if id.starts_with("zoom_") && app.views.active.camera.zoom() == zoom_before {
                return Err(format!("{name}: zoom input missed canvas"));
            }
        }
        if encode_packed(app.session.snapshot().ok_or("missing project")?)? != original {
            return Err("zoom modified authored data".into());
        }
        results.push(json!({"workspace":format!("{workspace:?}"),"document":name,"sheet":sheet,"texture_upload_bytes":texture_upload_bytes,"metrics":samples.into_iter().map(|(id,samples)|Metric::new(&id,samples)).collect::<Vec<_>>()}));
    }
    Ok(
        json!({"schema":"pcbcraft-project-zoom/1","project_sha256":super::digest(&original),"environment":environment,"viewport":[2560,1600],"debug_assertions":cfg!(debug_assertions),"warmup_frames":8,"results":results}),
    )
}

/// Replay idle, zoom and ordinary canvas clicks without authoring or saving data.
/// Eight warmup passes precede each ten-frame group; raw CPU samples exclude presentation.
pub fn navigation(project: Arc<Project>, environment: Environment) -> Result<Value> {
    environment.validate()?;
    let original = encode_packed(project.clone())?;
    let mut results = Vec::new();
    for workspace in [Workspace::Schematic, Workspace::Board] {
        let mut app = PcbApp::default();
        app.session.open_snapshot(project.clone());
        app.views.active.bind_opened_project(&app.session);
        app.views.active.switch(workspace, &app.session);
        app.views.active.camera.request_fit();
        app.home = false;
        let ctx = egui::Context::default();
        let mut samples = BTreeMap::new();
        for number in 0..38 {
            let pointer = pos2(780.0, 440.0);
            let (id, events) = match number {
                0..8 => ("warmup", vec![]),
                8..18 => ("idle", vec![Event::PointerMoved(pointer)]),
                18..28 => ("zoom", vec![Event::PointerMoved(pointer), Event::Zoom(if number % 2 == 0 { 1.05 } else { 1.0 / 1.05 })]),
                _ => (
                    "click",
                    vec![
                        Event::PointerMoved(pointer),
                        Event::PointerButton { pos: pointer, button: PointerButton::Primary, pressed: number % 2 == 0, modifiers: Modifiers::NONE },
                    ],
                ),
            };
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))),
                time: Some(number as f64 / 60.0),
                focused: true,
                events,
                ..Default::default()
            };
            let zoom_before = app.views.active.camera.zoom();
            let mut output = timed(&mut samples, &format!("{id}.ui"), || ctx.run_ui(input, |ui| app.frame(ui)));
            output.textures_delta.clear();
            timed(&mut samples, &format!("{id}.tessellate"), || ctx.tessellate(output.shapes, output.pixels_per_point));
            if id == "zoom" && app.views.active.camera.zoom() == zoom_before {
                return Err(format!("{workspace:?}: zoom input did not reach the canvas"));
            }
            if id == "click" && !ctx.input(|input| input.pointer.primary_pressed() || input.pointer.primary_released()) {
                return Err(format!("{workspace:?}: click input was not observed"));
            }
        }
        if encode_packed(app.session.snapshot().ok_or("missing project after navigation")?)? != original {
            return Err("navigation modified authored data".into());
        }
        results.push(json!({"workspace":format!("{workspace:?}"),"metrics":samples.into_iter().map(|(id,samples)|Metric::new(&id,samples)).collect::<Vec<_>>()}));
    }
    Ok(
        json!({"schema":"pcbcraft-project-navigation/2","environment":environment,"project_sha256":super::digest(&original),"debug_assertions":cfg!(debug_assertions),"warmup_frames":8,"geometry":{"components":project.components().count(),"symbols":project.all_symbols().count(),"sheets":project.sheets().len(),"footprints":project.boards().map(|b|b.footprints().count()).sum::<usize>()},"results":results}),
    )
}
