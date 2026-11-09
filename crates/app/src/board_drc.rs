//! Design rules check results for one board view: a floating card listing findings, canvas
//! markers, and a status summary. Running the check never edits the project.
use egui::{Rect, Ui};
use pcbcraft_chrome::{properties, shell, units, workspace};
use pcbcraft_engine::{
    board::{
        drc::{Kind, Report, Rules},
        view::{BoardTarget, BoardView},
    },
    schematic::Point,
};

/// The last check this view ran, with the revision and board it describes.
#[derive(Debug)]
struct Ran {
    revision: u64,
    board: BoardTarget,
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
}

impl Default for Drc {
    /// A view starts with no result and the card closed.
    fn default() -> Self {
        Self { open: false, collapsed: false, rect: Rect::NOTHING, result: None, focus: None }
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
        Kind::Clearance => "Clearance",
        Kind::TrackWidth => "Track width",
        Kind::AnnularRing => "Annular ring",
        Kind::Unconnected => "Unconnected",
        Kind::NoOutline => "No outline",
        Kind::OutsideBoard => "Outside board",
        Kind::EdgeClearance => "Edge clearance",
        Kind::CourtyardOverlap => "Courtyard overlap",
    }
}

impl Drc {
    /// Check `view` with the default rules and open the card on the result. Nothing is edited.
    pub(super) fn run(&mut self, view: BoardView<'_>, revision: u64) {
        self.result = Some(Ran { revision, board: view.target, report: view.check_design(Rules::default()) });
        self.focus = None;
        self.open = true;
        self.collapsed = false;
    }

    /// A status-bar summary: not run, clean, the finding count, or out of date after an edit.
    pub(super) fn status(&self, board: Option<BoardTarget>, revision: u64) -> String {
        match &self.result {
            Some(ran) if Some(ran.board) == board => {
                let stale = if ran.revision == revision { "" } else { " (out of date)" };
                match ran.report.total {
                    0 => format!("DRC clean{stale}"),
                    1 => format!("DRC 1 issue{stale}"),
                    total => format!("DRC {total} issues{stale}"),
                }
            }
            _ => "DRC not run".into(),
        }
    }

    /// Marker positions for this board, with the focused finding flagged.
    pub(super) fn markers(&self, board: Option<BoardTarget>) -> Vec<(Point, bool)> {
        self.result
            .as_ref()
            .filter(|ran| Some(ran.board) == board)
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
        let (rect, collapse) = workspace::floating_card_width(ui, ui.id().with("board_drc"), "DRC", area, self.collapsed, true, width, |ui| {
            properties::inspector_body(ui);
            ui.horizontal(|ui| {
                if ui.button("Run again").clicked() {
                    action = Action::Rerun;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
            let Some(ran) = &self.result else {
                ui.label(shell::dim("Not run"));
                return;
            };
            if ran.revision != revision {
                ui.label(shell::dim("The board changed since this check. Run again to refresh."));
            }
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
            self.open = false;
            self.rect = Rect::NOTHING;
        }
        action
    }
}
