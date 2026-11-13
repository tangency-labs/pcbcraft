//! Pin- and wire-attached original schematic marks and a focused read-only intent report.
use crate::{Dialog, PcbApp, Selection, Tool, ViewInput, Workspace, editor_view::DocumentTarget, schematic_canvas::Camera};
use egui::{Painter, Pos2, Rect, Stroke, StrokeKind, Ui, vec2};
use pcbcraft_chrome::{Tokens, controls, shell};
use pcbcraft_engine::{
    Project,
    no_connect::{Appearance, Attachment, Report, WireAppearance},
    schematic::{Point, Shape, Terminal},
};
use serde_json::json;

/// One report is tied to its initiating view, opening and revision; it cannot navigate stale data.
#[derive(Debug)]
pub(super) struct Inspection {
    source: ViewInput,
    report: Report,
}

/// Report navigation carries a sheet-local selection, independently of which marker kind owns it.
#[derive(Clone)]
struct Navigation {
    sheet: pcbcraft_engine::SheetTarget,
    selection: Selection,
    at: Point,
}

impl From<&Appearance> for Navigation {
    fn from(at: &Appearance) -> Self {
        Self { sheet: at.sheet, selection: Selection::one_symbol(at.symbol.clone()), at: at.at }
    }
}

impl From<&WireAppearance> for Navigation {
    fn from(at: &WireAppearance) -> Self {
        Self { sheet: at.sheet, selection: Selection::one_wire(at.wire.clone()), at: at.at }
    }
}

/// One bounded display row uses the same stable target for navigation and removal.
struct Row {
    target: Attachment,
    label: String,
    positions: Vec<Navigation>,
    issue: Option<&'static str>,
}

/// A geometric hit carries a stable authored address and the exact snapped placement point.
pub(super) struct Pick {
    pub(super) target: Attachment,
    pub(super) at: Point,
}

/// Find a pin by its endpoint or actual drawn lead. Nearby endpoints win over lead hits;
/// selection breaks coincident-symbol ties without treating a body or label as a terminal.
pub(super) fn pin_at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, selected: &Selection) -> Option<Terminal> {
    project
        .symbols()
        .iter()
        .flat_map(|symbol| {
            let unselected = !selected.symbols.contains(&symbol.id());
            symbol.terminals().into_iter().zip(symbol.pins()).enumerate().filter_map(move |(index, (terminal, at))| {
                let distance = camera.screen(at, rect).distance(pointer);
                let pin = &symbol.unit_definition().pins()[index];
                let screen = |point: Point| {
                    let (x, y) = pcbcraft_engine::schematic::place(point.x_nm, point.y_nm, symbol.turns(), symbol.is_mirrored());
                    camera.screen(Point { x_nm: symbol.position.x_nm + x, y_nm: symbol.position.y_nm + y }, rect)
                };
                let lead_hit = |a: Point, b: Point| {
                    if a != pin.offset && b != pin.offset {
                        return false;
                    }
                    let a = screen(a);
                    let b = screen(b);
                    let delta = b - a;
                    let t = ((pointer - a).dot(delta) / delta.length_sq().max(0.001)).clamp(0., 1.);
                    (a + delta * t).distance(pointer) <= 7.
                };
                let lead = pin.appearance.as_ref().is_some_and(|appearance| lead_hit(pin.offset, appearance.end(pin.offset)))
                    || symbol.unit_definition().drawing.iter().any(|shape| match shape {
                        Shape::Line(a, b) => lead_hit(*a, *b),
                        Shape::Polyline(points) => points.windows(2).any(|p| lead_hit(p[0], p[1])),
                        Shape::Graphic(graphic) => graphic.paths().iter().any(|path| path.windows(2).any(|p| lead_hit(p[0], p[1]))),
                        _ => false,
                    });
                (distance <= 12. || lead).then_some((distance > 12., unselected, distance, terminal))
            })
        })
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.total_cmp(&b.2)).then_with(|| a.3.cmp(&b.3)))
        .map(|(_, _, _, terminal)| terminal)
}

/// Snap to a pin lead or a dangling wire end, preferring the closest endpoint when both are
/// nearby. The preview and click share this result; wire interiors cannot author a free flag.
pub(super) fn target_at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, selected: &Selection) -> Option<Pick> {
    let pin = pin_at(project, camera, rect, pointer, selected)
        .and_then(|pin| Some(Pick { target: Attachment::Pin(project.no_connect_target(pin)?), at: project.terminal_position(pin)? }));
    let wire = project
        .wire_end_targets()
        .chain(project.wire_end_marks().map(|(target, at, _)| (target, at)))
        .filter_map(|(target, at)| {
            let distance = camera.screen(at, rect).distance(pointer);
            (distance <= 12.).then_some((distance, Pick { target: Attachment::WireEnd(target), at }))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    match (pin, wire) {
        (Some(pin), Some((distance, wire))) if distance < camera.screen(pin.at, rect).distance(pointer) => Some(wire),
        (Some(pin), _) => Some(pin),
        (_, Some((_, wire))) => Some(wire),
        _ => None,
    }
}

/// Draw the original attachment-centered cross independently of symbol body geometry and fields.
fn cross(painter: &Painter, center: Pos2, step: f32, color: egui::Color32) {
    let radius = radius(step);
    for direction in [-1., 1.] {
        let offset = vec2(radius, direction * radius);
        painter.line_segment([center - offset, center + offset], Stroke::new(2., color));
    }
}

/// The same extent drives painting, picking and window selection at every zoom.
pub(super) fn radius(step: f32) -> f32 {
    (step * 0.5).clamp(5., 12.)
}

/// Select only an authored X, leaving empty pin tips and unmarked ends to the wire tool.
pub(super) fn marker_at(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<Attachment> {
    project
        .no_connect_appearances()
        .filter_map(|(target, at)| {
            let delta = pointer - camera.screen(at, rect);
            let extent = radius(camera.unit_step());
            let on_stroke = delta.x.abs().max(delta.y.abs()) <= extent + 3. && (delta.x.abs() - delta.y.abs()).abs() <= 4.;
            on_stroke.then_some((delta.length_sq(), target))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        .map(|(_, target)| target)
}

/// Outline selected X geometry without selecting or recoloring its electrical owner.
fn selection_outline(painter: &Painter, center: Pos2, step: f32, ink: egui::Color32) {
    painter.rect_stroke(
        Rect::from_center_size(center, vec2(2. * radius(step) + 6., 2. * radius(step) + 6.)),
        0,
        Stroke::new(1.5, ink),
        StrokeKind::Outside,
    );
}

/// Paint only authored markers on this projection, resolving coordinates from current
/// terminal and endpoint geometry. Connected flags stay visible in error ink instead of hiding the wire.
/// The connectivity projection retains logical aliases wired on another sheet, so repeated
/// appearances of the same pin share their conflict color.
pub(super) fn paint(ui: &mut Ui, painter: &Painter, project: &Project, camera: &Camera, rect: Rect, selected: &Selection, highlighted: &Selection) {
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
            cross(
                painter,
                center,
                camera.unit_step(),
                if connected || incompatible { Tokens::get().error } else { Tokens::get().schematic_no_connect },
            );
            let target = Attachment::Pin(pcbcraft_engine::no_connect::Target { part: symbol.component_id(), terminal: terminal.number().into() });
            if selected.no_connects.contains(&target) {
                selection_outline(painter, center, camera.unit_step(), Tokens::get().accent);
            } else if highlighted.no_connects.contains(&target) {
                selection_outline(painter, center, camera.unit_step(), Tokens::get().schematic_highlight);
            }
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
    for (target, at, conflict) in project.wire_end_marks() {
        let center = camera.screen(at, rect);
        if !rect.contains(center) {
            continue;
        }
        cross(painter, center, camera.unit_step(), if conflict { Tokens::get().error } else { Tokens::get().schematic_no_connect });
        if selected.no_connects.contains(&Attachment::WireEnd(target.clone())) {
            selection_outline(painter, center, camera.unit_step(), Tokens::get().accent);
        } else if highlighted.no_connects.contains(&Attachment::WireEnd(target.clone())) {
            selection_outline(painter, center, camera.unit_step(), Tokens::get().schematic_highlight);
        }
        ui.interact(Rect::from_center_size(center, vec2(20., 20.)), ui.scope_id().with(("no_connect_end", &target.junction)), egui::Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, "No connect wire end"));
    }
}

/// Replace the system arrow with the no-connect cursor at the pointer or its exact attachment target.
/// The cross center is the click point; moving the cursor never authors a detached marker.
pub(super) fn preview(painter: &Painter, project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, selected: &Selection) -> Option<Attachment> {
    let target = target_at(project, camera, rect, pointer, selected);
    let center = target.as_ref().map_or(pointer, |pick| camera.screen(pick.at, rect));
    painter.ctx().set_cursor_icon(egui::CursorIcon::None);
    pcbcraft_chrome::cursor::no_connect(painter, center);
    target.map(|pick| pick.target)
}

/// Stable rule IDs are machine contracts; the report presents ordinary labels.
fn rule_label(rule: &str) -> &'static str {
    match rule {
        "no_connect_connected" => "Marked pin is connected",
        "no_connect_missing_target" => "No-connect target is missing",
        "no_connect_incompatible_terminal" => "Terminal type changed",
        "dangling_wire" => "Unterminated wire end",
        "no_connect_not_dangling" => "Marked wire end is connected",
        _ => "Unconnected pin",
    }
}

impl PcbApp {
    /// Merge pin and wire-end report records without hiding either kind's addressed findings.
    fn no_connect_rows(&self, report: &Report) -> Vec<Row> {
        let pin_row = |target: &pcbcraft_engine::no_connect::Target, appearances: &[Appearance], issue| {
            let name = self
                .session
                .project()
                .and_then(|p| p.components().find(|part| part.id() == target.part))
                .map_or(target.part.as_str(), |part| part.reference());
            Row {
                target: Attachment::Pin(target.clone()),
                label: format!("{name} pin {}", target.terminal),
                positions: appearances.iter().map(Navigation::from).collect(),
                issue,
            }
        };
        let mut rows = report.findings.iter().map(|finding| pin_row(&finding.target, &finding.appearances, Some(finding.rule))).collect::<Vec<_>>();
        rows.extend(report.markers.iter().filter(|marker| marker.issues.is_empty()).map(|marker| pin_row(&marker.target, &marker.appearances, None)));
        rows.extend(report.wire_findings.iter().map(|finding| Row {
            target: Attachment::WireEnd(finding.target.clone()),
            label: "wire end".into(),
            positions: vec![Navigation::from(&finding.appearance)],
            issue: Some(finding.rule),
        }));
        rows.extend(report.wire_markers.iter().filter(|marker| marker.issues.is_empty()).map(|marker| Row {
            target: Attachment::WireEnd(marker.target.clone()),
            label: "wire end".into(),
            positions: vec![Navigation::from(&marker.appearance)],
            issue: None,
        }));
        rows
    }

    /// Arm repeated marking at current view authority, releasing prior selection and highlights.
    /// Escape/right-click cancels normally without changing authored state.
    pub(super) fn begin_no_connect(&mut self) {
        if self.views.active.workspace != Workspace::Schematic || self.views.active.input(&self.session).is_none() {
            return;
        }
        self.views.sync(&self.session);
        self.select(Selection::default());
        self.message = None;
        self.views.active.schematic.tool = Some(Tool::NoConnect { revision: self.session.revision() });
    }

    /// Use the shared revision-checked edit. Successful placement releases prior edit selection
    /// and its counterpart highlights; removal leaves other valid selected markers alone.
    pub(super) fn set_no_connect(&mut self, source: ViewInput, target: Attachment, marked: bool) {
        if !self.views.view(source.view()).is_some_and(|view| view.accepts(source, &self.session)) {
            return;
        }
        let repeat = matches!(self.views.active.schematic.tool, Some(Tool::NoConnect { .. }));
        self.run(pcbcraft_engine::no_connect::SET, &json!({"target":target,"marked":marked,"expected_revision":source.revision}));
        if marked && self.message.is_none() {
            self.select(Selection::default());
        }
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
    /// stale opening/revision authority. Successful navigation focuses the target host;
    /// inspecting or navigating never edits the document.
    pub(super) fn no_connect_dialog(&mut self, ctx: &egui::Context, inspection: Box<Inspection>) {
        let current = self.views.view(inspection.source.view()).is_some_and(|view| view.accepts(inspection.source, &self.session));
        let report = &inspection.report;
        let rows = self.no_connect_rows(report);
        let (mut navigate, mut remove, mut close) = (None::<Navigation>, None::<Attachment>, false);
        let closing = shell::dialog_width(ctx, "no_connect_check", "No-connect check", 540., |ui| {
            ui.label(format!("No-connect markers: {} · Findings: {}", report.total_markers, report.total_findings));
            if !current {
                ui.label(shell::error("The project changed. Reopen the check to inspect current targets."));
            }
            if report.truncated {
                ui.label(format!(
                    "Showing {} marker records and {} findings.",
                    report.markers.len() + report.wire_markers.len(),
                    report.findings.len() + report.wire_findings.len()
                ));
            }
            let count = rows.len();
            let row_height = Tokens::get().control_height * 2. + Tokens::get().space_8;
            egui::ScrollArea::vertical().id_salt("no_connect_findings").max_height(420.).show_rows(ui, row_height, count, |ui, range| {
                for index in range {
                    let Row { target, label, positions, issue } = &rows[index];
                    ui.add(egui::Label::new(format!("{label} · {}", issue.map_or("Intentionally unconnected", rule_label))).truncate());
                    ui.horizontal(|ui| {
                        if ui.add_enabled_ui(current && !positions.is_empty(), |ui| controls::button(ui, format!("Show {label}"))).inner.clicked() {
                            navigate = positions.first().cloned();
                        }
                        if !matches!(issue, Some("unconnected_pin" | "dangling_wire"))
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
                view.schematic.selection = at.selection;
                view.camera.center_on(at.at);
                if let Some(host) = self.views.owner(id) {
                    ctx.send_viewport_cmd_to(crate::editor_windows::viewport(host), egui::ViewportCommand::Focus);
                }
            }
            return;
        }
        if !close && !closing {
            self.dialog = Dialog::NoConnects(inspection);
        }
    }
}
