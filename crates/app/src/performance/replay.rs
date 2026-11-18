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
/// Stable identity for the exact versioned route, reversal, layer, cancellation and edit steps.
pub(super) const STANDARD_TRACE_ID: &str = "route-reverse-layer-cancel-edit-v1";
pub(super) const COMMIT_TRACE_ID: &str = "route-contact-commit-history-v1";

/// One 60 Hz logical input frame; wall-clock measurements do not sleep to simulate pacing.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    pub(super) id: String,
    action: Action,
    pub(super) routing_after: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Key { key: ReplayKey, pressed: bool },
    Pointer { at: Point, pressed: Option<bool> },
    SharedEdit,
    Contact { target: bool, pressed: Option<bool> },
    ContactPreview,
    HistoryCheck,
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
    /// Press/release pairs must not be stretched by GPU readback: a slow adapter could
    /// otherwise turn an ordinary click into a long press. Capture releases and moves.
    pub(super) fn capture_after(&self) -> bool {
        !matches!(
            self.action,
            Action::Key { pressed: true, .. } | Action::Pointer { pressed: Some(true), .. } | Action::Contact { pressed: Some(true), .. }
        )
    }

    /// Short companion trace: connect a saved airwire through real pointer input, then
    /// check exact one-step undo/redo and reopen outside the commit timing boundary.
    pub(super) fn commit() -> Vec<Self> {
        let mut steps = Vec::new();
        for pressed in [true, false] {
            steps.push(Self {
                id: format!("arm.{}", if pressed { "down" } else { "up" }),
                action: Action::Key { key: ReplayKey::Route, pressed },
                routing_after: false,
            });
        }
        for pressed in [true, false] {
            steps.push(Self {
                id: format!("contact.start.{}", if pressed { "down" } else { "up" }),
                action: Action::Contact { target: false, pressed: Some(pressed) },
                routing_after: !pressed,
            });
        }
        steps.push(Self { id: "contact.preview".into(), action: Action::ContactPreview, routing_after: true });
        for pressed in [true, false] {
            steps.push(Self {
                id: format!("contact.commit.{}", if pressed { "down" } else { "up" }),
                action: Action::Contact { target: true, pressed: Some(pressed) },
                routing_after: pressed,
            });
        }
        steps.push(Self { id: "commit.history".into(), action: Action::HistoryCheck, routing_after: false });
        for pressed in [true, false] {
            steps.push(Self {
                id: format!("exit.{}", if pressed { "down" } else { "up" }),
                action: Action::Key { key: ReplayKey::Cancel, pressed },
                routing_after: false,
            });
        }
        steps
    }

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

/// Physical contacts and publication baseline, captured only for contact-dependent traces.
struct Contacts {
    points: [Point; 2],
    airwires: usize,
    original_tracks: std::collections::BTreeSet<uuid::Uuid>,
}

/// Shared trace driver for CPU replay and actual native/browser windows. Every input
/// has the same production state assertions, regardless of the rendering integration.
pub(super) struct Driver {
    original: Vec<u8>,
    revision: u64,
    edits: u64,
    track: uuid::Uuid,
    top: String,
    contacts: Option<Contacts>,
}

impl Driver {
    /// Restore canonical bytes and the addressed board, without restoring user preferences.
    /// Resolve airwires only for contact-dependent actions; other traces need no connection.
    pub(super) fn prepare(artifact: &Artifact) -> Result<(PcbApp, Self)> {
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
            .id
            .to_string();
        let track = board.tracks().first().ok_or("missing replay track")?.id.to_owned();
        app.views.active.board.active_layer = Some(top.clone());
        app.views.active.filters_open = false;
        app.views.active.camera.center_on(Point { x_nm: 10_160_000, y_nm: 0 });
        let contacts =
            if artifact.trace.iter().any(|step| matches!(step.action, Action::Contact { .. } | Action::ContactPreview | Action::HistoryCheck)) {
                let lines = board.ratsnest().ok_or("missing replay airwires")?.lines;
                // Legacy center-only pad markers are not conductive copper. Choose the first saved
                // airwire with physical copper at both ends, without consulting solve results.
                let physical: Vec<_> = board.footprints().flat_map(|f| f.pads).filter(|p| p.copper.is_some()).map(|p| p.center).collect();
                let first = lines
                    .iter()
                    .find(|line| physical.contains(&line.from.position) && physical.contains(&line.to.position))
                    .ok_or("missing supported replay connection")?;
                Some(Contacts {
                    points: [first.from.position, first.to.position],
                    airwires: lines.len(),
                    original_tracks: board.tracks().iter().map(|track| track.id).collect(),
                })
            } else {
                None
            };
        let original = encode_packed(source)?;
        let revision = app.session.revision();
        Ok((app, Self { original, revision, edits: 0, track, top, contacts }))
    }

    /// Translate one addressed trace action; shared edits use the revision-checked command path.
    pub(super) fn input(
        &mut self,
        app: &mut PcbApp,
        artifact: &Artifact,
        index: usize,
        samples: &mut BTreeMap<String, Vec<f64>>,
    ) -> Result<Vec<Event>> {
        let step = artifact.trace.get(index).ok_or("trace index outside artifact")?;
        let mut events = Vec::new();
        match &step.action {
            Action::Key { key, pressed } => {
                events.push(Event::Key { key: key.egui(), physical_key: None, pressed: *pressed, repeat: false, modifiers: Modifiers::NONE })
            }
            Action::Pointer { at, pressed } => events = Self::pointer(app, *at, *pressed)?,
            Action::Contact { target, pressed } => {
                let contacts = self.contacts.as_ref().ok_or("contact input has no captured connection")?;
                events = Self::pointer(app, contacts.points[usize::from(*target)], *pressed)?;
                if *target && *pressed == Some(false) {
                    self.edits += 1;
                }
            }
            Action::ContactPreview => {
                let [from, to] = self.contacts.as_ref().ok_or("contact preview has no captured connection")?.points;
                events = Self::pointer(app, Point { x_nm: (from.x_nm + to.x_nm) / 2, y_nm: (from.y_nm + to.y_nm) / 2 - 1_270_000 }, None)?;
            }
            Action::HistoryCheck => {
                let contacts = self.contacts.as_ref().ok_or("contact history has no captured connection")?;
                let source = app.session.snapshot().ok_or("missing committed project")?;
                let board = source.board_view(artifact.board).ok_or("missing committed board")?;
                let added: std::collections::BTreeSet<_> =
                    board.tracks().iter().filter(|t| !contacts.original_tracks.contains(&t.id)).map(|t| t.id.to_string()).collect();
                if added.is_empty() || board.ratsnest().ok_or("missing routed airwires")?.lines.len() >= contacts.airwires {
                    return Err("commit did not connect the saved airwire".into());
                }
                let report = board.check_design(board.effective_rules().values);
                if report.violations.iter().any(|v| {
                    matches!(
                        v.kind,
                        pcbcraft_engine::board::drc::Kind::Clearance
                            | pcbcraft_engine::board::drc::Kind::Short
                            | pcbcraft_engine::board::drc::Kind::TrackWidth
                            | pcbcraft_engine::board::drc::Kind::EdgeClearance
                            | pcbcraft_engine::board::drc::Kind::OutsideBoard
                    ) && v.items.iter().any(|id| added.contains(id))
                }) {
                    return Err("committed route fails full physical DRC".into());
                }
                let committed = encode_packed(source)?;
                if encode_packed(decode_packed(&committed)?.project)? != committed {
                    return Err("committed route did not reopen exactly".into());
                }
                app.session.apply("edit.undo", &json!({})).map_err(|e| e.to_string())?;
                if encode_packed(app.session.snapshot().ok_or("missing undo project")?)? != self.original {
                    return Err("one undo did not restore the original project".into());
                }
                app.session.apply("edit.redo", &json!({})).map_err(|e| e.to_string())?;
                if encode_packed(app.session.snapshot().ok_or("missing redo project")?)? != committed {
                    return Err("redo did not restore committed bytes".into());
                }
                self.edits += 2;
            }
            Action::SharedEdit => {
                if !app.views.active.board.tracks.routing() {
                    return Err("shared edit needs a pending route".into());
                }
                timed(samples, "replay.shared-edit.command", || {
                    fixture::apply(
                        &mut app.session,
                        MOVE_TRACKS,
                        json!({"board":artifact.board.board,"tracks":[self.track],"dx_nm":0,"dy_nm":100_000}),
                    )
                })?;
                self.edits += 1;
            }
        }
        Ok(events)
    }

    /// Resolve a board point through the real canvas, refusing clipped diagnostic input.
    fn pointer(app: &PcbApp, at: Point, pressed: Option<bool>) -> Result<Vec<Event>> {
        let rect = app.views.active.camera.performance_rect.ok_or("board canvas was not painted")?;
        let pos = app.views.active.camera.screen(at, rect);
        if !rect.contains(pos) {
            return Err("trace point is outside canvas".into());
        }
        let mut events = vec![Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
        Ok(events)
    }

    /// Reserve exactly one shared edit for delivery through the actual native MCP transport.
    /// The normal frame path owns execution and revision invalidation while the client waits.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn remote_request(&mut self, app: &PcbApp, artifact: &Artifact, index: usize) -> Result<Option<serde_json::Value>> {
        if !matches!(artifact.trace[index].action, Action::SharedEdit) {
            return Ok(None);
        }
        if !app.views.active.board.tracks.routing() {
            return Err("remote edit needs a pending route".into());
        }
        self.edits += 1;
        Ok(Some(json!({"method":"engine.execute","params":{"command":MOVE_TRACKS,"inspect":false,
            "params":{"board":artifact.board.board,"tracks":[self.track],"dx_nm":0,"dy_nm":100_000,"expected_revision":app.session.revision()}}})))
    }

    /// Wait for validated layer/preview results and committed revision in their respective
    /// trace steps. Other actions assert state on the first accepted pass.
    pub(super) fn pending(&self, app: &PcbApp, artifact: &Artifact, index: usize) -> bool {
        match artifact.trace[index].id.as_str() {
            "layer.down" | "layer.up" => app.views.active.board.tracks.committing(),
            "contact.preview" => app.message.is_none() && app.views.active.board.tracks.preparation_status().as_deref() != Some("Route prepared"),
            "contact.commit.up" => app.message.is_none() && app.session.revision() < self.revision + self.edits,
            _ => false,
        }
    }

    /// Drive a pending validation/publication step with a wall-clock and frame bound.
    /// No input is repeated, and the normal final layer/cancellation assertions still apply.
    fn settle(
        &self,
        app: &mut PcbApp,
        ctx: &egui::Context,
        artifact: &Artifact,
        index: usize,
        frame_id: &mut usize,
        samples: &mut BTreeMap<String, Vec<f64>>,
    ) -> Result<()> {
        let id = &artifact.trace[index].id;
        if !matches!(id.as_str(), "layer.down" | "layer.up" | "contact.preview" | "contact.commit.up") {
            return Ok(());
        }
        let started = super::clock_ms();
        let mut settling = BTreeMap::new();
        for _ in 0..10_000 {
            if !self.pending(app, artifact, index) {
                break;
            }
            if super::clock_ms() - started > 5_000. {
                return Err("route validation did not settle within five seconds".into());
            }
            frame(app, ctx, Vec::new(), *frame_id, &mut settling, &format!("settle.{id}"));
            *frame_id += 1;
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::yield_now();
        }
        if self.pending(app, artifact, index) {
            return Err("route validation exceeded its frame bound".into());
        }
        for suffix in ["ui", "tessellate"] {
            let key = format!("replay.settle.{id}.{suffix}");
            let total = settling.remove(&key).map_or(0., |values| values.into_iter().sum());
            samples.entry(key).or_default().push(total);
        }
        Ok(())
    }

    /// Reject missed routing input, ineffective layer changes and cancellation that authors data.
    pub(super) fn verify(&self, app: &PcbApp, artifact: &Artifact, index: usize) -> Result<()> {
        let step = &artifact.trace[index];
        if app.views.active.board.tracks.routing() != step.routing_after {
            return Err(format!("{}: route state mismatch (input may have missed canvas)", step.id));
        }
        if let Some(message) = &app.message {
            return Err(format!("{}: {message}", step.id));
        }
        if matches!(step.id.as_str(), "layer.down" | "layer.up") && app.views.active.board.active_layer.as_deref() == Some(&self.top) {
            return Err("layer key did not change the route layer".into());
        }
        if step.id == "contact.preview" && app.views.active.board.tracks.preparation_status().as_deref() != Some("Route prepared") {
            return Err("preview did not finish validated preparation".into());
        }
        if step.id == "contact.commit.up" && app.session.revision() != self.revision + self.edits {
            return Err("route did not publish exactly one revision".into());
        }
        if step.id == "cancel.up" && encode_packed(app.session.snapshot().ok_or("missing project after cancel")?)? != self.original {
            return Err("cancel published authored changes".into());
        }
        Ok(())
    }

    /// Require exactly the trace's shared edits and no stranded route at completion.
    pub(super) fn finish(&self, app: &PcbApp) -> Result<()> {
        if app.session.revision() != self.revision + self.edits || app.views.active.board.tracks.busy() {
            return Err("replay ended with unexpected edits or an active tool".into());
        }
        Ok(())
    }
}

/// Warm fonts/layout, then replay the versioned trace through the real editor CPU path.
pub(super) fn run(artifact: &Artifact, samples: &mut BTreeMap<String, Vec<f64>>) -> Result<()> {
    let (mut app, mut driver) = Driver::prepare(artifact)?;
    let ctx = egui::Context::default();
    pcbcraft_chrome::install(&ctx);
    let mut warmup = BTreeMap::new();
    for i in 0..WARMUP {
        frame(&mut app, &ctx, Vec::new(), i, &mut warmup, "warmup");
    }
    let mut frame_id = WARMUP;
    for (index, step) in artifact.trace.iter().enumerate() {
        let events = driver.input(&mut app, artifact, index, samples)?;
        frame(&mut app, &ctx, events, frame_id, samples, &step.id);
        frame_id += 1;
        driver.settle(&mut app, &ctx, artifact, index, &mut frame_id, samples)?;
        driver.verify(&app, artifact, index)?;
    }
    driver.finish(&app)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// No-contact replays need only front copper and an editable track. The fixture's
    /// native bytes come from commands; contact traces still reject its absent airwires.
    #[test]
    fn review408_noncontact_traces_do_not_require_airwires() {
        let mut session = pcbcraft_engine::Session::default();
        session.apply("project.new", &json!({"create_board":true})).unwrap();
        let target = session.project().unwrap().board_target().unwrap();
        fixture::apply(&mut session, pcbcraft_engine::INITIALIZE_BOARD_SETUP, json!({"board":target.board,"copper_layers":2})).unwrap();
        let layer = session
            .project()
            .unwrap()
            .board_view(target)
            .unwrap()
            .layers()
            .into_iter()
            .find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Front))
            .unwrap()
            .id;
        fixture::apply(
            &mut session,
            pcbcraft_engine::ADD_TRACKS,
            json!({"board":target.board,"layer":layer,"width_nm":250000,"points":[{"x_nm":50000000,"y_nm":0},{"x_nm":51000000,"y_nm":0}]}),
        )
        .unwrap();
        assert!(session.project().unwrap().board_view(target).unwrap().ratsnest().is_none_or(|r| r.lines.is_empty()));
        let mut artifact = super::super::load_artifact(include_str!("../../../../tools/performance/fixtures/reference-v1.json")).unwrap();
        artifact.board = target;
        artifact.project = encode_packed(session.snapshot().unwrap()).unwrap();
        artifact.project_sha256 = super::super::digest(&artifact.project);
        run(&artifact, &mut BTreeMap::new()).unwrap();
        artifact.trace_id = "custom-idle-v1".into();
        artifact.trace = vec![Step {
            id: "idle".into(),
            action: Action::Pointer { at: Point { x_nm: 0, y_nm: -5_080_000 }, pressed: None },
            routing_after: false,
        }];
        run(&artifact, &mut BTreeMap::new()).unwrap();
        artifact.trace = Step::commit();
        assert!(Driver::prepare(&artifact).is_err(), "contact replay still needs a supported connection");
    }

    /// Real renderers can take longer than a click threshold between passes. Deliver paired
    /// press/release events together and still require route, layer, cancel and shared-edit checks.
    #[test]
    fn atomic_input_pairs_keep_route_and_commit_traces_correct() {
        for commit in [false, true] {
            let mut artifact = super::super::load_artifact(&super::super::generate("reference-v1", "test").unwrap()).unwrap();
            if commit {
                artifact.trace_id = COMMIT_TRACE_ID.into();
                artifact.trace = Step::commit();
            }
            let (mut app, mut driver) = Driver::prepare(&artifact).unwrap();
            let ctx = egui::Context::default();
            ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::MIN);
            pcbcraft_chrome::install(&ctx);
            let mut samples = BTreeMap::new();
            for i in 0..WARMUP {
                frame(&mut app, &ctx, Vec::new(), i, &mut samples, "warmup");
            }
            let mut index = 0;
            let mut frame_id = WARMUP;
            while index < artifact.trace.len() {
                let mut events = driver.input(&mut app, &artifact, index, &mut samples).unwrap();
                if !artifact.trace[index].capture_after() {
                    index += 1;
                    events.extend(driver.input(&mut app, &artifact, index, &mut samples).unwrap());
                }
                frame(&mut app, &ctx, events, frame_id, &mut samples, &artifact.trace[index].id);
                frame_id += 1;
                driver.settle(&mut app, &ctx, &artifact, index, &mut frame_id, &mut samples).unwrap();
                driver.verify(&app, &artifact, index).unwrap();
                index += 1;
            }
            driver.finish(&app).unwrap();
        }
    }
}
