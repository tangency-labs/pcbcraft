//! Design rules check results for one board view: a floating card listing findings, canvas
//! markers, and a status summary. Running the check never edits the project.
use egui::{Rect, Ui};
pub(crate) mod jobs;
use pcbcraft_chrome::{properties, shell, units, workspace};
use pcbcraft_engine::{
    board::{
        drc::{CheckSource, Kind, Report},
        view::BoardTarget,
    },
    schematic::Point,
};

/// The last check this view ran, with the revision and board it describes.
#[derive(Debug)]
struct Ran {
    source: CheckSource,
    outdated: bool,
    report: Report,
}

/// Per-view DRC state: whether the card is open, the last result and the focused finding.
#[derive(Debug)]
pub(super) struct Drc {
    pub(super) open: bool,
    collapsed: bool,
    pub(super) rect: Rect,
    result: Option<Ran>,
    focus: Option<usize>,
    queue: jobs::Queue,
    generation: u64,
    waiting: bool,
    notice: Option<String>,
}

impl Default for Drc {
    /// A view starts with no result and the card closed.
    fn default() -> Self {
        Self {
            open: false,
            collapsed: false,
            rect: Rect::NOTHING,
            result: None,
            focus: None,
            queue: Default::default(),
            generation: 0,
            waiting: false,
            notice: None,
        }
    }
}

/// What a person did in the card this frame.
pub(super) enum Action {
    None,
    /// Center the view on a finding.
    Focus(Point),
    Rerun,
}

/// A short human label for each finding kind.
fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Short => "Short",
        Kind::Clearance => "Clearance",
        Kind::TrackWidth => "Track width",
        Kind::ViaDiameter => "Via diameter",
        Kind::ViaDrill => "Via drill",
        Kind::AnnularRing => "Annular ring",
        Kind::Unconnected => "Unconnected",
        Kind::NoOutline => "No outline",
        Kind::OutsideBoard => "Outside board",
        Kind::EdgeClearance => "Edge clearance",
        Kind::CourtyardOverlap => "Courtyard overlap",
    }
}

impl Drc {
    /// Capture read-only checker work without preparing geometry on the interactive path.
    pub(super) fn run(&mut self, session: &pcbcraft_engine::Session, target: BoardTarget) -> Result<(), String> {
        let job = session.design_check_job(&serde_json::json!({"board":target.board})).map_err(|e| e.to_string())?;
        self.generation = self.generation.wrapping_add(1);
        self.queue.submit(jobs::Request { id: self.generation, job });
        self.waiting = true;
        self.notice = None;
        self.focus = None;
        self.open = true;
        self.collapsed = false;
        Ok(())
    }

    /// Invalidate this view's generation before signaling cooperative worker cancellation.
    pub(super) fn cancel(&mut self) {
        if self.waiting {
            self.generation = self.generation.wrapping_add(1);
            self.waiting = false;
            self.notice = Some("Check canceled".into());
            self.queue.cancel();
        }
    }

    /// Poll without blocking; only a current request for this view/board can publish a report.
    pub(super) fn poll(&mut self, session: &pcbcraft_engine::Session, target: Option<BoardTarget>) -> bool {
        if let Some(ran) = &mut self.result {
            ran.outdated = !ran.source.is_current(session);
        }
        if self.waiting && !self.queue.current(session, target) {
            self.cancel();
            self.notice = Some("Board changed; check discarded".into());
        }
        let completion = self.queue.poll();
        let changed = completion.is_some();
        if let Some(completion) = completion
            && completion.id == self.generation
        {
            self.waiting = false;
            if Some(completion.source.target()) != target || !completion.source.is_current(session) {
                self.notice = Some("Board changed; check discarded".into());
            } else {
                match completion.report {
                    Ok(Some(report)) => {
                        self.result = Some(Ran { source: completion.source, outdated: false, report });
                        self.notice = None;
                    }
                    Ok(None) => self.notice = Some("Check canceled".into()),
                    Err(error) => self.notice = Some(error),
                }
            }
        }
        self.queue.busy() || changed
    }

    /// Whether Escape should cancel an outstanding check in the focused view.
    pub(super) fn pending(&self) -> bool {
        self.waiting
    }

    /// A status-bar summary: not run, clean, the finding count, or out of date after an edit.
    pub(super) fn status(&self, board: Option<BoardTarget>, revision: u64) -> String {
        if self.waiting {
            return "DRC checking…".into();
        }
        if let Some(notice) = &self.notice {
            return format!("DRC: {notice}");
        }
        match &self.result {
            Some(ran) if Some(ran.source.target()) == board => {
                let stale = if ran.source.revision() == revision && !ran.outdated { "" } else { " (out of date)" };
                match ran.report.total {
                    0 => format!("DRC clean{stale}"),
                    1 => format!("DRC 1 issue{stale}"),
                    total => format!("DRC {total} issues{stale}"),
                }
            }
            _ => "DRC not run".into(),
        }
    }

    /// Marker positions for this board while the card is open, with the focused finding
    /// flagged. A closed card draws no markers.
    pub(super) fn markers(&self, board: Option<BoardTarget>) -> Vec<(Point, bool)> {
        self.result
            .as_ref()
            .filter(|ran| self.open && !ran.outdated && Some(ran.source.target()) == board)
            .map(|ran| ran.report.violations.iter().enumerate().map(|(index, violation)| (violation.at, self.focus == Some(index))).collect())
            .unwrap_or_default()
    }

    /// Float the results card inside `area`. Clicking a finding focuses it and asks the caller
    /// to center the view there; Run again asks for a fresh check; Close hides the card.
    pub(super) fn show(&mut self, ui: &Ui, area: Rect, revision: u64, unit: units::Unit) -> Action {
        if !self.open {
            self.rect = Rect::NOTHING;
            return Action::None;
        }
        let mut action = Action::None;
        let mut close = false;
        let width = if area.width() < 600. { area.width() } else { 300. };
        let (rect, collapse) = workspace::floating_card_width(ui, ui.scope_id().with("board_drc"), "DRC", area, self.collapsed, true, width, |ui| {
            properties::inspector_body(ui);
            ui.horizontal(|ui| {
                if self.waiting && pcbcraft_chrome::controls::button(ui, "Cancel check").clicked() {
                    self.cancel();
                }
                if pcbcraft_chrome::controls::button(ui, "Run again").clicked() {
                    action = Action::Rerun;
                }
                if pcbcraft_chrome::controls::button(ui, "Close").clicked() {
                    close = true;
                }
            });
            if self.waiting {
                if let Some(progress) = self.queue.progress() {
                    ui.label(progress.stage);
                    let fraction = progress.completed as f32 / progress.total.max(1) as f32;
                    pcbcraft_chrome::controls::progress(ui, "drc", fraction);
                }
            } else if let Some(notice) = &self.notice {
                ui.label(shell::dim(notice));
            }
            let Some(ran) = &self.result else {
                if !self.waiting && self.notice.is_none() {
                    ui.label(shell::dim("Not run"));
                }
                return;
            };
            if ran.source.revision() != revision || ran.outdated {
                ui.label(shell::dim("The board changed since this check. Run again to refresh."));
            }
            ui.label(shell::dim(format!("Revision {}", ran.source.revision())));
            let report = &ran.report;
            if report.total == 0 {
                ui.label("No problems found with the current rules.");
            } else {
                let summary: Vec<_> = report.counts.iter().map(|(kind, count)| format!("{count} {}", label(*kind).to_lowercase())).collect();
                ui.label(format!("{} issue{}: {}", report.total, if report.total == 1 { "" } else { "s" }, summary.join(", ")));
            }
            ui.label(shell::dim(format!(
                "Clearance {} · track ≥ {} · edge {} · ring ≥ {}",
                units::label(report.rules.clearance_nm, unit),
                units::label(report.rules.min_track_width_nm, unit),
                units::label(report.rules.edge_clearance_nm, unit),
                units::label(report.rules.min_annular_ring_nm, unit),
            )));
            egui::ScrollArea::vertical().id_salt("board_drc_list").max_height(320.).show(ui, |ui| {
                for (index, violation) in report.violations.iter().enumerate() {
                    let text = format!("{} · {}", label(violation.kind), violation.message);
                    if ui.selectable_label(self.focus == Some(index), text).clicked() {
                        self.focus = Some(index);
                        action = Action::Focus(violation.at);
                    }
                }
                if report.violations.len() < report.total {
                    ui.label(shell::dim(format!("{} more not listed", report.total - report.violations.len())));
                }
            });
        });
        self.rect = rect;
        if collapse {
            self.collapsed = !self.collapsed;
        }
        if close {
            self.cancel();
            self.open = false;
            self.rect = Rect::NOTHING;
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::Session;
    use serde_json::json;
    /// Checks for separate views never share a completion generation or a report slot.
    #[test]
    fn independent_views_replacements_stale_edits_and_focus_cancellation() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":true})).unwrap();
        let target = session.project().unwrap().board_target().unwrap();
        let snapshot = session.snapshot().unwrap();
        let mut first = Drc::default();
        let mut second = Drc::default();
        first.run(&session, target).unwrap();
        second.run(&session, target).unwrap();
        first.cancel();
        for _ in 0..100 {
            first.poll(&session, Some(target));
            second.poll(&session, Some(target));
        }
        assert!(first.result.is_none());
        assert!(second.result.is_some());
        first.run(&session, target).unwrap();
        first.run(&session, target).unwrap();
        for _ in 0..100 {
            first.poll(&session, Some(target));
        }
        assert!(first.result.is_some());
        assert!(!first.pending());
        first.run(&session, target).unwrap();
        session.open_snapshot(snapshot);
        for _ in 0..100 {
            first.poll(&session, Some(target));
        }
        assert!(!first.pending());
        assert!(first.result.as_ref().unwrap().outdated);
        assert!(first.markers(Some(target)).is_empty());
        assert_eq!(first.notice.as_deref(), Some("Board changed; check discarded"));
    }
}
