//! Queued UI input for detached editor windows.
//!
//! Detached editors are immediate child viewports: they take their input inside window 0's
//! pass, so eframe's raw input hook never sees it. Before showing a child, the app stages
//! that pass's events and checks; egui's plugin input hook applies them before the pass
//! processes input, and the child's own UI pass then records what happened.
use egui::{Event, RawInput, ViewportId};

use super::{Checks, interrupted};
use crate::{PcbApp, editor_layout::HostId, editor_windows::viewport};

/// One child pass's input, prepared by the app from its queue.
struct Stage {
    viewport: ViewportId,
    release: Vec<Event>,
    frame: Vec<Event>,
    checks: Option<Checks>,
}

/// What the staged pass received.
enum Outcome {
    Issued { release: Vec<Event>, frame: Vec<Event> },
    Interrupted(&'static str),
}

/// Applies staged events to exactly the addressed child's next pass; other passes are untouched.
#[derive(Default)]
pub(super) struct ChildInput {
    stage: Option<Stage>,
    outcome: Option<Outcome>,
}

impl egui::Plugin for ChildInput {
    fn debug_name(&self) -> &'static str {
        "pcbcraft_child_input"
    }

    /// Physical input and window state win: an interrupted job receives nothing in this pass.
    fn input_hook(&mut self, ctx: &egui::Context, input: &mut RawInput) {
        let Some(stage) = self.stage.take_if(|stage| stage.viewport == input.viewport_id) else { return };
        // egui derives the pass's scale the same way after this hook.
        let scale = ctx.zoom_factor() * input.viewport().native_pixels_per_point.unwrap_or(1.0);
        if let Some(reason) = stage.checks.and_then(|checks| interrupted(input, scale, checks)) {
            self.outcome = Some(Outcome::Interrupted(reason));
            return;
        }
        input.events.splice(0..0, stage.release.iter().cloned());
        input.events.extend_from_slice(&stage.frame);
        self.outcome = Some(Outcome::Issued { release: stage.release, frame: stage.frame });
    }
}

impl PcbApp {
    /// Stage `host`'s queued input before its window is shown. The job is checked against
    /// application state here, and against the window's raw input by the plugin hook.
    pub(crate) fn automation_stage(&mut self, ctx: &egui::Context, host: HostId) {
        if let Some(reason) = self.automation_invalid() {
            self.automation_stop(reason);
        }
        let (release, frame) = self.automation_events(viewport(host));
        if release.is_empty() && frame.is_empty() {
            return;
        }
        let checks = self.automation.job.as_ref().filter(|job| job.host == host).map(|job| Checks {
            rect: job.rect,
            scale: job.scale,
            held: job.held.is_some(),
            pointer_down: self.automation.snapshots.get(&host).is_some_and(|snapshot| snapshot.pointer_down),
        });
        ctx.add_plugin(ChildInput::default());
        ctx.with_plugin(|plugin: &mut ChildInput| {
            plugin.stage = Some(Stage { viewport: viewport(host), release, frame, checks });
            plugin.outcome = None;
        });
    }

    /// At the start of `host`'s own pass, record what its input hook did. A stage the pass
    /// never consumed (no native window yet) is dropped and retried on the next pass.
    pub(crate) fn automation_child_input(&mut self, ctx: &egui::Context, host: HostId) {
        let outcome = ctx.with_plugin(|plugin: &mut ChildInput| {
            plugin.stage = None;
            plugin.outcome.take()
        });
        let issued = matches!(outcome, Some(Some(Outcome::Issued { .. })));
        pcbcraft_chrome::automation::mark_synthetic_input(ctx, issued);
        match outcome.flatten() {
            Some(Outcome::Issued { release, frame }) => self.automation_issued(ctx, viewport(host), release, frame),
            Some(Outcome::Interrupted(reason)) if self.automation.job.as_ref().is_some_and(|job| job.host == host) => {
                self.automation_stop(reason);
                ctx.request_repaint_of(viewport(host));
            }
            _ => {}
        }
    }

    /// Drop a stage whose pass did not run, so it cannot reach a later pass of the same window.
    pub(crate) fn automation_unstage(&mut self, ctx: &egui::Context) {
        ctx.with_plugin(|plugin: &mut ChildInput| plugin.stage = None);
    }
}
