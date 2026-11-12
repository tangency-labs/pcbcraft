//! Deterministic input frames through PcbApp, with state assertions to reject missed gestures.
use super::{Artifact, Result, fixture, timed};
use crate::{PcbApp, editor_view::Workspace};
use egui::{Event, Key, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};
use pcbcraft_engine::{
    board::{
        setup::{LayerPurpose, Side},
        track::MOVE_TRACKS,
    },
    persistence::{decode_packed, encode_packed},
    schematic::Point,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

pub(super) const WARMUP: usize = 8;

/// One 60 Hz logical input frame; wall-clock measurements do not sleep to simulate pacing.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    id: String,
    action: Action,
    routing_after: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Key { key: ReplayKey, pressed: bool },
    Pointer { at: Point, pressed: Option<bool> },
    SharedEdit,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplayKey {
    Route,
    Cancel,
    BackLayer,
}
impl ReplayKey {
    /// Map stable trace vocabulary to production keyboard events.
    fn egui(self) -> Key {
        match self {
            Self::Route => Key::X,
            Self::Cancel => Key::Escape,
            Self::BackLayer => Key::PageDown,
        }
    }
}
impl Step {
    /// Versioned reversal, layer-transition, cancellation and concurrent-edit authority trace.
    pub(super) fn standard() -> Vec<Self> {
        let mut trace = Vec::new();
        let key = |trace: &mut Vec<Self>, id: &str, key: ReplayKey, routing_after: bool| {
            for pressed in [true, false] {
                trace.push(Self { id: format!("{id}.{}", if pressed { "down" } else { "up" }), action: Action::Key { key, pressed }, routing_after });
            }
        };
        let pointer = |trace: &mut Vec<Self>, id: &str, x: i64, pressed: Option<bool>, routing_after: bool| {
            trace.push(Self { id: id.into(), action: Action::Pointer { at: Point { x_nm: x, y_nm: -5_080_000 }, pressed }, routing_after })
        };
        key(&mut trace, "arm", ReplayKey::Route, false);
        pointer(&mut trace, "start.down", 0, Some(true), false);
        pointer(&mut trace, "start.up", 0, Some(false), true);
        for (i, x) in [2_540_000, 5_080_000, 7_620_000, 5_080_000, 2_540_000, 7_620_000].into_iter().enumerate() {
            pointer(&mut trace, &format!("reverse.{i}"), x, None, true);
        }
        key(&mut trace, "layer", ReplayKey::BackLayer, true);
        key(&mut trace, "cancel", ReplayKey::Cancel, false);
        // Wait beyond the double-click interval in logical time before starting a new route.
        for i in 0..30 {
            pointer(&mut trace, &format!("idle.{i}"), 0, None, false);
        }
        pointer(&mut trace, "restart.down", 0, Some(true), false);
        pointer(&mut trace, "restart.up", 0, Some(false), true);
        pointer(&mut trace, "pending", 5_080_000, None, true);
        trace.push(Self { id: "shared-edit".into(), action: Action::SharedEdit, routing_after: false });
        key(&mut trace, "exit", ReplayKey::Cancel, false);
        trace
    }
}

/// Paint and tessellate a real editor frame. These CPU boundaries exclude event-loop waits,
/// renderer submission, compositor presentation and physical visibility.
fn frame(app: &mut PcbApp, ctx: &egui::Context, events: Vec<Event>, number: usize, samples: &mut BTreeMap<String, Vec<f64>>, id: &str) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))),
        time: Some(number as f64 / 60.0),
        focused: true,
        events,
        ..Default::default()
    };
    let mut output = timed(samples, &format!("replay.{id}.ui"), || ctx.run_ui(input, |ui| app.frame(ui)));
    output.textures_delta.clear();
    timed(samples, &format!("replay.{id}.tessellate"), || std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point)));
}

/// Restore identical project bytes for each replay, warming fonts/layout separately. A shared
/// command is injected while a route is pending to exercise revision invalidation, not transport.
pub(super) fn run(artifact: &Artifact, samples: &mut BTreeMap<String, Vec<f64>>) -> Result<()> {
    let mut app = PcbApp::default();
    app.session.open_snapshot(decode_packed(&artifact.project)?.project);
    app.views.active.bind_opened_project(&app.session);
    app.views.active.switch(Workspace::Board, &app.session);
    let source = app.session.snapshot().ok_or("missing replay project")?;
    let board = source.board_view(artifact.board).ok_or("missing replay board")?;
    let top = board
        .setup()
        .and_then(|s| s.layers().iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Front)))
        .ok_or("missing copper")?
        .id;
    app.views.active.board.active_layer = Some(top.to_string());
    app.views.active.filters_open = false;
    app.views.active.camera.center_on(Point { x_nm: 10_160_000, y_nm: 0 });
    let ctx = egui::Context::default();
    pcbcraft_chrome::install(&ctx);
    let mut warmup = BTreeMap::new();
    for i in 0..WARMUP {
        frame(&mut app, &ctx, Vec::new(), i, &mut warmup, "warmup");
    }
    let original = encode_packed(app.session.snapshot().ok_or("missing replay source")?)?;
    let revision = app.session.revision();
    let mut edits = 0;
    for (index, step) in artifact.trace.iter().enumerate() {
        let mut events = Vec::new();
        match &step.action {
            Action::Key { key, pressed } => {
                events.push(Event::Key { key: key.egui(), physical_key: None, pressed: *pressed, repeat: false, modifiers: Modifiers::NONE })
            }
            Action::Pointer { at, pressed } => {
                let rect = app.views.active.camera.performance_rect.ok_or("board canvas was not painted")?;
                let pos = app.views.active.camera.screen(*at, rect);
                if !rect.contains(pos) {
                    return Err(format!("{}: trace point is outside canvas", step.id));
                }
                events.push(Event::PointerMoved(pos));
                if let Some(pressed) = pressed {
                    events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: *pressed, modifiers: Modifiers::NONE });
                }
            }
            Action::SharedEdit => {
                if !app.views.active.board.tracks.routing() {
                    return Err("shared edit needs a pending route".into());
                }
                let track = board.tracks().first().ok_or("missing replay track")?.id;
                timed(samples, "replay.shared-edit.command", || {
                    fixture::apply(&mut app.session, MOVE_TRACKS, json!({"board":artifact.board.board,"tracks":[track],"dx_nm":0,"dy_nm":100_000}))
                })?;
                edits += 1;
            }
        }
        frame(&mut app, &ctx, events, index + WARMUP, samples, &step.id);
        if app.views.active.board.tracks.routing() != step.routing_after {
            return Err(format!("{}: route state mismatch (input may have missed canvas)", step.id));
        }
        if let Some(message) = &app.message {
            return Err(format!("{}: {message}", step.id));
        }
        if step.id == "layer.down" && app.views.active.board.active_layer.as_deref() == Some(&top.to_string()) {
            return Err("layer key did not change the route layer".into());
        }
        if step.id == "cancel.up" && encode_packed(app.session.snapshot().ok_or("missing project after cancel")?)? != original {
            return Err("cancel published authored changes".into());
        }
    }
    if app.session.revision() != revision + edits || app.views.active.board.tracks.busy() {
        return Err("replay ended with unexpected edits or an active tool".into());
    }
    Ok(())
}
