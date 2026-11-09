//! Explicit ordinary-text conversion choices precede any native preview or project transaction.
use crate::{Dialog, PcbApp, ViewInput, Workspace};
use egui::{Context, Ui};
use pcbcraft_chrome::{properties, shell, units};
use pcbcraft_engine::{
    clipboard_text::{self as engine, CoordinateUnits},
    schematic::GRID_NM,
};
use serde_json::json;

/// A bounded editable conversion draft retains its originating destination and electrical scope.
#[derive(Debug)]
pub(super) struct Options {
    source: ViewInput,
    workspace: Workspace,
    text: String,
    global: bool,
    suffix: String,
    spacing: String,
    units: String,
    closed: bool,
    invert_y: bool,
    original: bool,
    error: Option<String>,
}
impl PcbApp {
    /// Offer conversion for ordinary text only; native JSON remains on the strict native parser.
    pub(super) fn begin_external_clipboard(&mut self, text: String) {
        if text.len() > engine::MAX_TEXT_BYTES {
            self.message = Some("External clipboard text exceeds 64 KiB.".into());
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else { return };
        let units = if text.trim_start_matches('\u{feff}').trim_start().lines().next().is_some_and(|s| s.to_ascii_lowercase().contains("x_mil")) {
            "mil"
        } else {
            "mm"
        };
        let workspace = self.views.active.workspace;
        self.views.active.cancel_gesture();
        self.dialog = Dialog::ExternalPasteOptions(Box::new(Options {
            source,
            workspace,
            text,
            global: false,
            suffix: String::new(),
            spacing: units::edit_label(2 * GRID_NM, self.preferences.units),
            units: units.into(),
            closed: false,
            invert_y: false,
            original: false,
            error: None,
        }));
        self.message = None;
    }
    /// Conversion choices stay in the same host as their captured schematic/physical destination.
    pub(super) fn external_paste_options_host(&self) -> Option<crate::editor_layout::HostId> {
        match &self.dialog {
            Dialog::ExternalPasteOptions(d) => self.views.owner(d.source.view()),
            _ => None,
        }
    }
    /// Display ordinary text with an explicit interpretation. Preparation is read-only; only
    /// successful native placement can publish one undoable edit, and stale forms retain errors.
    pub(super) fn external_paste_options_dialog(&mut self, ctx: &Context, mut draft: Box<Options>) {
        let mut apply = false;
        let mut cancel = false;
        let dismissed = shell::scrolling_dialog_width(ctx, "external_paste_options", "Paste Text", 520., |ui| {
            match draft.workspace {
                Workspace::Schematic => {
                    ui.label(shell::dim(
                        "Each line becomes an electrical net label. Matching names connect within their scope; these are not drawing notes.",
                    ));
                    let mut scope = if draft.global { "global".into() } else { "local".into() };
                    properties::choice(ui, "Scope", ui.id().with("scope"), &mut scope, &[("local", "Local".into()), ("global", "Global".into())]);
                    draft.global = scope == "global";
                    properties::text(ui, "Spacing", &mut draft.spacing);
                    properties::text(ui, "Net-name suffix", &mut draft.suffix);
                }
                Workspace::Board => {
                    ui.label(shell::dim("Paste a two-column X/Y coordinate table as a Board Outline polyline. CSV and tab-separated numbers are supported; source units are independent of the display units."));
                    properties::choice(ui, "Source units", ui.id().with("units"), &mut draft.units, &[("mm", "mm".into()), ("mil", "mil".into())]);
                    ui.checkbox(&mut draft.closed, "Close outline");
                    ui.checkbox(&mut draft.invert_y, "Invert Y axis");
                    ui.checkbox(&mut draft.original, "Keep source coordinates");
                    outline_size(ui, &draft);
                }
            }
            let label = ui.label("Clipboard text");
            ui.add(
                egui::TextEdit::multiline(&mut draft.text)
                    .desired_rows(5)
                    .desired_width(f32::INFINITY)
                    .char_limit(engine::MAX_TEXT_BYTES)
                    .code_editor(),
            )
            .labelled_by(label.id);
            if let Some(error) = &draft.error {
                ui.label(shell::error(error));
            }
            ui.horizontal(|ui| {
                apply = ui.button(if draft.workspace == Workspace::Schematic { "Paste labels" } else { "Paste outline" }).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if apply {
            if !self.views.active.accepts(draft.source, &self.session) {
                draft.error = Some("The destination changed. Close this form and paste again.".into());
                self.dialog = Dialog::ExternalPasteOptions(draft);
                return;
            }
            let params = match draft.workspace {
                Workspace::Schematic => units::parse_length(&draft.spacing,self.preferences.units).map(|spacing| (engine::LABELS,json!({"text":draft.text,"style":if draft.global {"global"}else{"local"},"spacing_nm":spacing,"net_name_suffix":draft.suffix,"expected_revision":draft.source.revision}))).map_err(|e| e.to_string()),
                Workspace::Board => Ok((engine::OUTLINE,json!({"text":draft.text,"units":draft.units,"closed":draft.closed,"invert_y":draft.invert_y,"expected_revision":draft.source.revision}))),
            };
            let prepared = params
                .and_then(|(id, params)| self.session.execute(id, &params).map_err(|e| e.to_string()))
                .and_then(|v| v["clipboard"].as_str().map(str::to_owned).ok_or_else(|| "No prepared fragment was returned.".into()));
            match prepared {
                Ok(text) => match draft.workspace {
                    Workspace::Schematic => self.receive_clipboard(text, false),
                    Workspace::Board => self.receive_prepared_board_clipboard(text, draft.original),
                },
                Err(error) => {
                    draft.error = Some(error);
                    self.dialog = Dialog::ExternalPasteOptions(draft);
                }
            }
        } else if !(cancel || dismissed) {
            self.dialog = Dialog::ExternalPasteOptions(draft);
        }
    }
}
/// Show dimensions from the same bounded parser used by the command, so unit choices can be
/// checked before placement. Incomplete text stays editable and is reported only on Apply.
fn outline_size(ui: &mut Ui, draft: &Options) {
    let units = if draft.units == "mil" { CoordinateUnits::Mil } else { CoordinateUnits::Mm };
    let Ok(points) = engine::coordinate_points(&draft.text, units, draft.closed, draft.invert_y) else { return };
    let (min_x, max_x) = points.iter().map(|p| p.x_nm).fold((i64::MAX, i64::MIN), |(a, b), v| (a.min(v), b.max(v)));
    let (min_y, max_y) = points.iter().map(|p| p.y_nm).fold((i64::MAX, i64::MIN), |(a, b), v| (a.min(v), b.max(v)));
    let unit = if units == CoordinateUnits::Mil { units::Unit::Mil } else { units::Unit::Mm };
    ui.label(format!("{} vertices · {} × {}", points.len(), units::label(max_x - min_x, unit), units::label(max_y - min_y, unit)));
}

#[cfg(test)]
mod tests;
