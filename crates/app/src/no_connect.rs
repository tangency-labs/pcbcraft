//! Pin-attached original schematic marks and a focused read-only intent report.
use crate::{Dialog, PcbApp, Selection, Tool, ViewInput, Workspace, editor_view::DocumentTarget, schematic_canvas::Camera};
use egui::{Painter, Pos2, Rect, Stroke, Ui, vec2};
use pcbcraft_chrome::{Tokens, controls, shell};
use pcbcraft_engine::{
    Project,
    no_connect::{Appearance, Report, Target},
    schematic::Terminal,
};
use serde_json::json;

/// One report is tied to its initiating view, opening and revision; it cannot navigate stale data.
#[derive(Debug)]
pub(super) struct Inspection {
    source: ViewInput,
    report: Report,
}

/// Pick an actual pin endpoint within a small screen tolerance. Body/field/empty-space hits
/// cannot author a detached flag. The selected symbol wins otherwise coincident endpoints.
pub(super) fn pin_at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, selected: &Selection) -> Option<Terminal> {
    project
        .symbols()
        .iter()
        .flat_map(|symbol| {
            let unselected = !selected.symbols.contains(&symbol.id());
            symbol.terminals().into_iter().zip(symbol.pins()).filter_map(move |(terminal, at)| {
                let distance = camera.screen(at, rect).distance(pointer);
                (distance <= 6.).then_some((unselected, distance, terminal))
            })
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)).then_with(|| a.2.cmp(&b.2)))
        .map(|(_, _, terminal)| terminal)
}

/// Draw the original pin-centered cross independently of symbol body geometry and fields.
fn cross(painter: &Painter, center: Pos2, step: f32, color: egui::Color32) {
    let radius = (step * 0.3).clamp(2., 6.);
    for direction in [-1., 1.] {
        let offset = vec2(radius, direction * radius);
        painter.line_segment([center - offset, center + offset], Stroke::new(1.5, color));
    }
}

/// Paint only authored markers on this projection, resolving every coordinate from current
/// terminal geometry. Connected flags stay visible in error ink instead of hiding the wire.
pub(super) fn paint(ui: &mut Ui, painter: &Painter, project: &Project, camera: &Camera, rect: Rect) {
    for symbol in project.symbols() {
        let Some(part) = project.component(symbol).filter(|part| !part.no_connects.is_empty()) else { continue };
        for (index, (terminal, at)) in symbol.terminals().into_iter().zip(symbol.pins()).enumerate() {
            let Some(expected) = part.no_connects.get(terminal.number()) else { continue };
            let center = camera.screen(at, rect);
            if !rect.contains(center) {
                continue;
            }
            let connected = project.connectivity().net_index_for(terminal).is_some();
            let incompatible = symbol.unit_definition().pins().get(index).is_some_and(|pin| pin.electrical_type != *expected);
            cross(painter, center, camera.unit_step(), if connected || incompatible { Tokens::get().error } else { Tokens::get().schematic_wire });
            ui.interact(
                Rect::from_center_size(center, vec2(12., 12.)),
                ui.scope_id().with(("no_connect", symbol.id(), terminal.number())),
                egui::Sense::hover(),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Label, true, format!("No connect {} pin {}", project.reference(symbol), terminal.number()))
            });
        }
    }
}

/// Preview a valid pin target without placing free marks or changing project state.
pub(super) fn preview(painter: &Painter, project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, selected: &Selection) -> Option<Terminal> {
    let terminal = pin_at(project, camera, rect, pointer, selected)?;
    cross(painter, camera.screen(project.terminal_position(terminal)?, rect), camera.unit_step(), Tokens::get().accent);
    Some(terminal)
}

/// Stable rule IDs are machine contracts; the report presents ordinary labels.
fn rule_label(rule: &str) -> &'static str {
    match rule {
        "no_connect_connected" => "Marked pin is connected",
        "no_connect_missing_target" => "No-connect target is missing",
        "no_connect_incompatible_terminal" => "Terminal type changed",
        _ => "Unconnected pin",
    }
}

impl PcbApp {
    /// Arm repeated pin marking at current view authority; Escape/right-click cancels normally.
    pub(super) fn begin_no_connect(&mut self) {
        if self.views.active.workspace != Workspace::Schematic || self.views.active.input(&self.session).is_none() {
            return;
        }
        self.views.sync(&self.session);
        self.message = None;
        self.views.active.schematic.tool = Some(Tool::NoConnect { revision: self.session.revision() });
    }

    /// Use the same revision-checked engine edit for canvas/context and MCP actions.
    pub(super) fn set_no_connect(&mut self, source: ViewInput, target: Target, marked: bool) {
        if !self.views.view(source.view()).is_some_and(|view| view.accepts(source, &self.session)) {
            return;
        }
        let repeat = matches!(self.views.active.schematic.tool, Some(Tool::NoConnect { .. }));
        self.run(pcbcraft_engine::no_connect::SET, &json!({"target":target,"marked":marked,"expected_revision":source.revision}));
        if repeat && self.message.is_none() {
            self.views.active.schematic.tool = Some(Tool::NoConnect { revision: self.session.revision() });
        }
    }

    /// Capture a focused report without touching authored state or claiming full ERC coverage.
    pub(super) fn inspect_no_connects(&mut self) {
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(project) = self.session.project() else { return };
        self.dialog = Dialog::NoConnects(Box::new(Inspection { source, report: project.check_no_connects(self.session.revision()) }));
    }

    /// Shared modal chrome contains bounded report rows and guards navigation/removal against
    /// stale opening/revision authority. Merely inspecting or navigating is a read-only action.
    pub(super) fn no_connect_dialog(&mut self, ctx: &egui::Context, inspection: Box<Inspection>) {
        let current = self.views.view(inspection.source.view()).is_some_and(|view| view.accepts(inspection.source, &self.session));
        let report = &inspection.report;
        let (mut navigate, mut remove, mut close) = (None::<Appearance>, None::<Target>, false);
        let closing = shell::dialog_width(ctx, "no_connect_check", "No-connect check", 540., |ui| {
            ui.label(format!("No-connect markers: {} · Findings: {}", report.total_markers, report.total_findings));
            if !current {
                ui.label(shell::error("The project changed. Reopen the check to inspect current pins."));
            }
            if report.truncated {
                ui.label(format!("Showing {} marker records and {} findings.", report.markers.len(), report.findings.len()));
            }
            let valid: Vec<_> = report.markers.iter().filter(|marker| marker.issues.is_empty()).collect();
            let count = report.findings.len() + valid.len();
            let row_height = Tokens::get().control_height * 2. + Tokens::get().space_8;
            egui::ScrollArea::vertical().id_salt("no_connect_findings").max_height(420.).show_rows(ui, row_height, count, |ui, range| {
                for index in range {
                    let (target, positions, issue) = if let Some(finding) = report.findings.get(index) {
                        (&finding.target, &finding.appearances, Some(finding.rule))
                    } else {
                        let marker = valid[index - report.findings.len()];
                        (&marker.target, &marker.appearances, None)
                    };
                    let name = self
                        .session
                        .project()
                        .and_then(|p| p.components().find(|part| part.id() == target.part))
                        .map_or(target.part.as_str(), |part| part.reference());
                    ui.add(
                        egui::Label::new(format!("{name} pin {} · {}", target.terminal, issue.map_or("Intentionally unconnected", rule_label)))
                            .truncate(),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled_ui(current && !positions.is_empty(), |ui| {
                                controls::button(ui, format!("Show {name} pin {}", target.terminal))
                            })
                            .inner
                            .clicked()
                        {
                            navigate = positions.first().cloned();
                        }
                        if (issue != Some("unconnected_pin"))
                            && ui.add_enabled_ui(current, |ui| controls::button(ui, "Remove marker")).inner.clicked()
                        {
                            remove = Some(target.clone());
                        }
                    });
                }
            });
            close = controls::button(ui, "Close").clicked();
        });
        if current && let Some(target) = remove {
            self.set_no_connect(inspection.source, target, false);
            return;
        }
        if current && let Some(at) = navigate {
            if let Some(id) = self.views.open_document(inspection.source.view(), DocumentTarget::Sheet(at.sheet), &self.session)
                && let Some(view) = self.views.view_mut(id)
            {
                view.schematic.selection = Selection::one_symbol(at.symbol);
                view.camera.center_on(at.at);
            }
            return;
        }
        if !close && !closing {
            self.dialog = Dialog::NoConnects(inspection);
        }
    }
}
