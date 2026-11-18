//! Paced pointer feedback through production frames and native snapshot workers.
use super::*;
use egui::{Event, Key, Modifiers, PointerButton, RawInput, Rect, Shape, pos2, vec2};

/// Frame-level shape feedback on unchanged command-generated projects. This distinguishes
/// continuous target updates from stationary settling, without claiming GPU presentation.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    use std::time::{Duration, Instant};
    environment.validate()?;
    if !(1..=20).contains(&iterations) {
        return Err("pointer iterations must be from 1 to 20".into());
    }
    let artifact = load_artifact(artifact_json)?;
    let mut samples: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut runs = Vec::new();
    for _ in 0..iterations {
        let (mut app, _) = replay::Driver::prepare(&artifact)?;
        let original = pcbcraft_engine::persistence::encode_packed(app.session.snapshot().ok_or("missing project")?).map_err(|e| e.to_string())?;
        let ctx = egui::Context::default();
        pcbcraft_chrome::install(&ctx);
        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::MIN);
        let began = Instant::now();
        let mut next = Instant::now();
        let mut frame = |app: &mut crate::PcbApp, events: Vec<Event>| {
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
            next = Instant::now() + Duration::from_secs_f64(1. / 60.);
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1400., 900.))),
                time: Some(began.elapsed().as_secs_f64()),
                focused: true,
                events,
                ..Default::default()
            };
            let start = Instant::now();
            let output = ctx.run_ui(raw, |ui| app.frame(ui));
            (output, start.elapsed().as_secs_f64() * 1000.)
        };
        for _ in 0..8 {
            frame(&mut app, Vec::new());
        }
        let keys = |key| {
            vec![
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
            ]
        };
        frame(&mut app, keys(Key::X));
        let rect = app.views.active.camera.performance_rect.ok_or("missing canvas")?;
        let head = app.views.active.camera.screen(Point { x_nm: 0, y_nm: -5_080_000 }, rect);
        frame(
            &mut app,
            vec![
                Event::PointerMoved(head),
                Event::PointerButton { pos: head, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                Event::PointerButton { pos: head, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
            ],
        );
        if !app.views.active.board.tracks.routing() {
            return Err("pointer test failed to start a route".into());
        }
        let warm = app.views.active.camera.screen(Point { x_nm: 1_270_000, y_nm: -5_080_000 }, rect);
        let mut settled = false;
        for _ in 0..60 {
            let (output, _) = frame(&mut app, vec![Event::PointerMoved(warm)]);
            if preview_at(&output, warm) {
                settled = true;
                break;
            }
        }
        if !settled {
            return Err("initial preview did not settle within 60 frames".into());
        }
        let mut current = 0;
        let mut endpoints = Vec::new();
        for i in 1..=24 {
            let target = Point { x_nm: 1_270_000 + i * 254_000, y_nm: -5_080_000 };
            let screen = app.views.active.camera.screen(target, rect);
            let (output, ms) = frame(&mut app, vec![Event::PointerMoved(screen)]);
            samples.entry("pointer.moving.ui".into()).or_default().push(ms);
            let fresh = preview_at(&output, screen);
            current += usize::from(fresh);
            endpoints.push(json!({"target":target,"current_in_input_frame":fresh}));
            if let Some(error) = &app.message {
                return Err(format!("pointer route failed: {error}"));
            }
        }
        let end = app.views.active.camera.screen(Point { x_nm: 7_366_000, y_nm: -5_080_000 }, rect);
        let mut extra = 0;
        for _ in 0..60 {
            let (output, _) = frame(&mut app, Vec::new());
            extra += 1;
            if preview_at(&output, end) {
                break;
            }
            if extra == 60 {
                return Err("moving preview failed to catch up after input stopped".into());
            }
        }
        frame(&mut app, keys(Key::Escape));
        frame(&mut app, keys(Key::Escape));
        if pcbcraft_engine::persistence::encode_packed(app.session.snapshot().ok_or("missing project")?).map_err(|e| e.to_string())? != original {
            return Err("pointer feedback altered authored project bytes".into());
        }
        runs.push(json!({"pointer_frames":24,"current_in_input_frame":current,"extra_frames_to_observe_final_target":extra,"frames":endpoints}));
    }
    Ok(
        json!({"schema":"pcbcraft-pointer-feedback/1","environment":environment,"artifact_sha256":digest(artifact_json.as_bytes()),"debug_assertions":cfg!(debug_assertions),"iterations":iterations,"native_worker_path":!cfg!(test),"pace_hz":60,"runs":runs,"metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),"correctness":"real route input; preview copper endpoint inspected; unchanged authored bytes after cancellation","unmeasured":["GPU submission/readback","compositor presentation","physical input-to-visible latency","browser"]}),
    )
}

/// Inspect the actual preview copper rather than a scheduler's readiness label.
#[cfg(not(target_arch = "wasm32"))]
fn preview_at(output: &egui::FullOutput, end: egui::Pos2) -> bool {
    let tint = pcbcraft_chrome::Tokens::get().board_front.lerp_to_gamma(egui::Color32::WHITE, 0.35);
    output.shapes.iter().any(|shape| matches!(&shape.shape,Shape::LineSegment{points,stroke} if stroke.color==tint && points[1].distance(end)<0.01))
}
