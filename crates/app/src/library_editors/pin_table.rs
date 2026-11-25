//! Symbol Editor dialog for loading, mapping, validating and previewing a CSV/TSV pin table.
//!
//! Staging a valid preview stores an immutable [`ImportPlan`] (with its source text and base
//! revision) on the active symbol document without creating pins or altering document/project
//! history. The plan stays in memory and is not saved; symbol generation in units and re-import
//! difference, the next slices of #78, will consume it.

use super::*;
use pcbcraft_chrome::{
    Tokens, controls,
    library_workspace::{self, TableEntry},
    shell,
};
use pcbcraft_engine::authoring::pin_table::{self, ColumnMapping, ColumnRole, Delimiter, ImportPlan, MAX_BYTES, PinSide, Preview, PreviewRow};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Number of preview rows displayed per table page to keep large 256+ pin tables responsive.
const ROWS_PER_PAGE: usize = 25;
/// Dialog width in logical points, giving all seven preview columns room without horizontal cramping.
const DIALOG_WIDTH: f32 = 880.0;
/// Stable widget ID for the pin-table source text editor so keyboard focus and Escape handling stay exact.
const SOURCE_TEXT_ID: &str = "pin_table_source_text";

/// Inline Discard / Keep editing confirmation shown when the header close button, Escape or a
/// backdrop click would otherwise drop unsaved pin-table edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DiscardPrompt {
    #[default]
    Hidden,
    /// Requested this frame: the next render focuses "Keep editing" and scrolls the prompt into view.
    Opening,
    Shown,
}

/// Validated pin-table plan and source state retained on an open symbol document after staging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StagedPinTable {
    pub(super) plan: ImportPlan,
    pub(super) raw_text: String,
    pub(super) delimiter: Delimiter,
    pub(super) has_header: bool,
    pub(super) mapping: ColumnMapping,
    pub(super) manual_mapping: bool,
}

#[derive(Debug)]
struct LoadedFile {
    name: String,
    text: String,
}

/// Source text and explicit column mapping captured when a file load starts. A finished load is
/// applied only while the draft still matches it, so edits made during a slow load are kept.
#[derive(Debug, PartialEq, Eq)]
struct LoadBase {
    raw_text: String,
    manual_mapping: Option<ColumnMapping>,
}

/// Dialog-owned state for the pin-table preview; never touches symbol history.
#[derive(Debug)]
pub(crate) struct Draft {
    pub(super) document: u64,
    revision: u64,
    raw_text: String,
    source_name: Option<String>,
    delimiter: Delimiter,
    has_header: bool,
    mapping: ColumnMapping,
    manual_mapping: bool,
    preview: Preview,
    findings_only: bool,
    page: usize,
    file_job: Option<Receiver<Result<Option<LoadedFile>, String>>>,
    /// Draft text and mapping when the pending `file_job` started; `None` when no base was taken.
    load_base: Option<LoadBase>,
    io_error: Option<String>,
    confirm_error: Option<String>,
    revision_stale: bool,
    discard_prompt: DiscardPrompt,
    /// Size of the dialog body last frame, to re-centre the modal after the body changes size.
    body_size: egui::Vec2,
}

impl Draft {
    /// Start a pin-table preview draft bound to one open symbol document revision, restoring any
    /// previously staged source text and mapping so staging stays visible and editable.
    pub(super) fn new(document: u64, revision: u64, staged: Option<&StagedPinTable>) -> Self {
        let (raw_text, delimiter, has_header, manual_mapping, mapping_override) = match staged {
            Some(prev) => (prev.raw_text.clone(), prev.delimiter, prev.has_header, prev.manual_mapping, prev.manual_mapping.then_some(prev.mapping)),
            None => (String::new(), Delimiter::Auto, true, false, None),
        };
        let preview = pin_table::preview(&raw_text, delimiter, has_header, mapping_override);
        let mapping = preview.mapping;
        Self {
            document,
            revision,
            raw_text,
            source_name: None,
            delimiter,
            has_header,
            mapping,
            manual_mapping,
            preview,
            findings_only: false,
            page: 0,
            file_job: None,
            load_base: None,
            io_error: None,
            confirm_error: None,
            revision_stale: false,
            discard_prompt: DiscardPrompt::Hidden,
            body_size: egui::Vec2::ZERO,
        }
    }

    /// True when closing now would lose work: non-empty text that was never staged, or any change
    /// to the source text, delimiter, header flag or column mapping of the staged plan.
    fn has_unsaved_edits(&self, staged: Option<&StagedPinTable>) -> bool {
        match staged {
            Some(staged) => {
                staged.raw_text != self.raw_text
                    || staged.delimiter != self.delimiter
                    || staged.has_header != self.has_header
                    || staged.mapping != self.mapping
                    || staged.manual_mapping != self.manual_mapping
            }
            None => !self.raw_text.trim().is_empty(),
        }
    }

    /// Re-run pure engine parsing and validation after any text, delimiter, header or mapping edit.
    fn recompute(&mut self) {
        let override_map = self.manual_mapping.then_some(self.mapping);
        self.preview = pin_table::preview(&self.raw_text, self.delimiter, self.has_header, override_map);
        self.mapping = self.preview.mapping;
        if self.preview.rows_with_findings_count() == 0 {
            self.findings_only = false;
        }
        let filtered_len = self.visible_row_count();
        let pages = filtered_len.div_ceil(ROWS_PER_PAGE).max(1);
        self.page = self.page.min(pages.saturating_sub(1));
        self.confirm_error = None;
        self.revision_stale = false;
        // Any edit answers a pending discard question with "keep editing".
        self.discard_prompt = DiscardPrompt::Hidden;
    }

    /// Count rows matching the active All / Findings-only filter.
    fn visible_row_count(&self) -> usize {
        if self.findings_only { self.preview.rows_with_findings_count() } else { self.preview.rows.len() }
    }

    /// The parts of the draft a finished file load replaces: source text and any manual mapping.
    fn current_load_base(&self) -> LoadBase {
        LoadBase { raw_text: self.raw_text.clone(), manual_mapping: self.manual_mapping.then_some(self.mapping) }
    }

    /// Track a started background file load, remembering the draft it will replace.
    #[cfg(not(target_arch = "wasm32"))]
    fn begin_file_job(&mut self, receiver: Receiver<Result<Option<LoadedFile>, String>>) {
        self.file_job = Some(receiver);
        self.load_base = Some(self.current_load_base());
        self.io_error = None;
        self.discard_prompt = DiscardPrompt::Hidden;
    }

    /// Poll a pending background file picker without blocking the UI thread.
    fn poll_file_job(&mut self) {
        let Some(receiver) = self.file_job.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Pin-table file loader stopped before reporting its result.".into()),
        };
        self.file_job = None;
        let edited_while_loading = self.load_base.take().is_some_and(|base| base != self.current_load_base());
        match result {
            Ok(Some(loaded)) if edited_while_loading => {
                self.io_error = Some(format!(
                    "{} was not loaded because the pin table text or column mapping changed while it was loading. Load it again to replace your edits.",
                    loaded.name
                ));
            }
            Ok(Some(loaded)) => {
                self.raw_text = loaded.text;
                self.source_name = Some(loaded.name);
                self.manual_mapping = false;
                self.io_error = None;
                self.page = 0;
                self.recompute();
            }
            Ok(None) => {}
            Err(error) => self.io_error = Some(error),
        }
    }

    /// Launch a background native file picker for `.csv`, `.tsv` or `.txt` files.
    fn start_file_load(&mut self, ctx: &Context) {
        if self.file_job.is_some() {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            self.io_error = Some("File loading is unavailable in the browser; paste CSV or TSV text below.".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (send, receive) = mpsc::sync_channel(1);
            let ctx = ctx.clone();
            match std::thread::Builder::new().name("pin-table-file".into()).spawn(move || {
                let outcome = pick_pin_table_file();
                let _ = send.send(outcome);
                ctx.request_repaint();
            }) {
                Ok(_) => self.begin_file_job(receive),
                Err(error) => self.io_error = Some(format!("Could not start file loader: {error}")),
            }
        }
    }
}

/// Read a bounded regular UTF-8 CSV/TSV file chosen from the native file dialog.
#[cfg(not(target_arch = "wasm32"))]
fn pick_pin_table_file() -> Result<Option<LoadedFile>, String> {
    use std::io::Read;
    let Some(path) = rfd::FileDialog::new().set_title("Load pin table").add_filter("Pin tables (CSV, TSV, TXT)", &["csv", "tsv", "txt"]).pick_file()
    else {
        return Ok(None);
    };
    if !std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?.file_type().is_file() {
        return Err("Choose a regular CSV or TSV file.".into());
    }
    let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular CSV or TSV file.".into());
    }
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err(format!("Selected file exceeds the {} KiB pin-table limit.", MAX_BYTES / 1024));
    }
    let text = String::from_utf8(bytes).map_err(|_| "Pin table file must be valid UTF-8 text.".to_owned())?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("pin-table").to_owned();
    Ok(Some(LoadedFile { name, text }))
}

/// Choose singular or plural noun form for a count.
fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

/// Format a nanometre span as millimetres with one decimal place for unit body size previews.
fn format_mm(nm: i64) -> String {
    format!("{:.1} mm", nm as f64 / 1_000_000.0)
}

/// Describe a column option in the mapping dropdowns using its 1-based column index and header text.
fn column_option_label(headers: &[String], index: usize) -> String {
    match headers.get(index).map(|text| text.trim()).filter(|text| !text.is_empty()) {
        Some(header) => format!("Column {}: {header}", index + 1),
        None => format!("Column {}", index + 1),
    }
}

/// Render one labelled column-mapping dropdown at the given bounded width and return whether the
/// selection changed.
fn column_selector(ui: &mut Ui, role: ColumnRole, preview: &Preview, value: &mut Option<usize>, required: bool, width: f32) -> bool {
    let selected_text = match *value {
        Some(index) => column_option_label(&preview.headers, index),
        None if required => "Unmapped (required)".to_owned(),
        None => "Not mapped".to_owned(),
    };
    let mut changed = false;
    controls::Choice::from_label(role.label()).selected_text(selected_text).width(width).show_ui(ui, |ui| {
        let none_label = if required { "Unmapped (required)" } else { "Not mapped" };
        if ui.selectable_label(value.is_none(), none_label).clicked() && value.is_some() {
            *value = None;
            changed = true;
        }
        for index in 0..preview.column_count {
            let label = column_option_label(&preview.headers, index);
            if ui.selectable_label(*value == Some(index), label).clicked() && *value != Some(index) {
                *value = Some(index);
                changed = true;
            }
        }
    });
    changed
}

/// Collect the rows visible on the current page under the active filter.
fn visible_page_rows(preview: &Preview, findings_only: bool, page: usize) -> Vec<&PreviewRow> {
    preview.rows.iter().filter(|row| !findings_only || !row.is_valid()).skip(page.saturating_mul(ROWS_PER_PAGE)).take(ROWS_PER_PAGE).collect()
}

/// Build one page of preview table entries. Pin name occupies column 0 (the widest flex column)
/// and Status lists each finding's full-phrase label (or "OK") so the cell reads without a tooltip.
fn preview_table_entries(preview: &Preview, page_rows: &[&PreviewRow]) -> Vec<TableEntry> {
    page_rows.iter().map(|row| row_to_table_entry(preview, row)).collect()
}

/// Convert one [`PreviewRow`] into a shared chrome [`TableEntry`] without marking invalid rows
/// as selected (which would announce them as selected to screen readers).
fn row_to_table_entry(preview: &Preview, row: &PreviewRow) -> TableEntry {
    let name = row.name.as_deref().unwrap_or("—");
    let number = row.number.as_deref().unwrap_or(row.raw_number.as_str());
    let etype = match row.electrical_type {
        Some(kind) => pin_table::electrical_type_label(kind).to_owned(),
        None if row.raw_type.is_empty() => "(missing)".to_owned(),
        None => format!("? {}", row.raw_type),
    };
    let side = match row.side {
        Some(PinSide::Left) => PinSide::Left.label().to_owned(),
        Some(PinSide::Right) => PinSide::Right.label().to_owned(),
        Some(PinSide::Top) => PinSide::Top.label().to_owned(),
        Some(PinSide::Bottom) => PinSide::Bottom.label().to_owned(),
        None => format!("? {}", row.raw_side),
    };
    let status = if row.is_valid() {
        "OK".to_owned()
    } else {
        row.finding_indices
            .iter()
            .filter_map(|&idx| preview.findings.get(idx))
            .map(|finding| finding.kind.short_label())
            .collect::<Vec<_>>()
            .join(", ")
    };
    TableEntry {
        key: format!("row-{}", row.row_index),
        selected: false,
        branch: None,
        nested: false,
        cells: vec![name.to_owned(), row.row_index.to_string(), number.to_owned(), etype, row.unit.clone(), side, status],
    }
}

impl PcbApp {
    /// Open the pin-table preview dialog for an open symbol document without changing its revision.
    pub(super) fn begin_library_pin_table(&mut self, document: u64) -> Result<(), String> {
        let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
        if doc.session.snapshot().kind != Kind::Symbol {
            return Err("Pin-table preview applies only to symbol definitions.".into());
        }
        let revision = doc.session.revision();
        let staged = doc.staged_pin_table.as_ref();
        self.dialog = Dialog::LibraryPinTable(Box::new(Draft::new(document, revision, staged)));
        Ok(())
    }

    /// Stage a validated [`ImportPlan`] in memory on the owning symbol document for symbol
    /// generation in units (next slice of #78).
    /// Never mutates symbol geometry or project undo history.
    pub(super) fn stage_library_pin_table(&mut self, draft: &Draft, plan: ImportPlan) -> Result<(), String> {
        let doc = self.library_editors.documents.get_mut(&draft.document).ok_or("Symbol document was closed before staging.")?;
        if doc.session.snapshot().kind != Kind::Symbol {
            return Err("Pin-table preview applies only to symbol definitions.".into());
        }
        if doc.session.revision() != draft.revision {
            return Err("Symbol changed while the pin-table dialog was open; click Refresh revision to stage on the current revision.".into());
        }
        let pin_count = plan.pins.len();
        let unit_count = plan.units.len();
        doc.staged_pin_table = Some(StagedPinTable {
            plan,
            raw_text: draft.raw_text.clone(),
            delimiter: draft.delimiter,
            has_header: draft.has_header,
            mapping: draft.mapping,
            manual_mapping: draft.manual_mapping,
        });
        self.library_editors.notice = Some(format!(
            "Pin table staged ({pin_count} {} across {unit_count} {}). Symbol generation is not yet enabled; no pins were created.",
            plural(pin_count, "pin", "pins"),
            plural(unit_count, "unit", "units"),
        ));
        Ok(())
    }

    /// Clear any staged pin-table plan on the owning symbol document.
    pub(super) fn clear_staged_pin_table(&mut self, document: u64) {
        if let Some(doc) = self.library_editors.documents.get_mut(&document) {
            doc.staged_pin_table = None;
            self.library_editors.notice = Some("Cleared staged pin table.".into());
        }
    }

    /// Render the modal pin-table preview dialog in the Symbol Editor.
    pub(crate) fn library_pin_table_dialog(&mut self, ctx: &Context, mut draft: Box<Draft>) {
        draft.poll_file_job();
        let tokens = Tokens::get();
        let staged = self.library_editors.documents.get(&draft.document).and_then(|doc| doc.staged_pin_table.as_ref());
        let has_staged = staged.is_some();
        let has_unsaved_edits = draft.has_unsaved_edits(staged);
        let source_id = egui::Id::unique(SOURCE_TEXT_ID);
        // egui drops keyboard focus on an unmodified Escape when the pass begins, before this code
        // runs, so `focused()` can no longer tell that the press was meant for the source field.
        // Read the previous frame's focus instead and consume that Escape, so leaving the text field
        // never reaches the shared modal as a close request. An open dropdown keeps its own Escape.
        let popup_open = egui::Popup::is_any_open(ctx);
        let escape_left_source = !popup_open
            && ctx.memory(|mem| mem.had_focus_last_frame(source_id))
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        // While the discard question is open, Escape answers it with "Keep editing".
        let escape_kept_editing = !popup_open
            && !escape_left_source
            && draft.discard_prompt != DiscardPrompt::Hidden
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut load_file = false;
        let mut clear_text = false;
        let mut auto_map = false;
        let mut changed = false;
        let mut stage = false;
        let mut clear_staged = false;
        let mut refresh_revision = false;
        let mut cancel = false;
        let mut discard = false;
        let mut keep_editing = false;
        let loading = draft.file_job.is_some();
        let is_empty = draft.raw_text.trim().is_empty();
        let can_stage = !is_empty && draft.preview.can_accept() && !loading;

        let dismissed = shell::scrolling_dialog_width(ctx, "library_pin_table", "Pin table preview", DIALOG_WIDTH, |ui| {
            ui.label(shell::dim(
                "Paste a CSV or TSV pin table or load a UTF-8 file to check it. Staging keeps the validated plan with this open symbol for symbol generation; it creates no pins and is not written to disk.",
            ));
            ui.add_space(tokens.space_8);

            // Source table
            library_workspace::heading(ui, "Source table");
            ui.horizontal_wrapped(|ui| {
                let load_label = if loading { "Loading file…" } else { "Load file…" };
                if controls::enabled_button(ui, !loading && !IN_BROWSER, load_label).clicked() {
                    load_file = true;
                }
                if controls::enabled_button(ui, !draft.raw_text.is_empty() && !loading, "Clear").clicked() {
                    clear_text = true;
                }
                ui.add_space(tokens.space_8);
                let detected_suffix = if draft.delimiter == Delimiter::Auto && !is_empty {
                    format!("Auto ({})", draft.preview.delimiter.label())
                } else {
                    draft.delimiter.label().to_owned()
                };
                controls::Choice::from_label("Delimiter").selected_text(detected_suffix).width(148.0).show_ui(ui, |ui| {
                    for mode in Delimiter::ALL {
                        if ui.selectable_label(draft.delimiter == mode, mode.label()).clicked() && draft.delimiter != mode {
                            draft.delimiter = mode;
                            changed = true;
                        }
                    }
                });
                if controls::checkbox(ui, &mut draft.has_header, "First row is header").changed() {
                    changed = true;
                }
            });
            if IN_BROWSER {
                ui.label(shell::dim("File loading is unavailable in the browser; paste CSV or TSV text below."));
            } else if let Some(name) = &draft.source_name {
                ui.label(shell::dim(format!("Loaded file: {name}")));
            }
            if let Some(io_error) = &draft.io_error {
                ui.label(shell::error(io_error));
            }
            ui.add_space(tokens.space_4);
            let source_label = ui.label(shell::dim(format!("Pin table text (CSV or TSV, up to {} KiB):", MAX_BYTES / 1024)));
            egui::ScrollArea::vertical().id_salt("pin_table_source_scroll").max_height(132.0).show(ui, |ui| {
                let editor = egui::TextEdit::multiline(&mut draft.raw_text)
                    .id(source_id)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(5)
                    .desired_width(f32::INFINITY)
                    .hint_text("Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n2,SDA/GPIO2,I/O,main,left")
                    .show(ui);
                // egui 0.37 wraps the text field's response in a WidgetAtomResponse; take the owned
                // inner Response, because `labelled_by` consumes it.
                let response = editor.response.response.labelled_by(source_label.id);
                if response.changed() {
                    draft.source_name = None;
                    draft.io_error = None;
                    changed = true;
                }
            });

            ui.add_space(tokens.space_12);

            // Column mapping
            ui.horizontal(|ui| {
                library_workspace::heading(ui, "Column mapping");
                ui.add_space(tokens.space_8);
                if controls::enabled_button(ui, draft.preview.column_count > 0, "Auto-map columns").clicked() {
                    auto_map = true;
                }
            });
            if draft.preview.column_count == 0 {
                ui.label(shell::dim("Paste or load a table to map its columns to pin fields."));
            } else {
                // `Choice::from_label` lays out each label and trigger in a nested `ui.horizontal`,
                // which does not wrap inside `ui.horizontal_wrapped`. Split the five selectors into
                // two explicit rows sized to the dialog width so the modal never widens past the
                // viewport and clips left-aligned controls off the left edge.
                let dropdown_width = ((ui.available_width() - 280.0) / 3.0).clamp(120.0, 168.0);
                ui.horizontal(|ui| {
                    if column_selector(ui, ColumnRole::Number, &draft.preview, &mut draft.mapping.number, true, dropdown_width) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                    if column_selector(ui, ColumnRole::Name, &draft.preview, &mut draft.mapping.name, false, dropdown_width) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                    if column_selector(ui, ColumnRole::ElectricalType, &draft.preview, &mut draft.mapping.electrical_type, false, dropdown_width) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    if column_selector(ui, ColumnRole::Unit, &draft.preview, &mut draft.mapping.unit, false, dropdown_width) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                    if column_selector(ui, ColumnRole::Side, &draft.preview, &mut draft.mapping.side, false, dropdown_width) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                });
            }

            ui.add_space(tokens.space_12);

            // Live preview
            library_workspace::heading(ui, "Live preview");
            let total_rows = draft.preview.rows.len();
            let finding_count = draft.preview.findings.len();
            let unit_count = draft.preview.units.len();
            if is_empty {
                ui.label(shell::dim("Paste or load a pin table to preview."));
            } else {
                let table_findings = draft.preview.table_finding_count();
                let row_issue_count = draft.preview.rows_with_findings_count();
                let ready_rows = if table_findings > 0 { 0 } else { draft.preview.valid_row_count };
                ui.label(format!(
                    "{total_rows} {} parsed · {ready_rows} ready · {finding_count} {} · {unit_count} {}",
                    plural(total_rows, "row", "rows"),
                    plural(finding_count, "finding", "findings"),
                    plural(unit_count, "unit", "units"),
                ));

                if table_findings > 0 {
                    ui.add_space(tokens.space_4);
                    ui.label(shell::error(format!(
                        "Table {} ({table_findings}) — fix before staging:",
                        plural(table_findings, "finding", "findings"),
                    )));
                    for finding in draft.preview.findings.iter().filter(|finding| finding.row.is_none()) {
                        ui.label(shell::error(format!("• {}", finding.message)));
                    }
                }

                if !draft.preview.units.is_empty() {
                    let unit_line = draft
                        .preview
                        .units
                        .iter()
                        .take(6)
                        .map(|unit| {
                            format!(
                                "{} ({} {}, {} × {})",
                                unit.unit,
                                unit.pin_count,
                                plural(unit.pin_count, "pin", "pins"),
                                format_mm(unit.estimated_width_nm),
                                format_mm(unit.estimated_height_nm),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" · ");
                    let suffix = if draft.preview.units.len() > 6 { format!(" · +{} more", draft.preview.units.len() - 6) } else { String::new() };
                    ui.label(shell::dim(format!("Units: {unit_line}{suffix}")));
                }

                let visible_count = draft.visible_row_count();
                let pages = visible_count.div_ceil(ROWS_PER_PAGE).max(1);
                draft.page = draft.page.min(pages.saturating_sub(1));

                if total_rows > 0 {
                    ui.horizontal_wrapped(|ui| {
                        let filter_label = format!("Show findings only ({row_issue_count})");
                        ui.add_enabled_ui(row_issue_count > 0, |ui| {
                            if controls::checkbox(ui, &mut draft.findings_only, filter_label).changed() {
                                draft.page = 0;
                            }
                        });
                        ui.add_space(tokens.space_8);
                        if controls::enabled_button(ui, draft.page > 0, "Previous").clicked() {
                            draft.page = draft.page.saturating_sub(1);
                        }
                        ui.label(shell::dim(format!("Page {} of {} · {} shown", draft.page + 1, pages, visible_count)));
                        if controls::enabled_button(ui, draft.page + 1 < pages, "Next").clicked() {
                            draft.page = draft.page.saturating_add(1);
                        }
                    });

                    let page_rows = visible_page_rows(&draft.preview, draft.findings_only, draft.page);
                    let table_rows = preview_table_entries(&draft.preview, &page_rows);
                    ui.allocate_ui(egui::vec2(ui.available_width(), 240.0), |ui| {
                        ui.push_id(("pin_table_preview_rows", draft.findings_only, draft.page), |ui| {
                            library_workspace::table(
                                ui,
                                &["Pin name", "Row", "Number", "Electrical type", "Unit", "Side", "Status"],
                                &table_rows,
                                |_| {},
                            );
                        });
                    });

                    let page_findings: Vec<_> =
                        page_rows.iter().flat_map(|row| row.finding_indices.iter().filter_map(|&idx| draft.preview.findings.get(idx))).collect();
                    if !page_findings.is_empty() {
                        ui.add_space(tokens.space_4);
                        ui.label(shell::error(format!(
                            "Row {} on this page ({} of {} total):",
                            plural(page_findings.len(), "finding", "findings"),
                            page_findings.len(),
                            finding_count.saturating_sub(table_findings),
                        )));
                        for finding in page_findings {
                            ui.label(shell::error(format!("• {}", finding.message)));
                        }
                    }
                }
            }

            ui.add_space(tokens.space_12);
            ui.separator();

            if let Some(confirm_error) = &draft.confirm_error {
                ui.horizontal_wrapped(|ui| {
                    ui.label(shell::error(confirm_error));
                    if draft.revision_stale && controls::button(ui, "Refresh revision").clicked() {
                        refresh_revision = true;
                    }
                });
                ui.add_space(tokens.space_4);
            }

            if draft.discard_prompt != DiscardPrompt::Hidden {
                // The footer becomes the decision, so the question sits where the actions were.
                let question = if has_staged {
                    "Close without staging? Your pin-table edits will be lost. The previously staged plan stays on this symbol."
                } else {
                    "Close without staging? Your pin-table edits will be lost."
                };
                let prompt = ui.label(question);
                ui.add_space(tokens.space_4);
                ui.horizontal(|ui| {
                    let keep = controls::button(ui, "Keep editing");
                    if controls::enabled_button(ui, !loading, "Discard pin-table edits").clicked() {
                        discard = true;
                    }
                    if draft.discard_prompt == DiscardPrompt::Opening {
                        // Focus the safe answer so Enter or Space keeps the edits.
                        keep.request_focus();
                        prompt.scroll_to_me(Some(egui::Align::Center));
                        draft.discard_prompt = DiscardPrompt::Shown;
                    }
                    if keep.clicked() {
                        keep_editing = true;
                    }
                });
            } else {
                let readiness = if loading {
                    "Loading pin-table file…".to_owned()
                } else if can_stage {
                    format!(
                        "Ready to stage {total_rows} {} across {unit_count} {}.",
                        plural(total_rows, "pin", "pins"),
                        plural(unit_count, "unit", "units"),
                    )
                } else if is_empty {
                    "Nothing to stage yet.".to_owned()
                } else {
                    format!("Resolve {finding_count} {} before staging.", plural(finding_count, "finding", "findings"))
                };
                ui.label(shell::dim(readiness));
                ui.add_space(tokens.space_4);
                ui.horizontal(|ui| {
                    if controls::enabled_button(ui, can_stage, "Stage pin table").clicked() {
                        stage = true;
                    }
                    if has_staged && controls::enabled_button(ui, !loading, "Clear staged plan").clicked() {
                        clear_staged = true;
                    }
                    if controls::enabled_button(ui, !loading, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            }

            // The centred modal is placed from its previous frame's size and repaints by itself
            // only after its first sizing pass. A body that grew or shrank without new input (a
            // recomputed preview, a finished file load) would stay off-centre, with the footer
            // below a short window, until the next event. One more frame settles it.
            let body_size = ui.min_rect().size();
            if (body_size - draft.body_size).abs().max_elem() > 0.5 {
                draft.body_size = body_size;
                ui.ctx().request_repaint();
            }
        });

        if (cancel || discard) && !loading {
            return;
        }
        if keep_editing || escape_kept_editing {
            draft.discard_prompt = DiscardPrompt::Hidden;
            ctx.memory_mut(|mem| mem.request_focus(source_id));
        } else if dismissed && !loading {
            if !has_unsaved_edits {
                return;
            }
            // Header close, Escape or a backdrop click asks before dropping edits. Repeating it
            // re-focuses the open question and scrolls it back into view.
            draft.discard_prompt = DiscardPrompt::Opening;
            ctx.request_repaint();
        }
        if load_file {
            draft.start_file_load(ctx);
        }
        if refresh_revision && let Some(doc) = self.library_editors.documents.get(&draft.document) {
            draft.revision = doc.session.revision();
            draft.confirm_error = None;
            draft.revision_stale = false;
        }
        if clear_staged {
            self.clear_staged_pin_table(draft.document);
            draft.confirm_error = None;
            draft.revision_stale = false;
        }
        if clear_text {
            draft.raw_text.clear();
            draft.source_name = None;
            draft.io_error = None;
            draft.confirm_error = None;
            draft.revision_stale = false;
            draft.manual_mapping = false;
            draft.page = 0;
            draft.recompute();
        } else if auto_map {
            draft.manual_mapping = false;
            draft.recompute();
        } else if changed {
            draft.recompute();
        }

        if stage && can_stage {
            match draft.preview.into_import_plan(draft.revision).and_then(|plan| self.stage_library_pin_table(&draft, plan)) {
                Ok(()) => return,
                Err(error) => {
                    draft.revision_stale =
                        self.library_editors.documents.get(&draft.document).is_some_and(|doc| doc.session.revision() != draft.revision);
                    draft.confirm_error = Some(error);
                    ctx.request_repaint();
                }
            }
        }

        self.dialog = Dialog::LibraryPinTable(draft);
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::tests::click;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    const DISCARD_QUESTION: &str = "Close without staging? Your pin-table edits will be lost.";
    const DISCARD_QUESTION_STAGED: &str =
        "Close without staging? Your pin-table edits will be lost. The previously staged plan stays on this symbol.";

    /// Assert the inline discard question is not shown and the normal footer actions are.
    fn assert_footer_without_discard_question(h: &Harness<'_, PcbApp>) {
        assert!(h.query_by_label(DISCARD_QUESTION).is_none());
        assert!(h.query_by_label(DISCARD_QUESTION_STAGED).is_none());
        assert!(h.query_by_label("Keep editing").is_none());
        assert!(h.query_by_label("Discard pin-table edits").is_none());
        h.get_by_label("Stage pin table");
        h.get_by_label("Cancel");
    }

    /// Assert the inline discard question replaced the footer and its safe answer has keyboard focus.
    fn assert_discard_question(h: &Harness<'_, PcbApp>, question: &str) {
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        h.get_by_label(question);
        assert!(h.get_by_label("Keep editing").is_focused());
        h.get_by_label("Discard pin-table edits");
        assert!(h.query_by_label("Stage pin table").is_none());
        assert!(h.query_by_label("Cancel").is_none());
    }

    /// Everything staging must leave alone: the symbol session's revision, dirty flag, undo/redo
    /// availability and item, the identity of its current snapshot, and the project session's
    /// revision and undo/redo availability. Any recorded edit would advance a revision.
    fn history_fingerprint(h: &Harness<'_, PcbApp>, doc_id: u64) -> (serde_json::Value, std::sync::Arc<Item>, serde_json::Value) {
        let session = &h.state().library_editors.documents.get(&doc_id).unwrap().session;
        (session.inspect(), session.snapshot(), h.state().session.inspect())
    }

    fn assert_history_unchanged(h: &Harness<'_, PcbApp>, doc_id: u64, before: &(serde_json::Value, std::sync::Arc<Item>, serde_json::Value)) {
        let after = history_fingerprint(h, doc_id);
        assert_eq!(after.0, before.0);
        assert!(std::sync::Arc::ptr_eq(&after.1, &before.1));
        assert_eq!(after.2, before.2);
    }

    fn draft_text(h: &Harness<'_, PcbApp>) -> String {
        let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
            panic!("expected pin table dialog");
        };
        draft.raw_text.clone()
    }

    fn synthetic_256_csv(with_duplicates: bool) -> String {
        let mut csv = String::from("Number,Name,Type,Unit,Side\n");
        for i in 1..=256 {
            let number = if with_duplicates {
                match i {
                    100 => 10,
                    200 => 20,
                    _ => i,
                }
            } else {
                i
            };
            let unit = format!("bank-{}", ((i - 1) / 64) + 1);
            let side = if i % 2 == 1 { "left" } else { "right" };
            let etype = match i % 4 {
                0 => "power_in",
                1 => "input",
                2 => "output",
                _ => "I/O",
            };
            csv.push_str(&format!("{number},SIG_{i}/ALT_{i},{etype},{unit},{side}\n"));
        }
        csv
    }

    fn symbol_harness(size: egui::Vec2) -> (Harness<'static, PcbApp>, u64) {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Symbol);
        let doc_id = app.library_editors.active.unwrap();
        app.begin_library_pin_table(doc_id).unwrap();
        let mut h =
            Harness::builder().with_size(size).with_context_setup(pcbcraft_chrome::install).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        (h, doc_id)
    }

    fn click_at(h: &mut Harness<'_, PcbApp>, pos: egui::Pos2) {
        h.hover_at(pos);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run();
        }
    }

    fn type_multiline(h: &mut Harness<'_, PcbApp>, lines: &[&str]) {
        h.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
        h.run();
        for (idx, line) in lines.iter().enumerate() {
            if idx > 0 {
                h.key_press(egui::Key::Enter);
                h.run();
            }
            h.get_by_role(egui::accesskit::Role::MultilineTextInput).type_text(line);
            h.run();
        }
    }

    fn tab_to_source_field(h: &mut Harness<'_, PcbApp>) -> egui::Id {
        let source_id = egui::Id::unique(SOURCE_TEXT_ID);
        for _ in 0..12 {
            if h.ctx.memory(|mem| mem.focused() == Some(source_id)) {
                return source_id;
            }
            h.key_press(egui::Key::Tab);
            h.run();
        }
        panic!("Tab did not reach pin-table source text field; focused = {:?}", h.ctx.memory(|mem| mem.focused()));
    }

    fn assert_no_findings_filter(h: &Harness<'_, PcbApp>) {
        let has_filter = h
            .query_all_by_role(egui::accesskit::Role::CheckBox)
            .any(|node| node.accesskit_node().label().unwrap_or_default().contains("Show findings only"));
        assert!(!has_filter);
    }

    fn synthetic_40_rows_with_30_errors_csv() -> String {
        let mut csv = String::from("Number,Name,Type,Unit,Side\n");
        for i in 1..=40 {
            let etype = if i <= 30 { "unknown_type" } else { "input" };
            let side = if i % 2 == 1 { "left" } else { "right" };
            csv.push_str(&format!("{i},PIN_{i},{etype},main,{side}\n"));
        }
        csv
    }

    /// Empty dialog opens without any red validation error; typing through the real text field
    /// clears stale file metadata and recomputes live; pasting a 256-pin CSV with 2 duplicates
    /// disables staging, lets the user toggle the findings-only filter, and after fixing duplicates
    /// clicking "Stage pin table" stages an `ImportPlan` without mutating symbol revision, exposes
    /// it in `inspect()`, and restores the exact staged text, delimiter and mapping on reopen.
    #[test]
    fn dialog_256_pin_csv_with_two_duplicates_filter_fix_and_stage() {
        let (mut h, doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        let revision_before = h.state().library_editors.documents.get(&doc_id).unwrap().session.revision();

        // Empty state renders the prompt once, shows the calm footer line, and omits findings/pager controls.
        assert_no_findings_filter(&h);
        assert!(h.query_by_label("Previous").is_none());
        h.get_by_label("Paste or load a pin table to preview.");
        h.get_by_label("Nothing to stage yet.");

        // Drive real TextEdit typing to exercise response.changed() clearing source_name and io_error.
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.source_name = Some("stale.csv".into());
            draft.io_error = Some("Previous I/O error".into());
        }
        h.run();
        type_multiline(&mut h, &["Number,Name,Type,Unit,Side", "1,VCC,power_in,main,top", "1,GND,power_in,main,bottom"]);
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert!(draft.source_name.is_none());
            assert!(draft.io_error.is_none());
            assert_eq!(draft.preview.rows.len(), 2);
            assert_eq!(draft.preview.rows_with_findings_count(), 1);
        }
        h.get_by_label("Show findings only (1)");

        // Clear via the real Clear button, then load the 256-pin CSV with 2 duplicates.
        click(&mut h, "Clear");
        assert_no_findings_filter(&h);
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = synthetic_256_csv(true);
            draft.recompute();
        }
        h.run();

        // Clicking Stage pin table while invalid does not close or stage.
        h.get_by_label("Stage pin table").scroll_to_me();
        h.run();
        click(&mut h, "Stage pin table");
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());

        // Toggle findings-only filter through the real checkbox control.
        h.get_by_label("Show findings only (2)").scroll_to_me();
        h.run();
        click(&mut h, "Show findings only (2)");
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert!(draft.findings_only);
            assert_eq!(draft.visible_row_count(), 2);
        }

        // Fix the duplicates and click "Stage pin table".
        let expected_csv = synthetic_256_csv(false);
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = expected_csv.clone();
            draft.recompute();
        }
        h.run();
        h.get_by_label("Stage pin table").scroll_to_me();
        h.run();
        click(&mut h, "Stage pin table");

        assert!(matches!(h.state().dialog, Dialog::None));
        let (expected_mapping, expected_delimiter) = {
            let doc = h.state().library_editors.documents.get(&doc_id).unwrap();
            assert_eq!(doc.session.revision(), revision_before);
            let staged = doc.staged_pin_table.as_ref().expect("staged pin table");
            assert_eq!(staged.plan.base_revision, revision_before);
            assert_eq!(staged.plan.pins.len(), 256);
            assert_eq!(staged.plan.units.len(), 4);
            assert_eq!(staged.plan.pins[0].name.as_deref(), Some("SIG_1/ALT_1"));
            (staged.mapping, staged.delimiter)
        };
        assert!(h.state().library_editors.notice.as_deref().unwrap_or_default().contains("256 pins across 4 units"));
        let inspected = h.state().library_editors.inspect();
        assert_eq!(inspected["documents"][0]["staged_pin_table"]["pin_count"], 256);
        assert_eq!(inspected["documents"][0]["staged_pin_table"]["unit_count"], 4);

        // Reopening restores the exact staged source text, delimiter and mapping, and exposes Clear staged plan.
        h.state_mut().begin_library_pin_table(doc_id).unwrap();
        h.run();
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert_eq!(draft.raw_text, expected_csv);
            assert_eq!(draft.mapping, expected_mapping);
            assert_eq!(draft.delimiter, expected_delimiter);
        }
        h.get_by_label("Clear staged plan").scroll_to_me();
        h.run();
        click(&mut h, "Clear staged plan");
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());
    }

    /// Column remapping via the labelled ComboBox updates the preview live; Escape closes an open
    /// ComboBox popup before dismissing the modal; Escape with unsaved text asks Discard / Keep
    /// editing and a second Escape keeps editing; Cancel discards cleanly; and Escape closes an
    /// empty dialog immediately.
    #[test]
    fn dialog_column_remapping_choice_escape_and_cancel() {
        let (mut h, doc_id) = symbol_harness(egui::vec2(1024.0, 760.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Signal\tPin\tType\nTXD\t1\toutput\nRXD\t2\tinput\n".into();
            draft.recompute();
            assert_eq!(draft.mapping.number, Some(1));
        }
        h.run();

        // Open the labelled Pin number ComboBox, choose "Unmapped (required)", and verify preview blocks staging.
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pin number").click();
        h.run();
        click(&mut h, "Unmapped (required)");
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert_eq!(draft.mapping.number, None);
            assert!(!draft.preview.can_accept());
        }
        h.get_by_label("Table finding (1) — fix before staging:");

        // Click Auto-map columns to restore valid header mapping.
        click(&mut h, "Auto-map columns");
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert_eq!(draft.mapping.number, Some(1));
            assert!(draft.preview.can_accept());
        }

        // Open Pin number ComboBox again: the first Escape closes the popup only and asks nothing.
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pin number").click();
        h.run();
        h.get_by_label("Unmapped (required)");
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        assert!(h.query_by_label("Unmapped (required)").is_none());
        assert_footer_without_discard_question(&h);

        // The second Escape would close the modal, so it asks first; a third Escape keeps editing.
        h.key_press(egui::Key::Escape);
        h.run();
        assert_discard_question(&h, DISCARD_QUESTION);
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        assert_footer_without_discard_question(&h);
        assert_eq!(h.ctx.memory(|mem| mem.focused()), Some(egui::Id::unique(SOURCE_TEXT_ID)));
        assert_eq!(draft_text(&h), "Signal\tPin\tType\nTXD\t1\toutput\nRXD\t2\tinput\n");

        // Explicit Cancel discards cleanly.
        h.get_by_label("Cancel").scroll_to_me();
        h.run();
        click(&mut h, "Cancel");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());
        // Let egui drop focus from the source field that no longer exists.
        h.run_steps(2);
        assert_ne!(h.ctx.memory(|mem| mem.focused()), Some(egui::Id::unique(SOURCE_TEXT_ID)));

        // Reopening an empty dialog allows Escape to close immediately.
        h.state_mut().begin_library_pin_table(doc_id).unwrap();
        h.run();
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
    }

    /// Keyboard navigation and unsaved-edit protection: Tab enters and leaves the source text field
    /// without inserting a tab character; Escape inside the focused text field only leaves the
    /// field (no question, footer intact); a backdrop click then asks Discard / Keep editing with
    /// focus on Keep editing, and Enter keeps editing with focus back in the field; the header
    /// close button asks the same question, and Discard closes without staging.
    #[test]
    fn dialog_keyboard_tab_escape_and_backdrop_protection() {
        let (mut h, doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));

        // Tab reaches the source field and Tab leaves it without inserting '\t'.
        let source_id = tab_to_source_field(&mut h);
        h.key_press(egui::Key::Tab);
        h.run();
        assert_ne!(h.ctx.memory(|mem| mem.focused()), Some(source_id));
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert!(!draft.raw_text.contains('\t'));
        }

        // Type CSV into the source field via real keyboard input, then press Escape while focused.
        type_multiline(&mut h, &["Number,Name,Type,Unit,Side", "1,VCC,power_in,main,top"]);
        assert_eq!(h.ctx.memory(|mem| mem.focused()), Some(source_id));
        assert_footer_without_discard_question(&h);
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        assert_ne!(h.ctx.memory(|mem| mem.focused()), Some(source_id));
        assert_footer_without_discard_question(&h);
        assert!(draft_text(&h).contains("1,VCC,power_in,main,top"));

        // The question is absent before the backdrop click, so its appearance proves the click
        // reached the backdrop and was turned into a question instead of a silent close.
        click_at(&mut h, egui::pos2(5.0, 5.0));
        assert_discard_question(&h, DISCARD_QUESTION);
        assert!(draft_text(&h).contains("1,VCC,power_in,main,top"));

        // Enter on the focused Keep editing restores the footer and returns focus to the field.
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        assert_footer_without_discard_question(&h);
        assert_eq!(h.ctx.memory(|mem| mem.focused()), Some(source_id));
        assert!(draft_text(&h).contains("1,VCC,power_in,main,top"));

        // The header close button asks the same question; Discard closes without staging.
        click(&mut h, "Close Pin table preview");
        assert_discard_question(&h, DISCARD_QUESTION);
        click(&mut h, "Discard pin-table edits");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());
    }

    /// Escape in the focused source field of a reopened, unchanged staged plan leaves the field
    /// without closing the dialog; a second Escape closes the clean dialog and keeps the plan. A
    /// mapping-only change to the staged plan counts as unsaved: Escape asks, naming the kept plan,
    /// "Keep editing" returns to the form, and Cancel leaves the staged plan untouched.
    #[test]
    fn dialog_escape_in_source_field_of_reopened_staged_plan() {
        let (mut h, doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        let staged_text = "Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n2,GND,power_in,main,bottom\n";
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = staged_text.into();
            draft.recompute();
        }
        h.run();
        h.get_by_label("Stage pin table").scroll_to_me();
        h.run();
        click(&mut h, "Stage pin table");
        assert!(matches!(h.state().dialog, Dialog::None));

        h.state_mut().begin_library_pin_table(doc_id).unwrap();
        h.run();
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            let staged = h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.as_ref();
            assert!(!draft.has_unsaved_edits(staged));
        }
        let source_id = tab_to_source_field(&mut h);
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        assert_ne!(h.ctx.memory(|mem| mem.focused()), Some(source_id));
        assert_footer_without_discard_question(&h);
        h.get_by_label("Clear staged plan");

        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_some());

        // Unmapping the optional Pin name column changes the staged plan's mapping only.
        h.state_mut().begin_library_pin_table(doc_id).unwrap();
        h.run();
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pin name").click();
        h.run();
        click(&mut h, "Not mapped");
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert_eq!(draft.raw_text, staged_text);
            assert_eq!(draft.mapping.name, None);
        }
        h.key_press(egui::Key::Escape);
        h.run();
        assert_discard_question(&h, DISCARD_QUESTION_STAGED);
        click(&mut h, "Keep editing");
        assert_footer_without_discard_question(&h);
        h.get_by_label("Cancel").scroll_to_me();
        h.run();
        click(&mut h, "Cancel");
        assert!(matches!(h.state().dialog, Dialog::None));
        let doc = h.state().library_editors.documents.get(&doc_id).unwrap();
        let staged = doc.staged_pin_table.as_ref().expect("staged plan kept");
        assert_eq!(staged.raw_text, staged_text);
        assert!(staged.mapping.name.is_some());
    }

    /// Short-host viewport (L7): scrolling the modal brings footer buttons cleanly inside the
    /// viewport; revision mismatch reports inline beside the footer with a Refresh revision button
    /// that preserves text and mapping; non-symbol documents refuse the pin-table preview.
    #[test]
    fn dialog_short_host_footer_reachable_revision_refresh_and_non_symbol_refusal() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let fp_id = app.library_editors.active.unwrap();
        assert!(app.begin_library_pin_table(fp_id).is_err());

        let viewport_size = egui::vec2(960.0, 420.0);
        let (mut h, doc_id) = symbol_harness(viewport_size);
        let original_text = "Number,Name,Type\n1,VCC,power_in\n".to_owned();
        let original_mapping = {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = original_text.clone();
            draft.recompute();
            draft.mapping
        };
        h.run();

        // Scroll footer controls into view and assert both lie inside the 960x420 viewport.
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport_size);
        h.get_by_label("Cancel").scroll_to_me();
        h.run();
        assert!(viewport.contains_rect(h.get_by_label("Cancel").rect()));
        assert!(viewport.contains_rect(h.get_by_label("Stage pin table").rect()));

        // Advance the underlying symbol revision behind the dialog's back.
        h.state_mut().library_editors.edit(doc_id, 0, Action::AddUnit { unit: "aux".into() }).unwrap();
        let before_refusal = history_fingerprint(&h, doc_id);
        click(&mut h, "Stage pin table");
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        h.get_by_label("Symbol changed while the pin-table dialog was open; click Refresh revision to stage on the current revision.");
        // The refused stage stores nothing and records nothing.
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());
        assert_history_unchanged(&h, doc_id, &before_refusal);

        // Refresh revision clears the error and preserves text and mapping so Stage pin table succeeds.
        h.get_by_label("Refresh revision").scroll_to_me();
        h.run();
        click(&mut h, "Refresh revision");
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert_eq!(draft.raw_text, original_text);
            assert_eq!(draft.mapping, original_mapping);
            assert!(draft.confirm_error.is_none());
        }
        h.get_by_label("Stage pin table").scroll_to_me();
        h.run();
        let before_stage = history_fingerprint(&h, doc_id);
        click(&mut h, "Stage pin table");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.as_ref().unwrap().plan.base_revision, 1);
        // A successful stage keeps the plan in memory only: no symbol or project history entry.
        assert_history_unchanged(&h, doc_id, &before_stage);
    }

    /// The modal is centred from its previous frame's size, so when the body grows without any
    /// input (here, text set and recomputed directly) the dialog must settle within the same
    /// `run`: further frames must not move the footer, and it must lie inside a short window.
    #[test]
    fn dialog_recentres_after_body_grows_without_input() {
        let viewport_size = egui::vec2(960.0, 420.0);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport_size);
        let (mut h, _doc_id) = symbol_harness(viewport_size);
        let empty_cancel = h.get_by_label("Cancel").rect();
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n2,GND,power_in,main,bottom\n".into();
            draft.recompute();
        }
        h.run();
        let settled_header = h.get_by_label("Close Pin table preview").rect();
        assert!(viewport.contains_rect(settled_header));
        h.run_steps(3);
        assert_eq!(h.get_by_label("Close Pin table preview").rect(), settled_header);
        h.get_by_label("Cancel").scroll_to_me();
        h.run();
        let cancel = h.get_by_label("Cancel").rect();
        assert!(viewport.contains_rect(cancel));
        assert!(viewport.contains_rect(h.get_by_label("Stage pin table").rect()));
        let pin_number = h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pin number").rect();
        let side = h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Side").rect();
        assert!(pin_number.min.x >= viewport.min.x && pin_number.max.x <= viewport.max.x);
        assert!(side.min.x >= viewport.min.x && side.max.x <= viewport.max.x);
        assert_ne!(cancel, empty_cancel);
        h.run_steps(3);
        assert_eq!(h.get_by_label("Cancel").rect(), cancel);
    }

    /// A table at both input caps (4,096 data rows padded to just under 256 KiB) still yields one
    /// bounded page: every row is parsed once, the dialog renders only `ROWS_PER_PAGE` of them, and
    /// the table-level pin limit blocks staging. One byte or one row past a cap is refused or
    /// truncated with a stated finding rather than parsed further. Counts only; no timing.
    #[test]
    fn dialog_preview_at_size_and_row_caps_stays_bounded() {
        let header = "Number,Name,Type,Unit,Side,Notes\n";
        let rows = |count: usize, pad: usize| {
            let mut text = String::from(header);
            for i in 1..=count {
                let side = if i % 2 == 1 { "left" } else { "right" };
                text.push_str(&format!("{i},S{i:04},input,bank-{},{side},{}\n", (i - 1) / 128 + 1, "n".repeat(pad)));
            }
            text
        };
        let pad = (MAX_BYTES - rows(pin_table::MAX_ROWS, 0).len()) / pin_table::MAX_ROWS;
        let at_caps = rows(pin_table::MAX_ROWS, pad);
        assert!(at_caps.len() <= MAX_BYTES && at_caps.len() > MAX_BYTES - pin_table::MAX_ROWS, "fixture is {} bytes", at_caps.len());

        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = at_caps;
            draft.recompute();
            assert_eq!(draft.preview.rows.len(), pin_table::MAX_ROWS);
            assert!(draft.preview.findings.iter().any(|finding| finding.kind == pin_table::FindingKind::TooManyPins));
            assert!(!draft.preview.can_accept());
            assert_eq!(visible_page_rows(&draft.preview, false, 0).len(), ROWS_PER_PAGE);
        }
        h.run();
        h.get_by_label(&format!("Page 1 of {} · {} shown", pin_table::MAX_ROWS.div_ceil(ROWS_PER_PAGE), pin_table::MAX_ROWS));
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Stage pin table").accesskit_node().is_disabled());

        let too_large = "x".repeat(MAX_BYTES + 1);
        let refused = pin_table::preview(&too_large, Delimiter::Auto, true, None);
        assert!(refused.rows.is_empty());
        assert!(refused.findings.iter().any(|finding| finding.kind == pin_table::FindingKind::InputTooLarge));

        let too_many = pin_table::preview(&rows(pin_table::MAX_ROWS + 1, 0), Delimiter::Auto, true, None);
        assert_eq!(too_many.rows.len(), pin_table::MAX_ROWS);
        assert!(too_many.findings.iter().any(|finding| finding.kind == pin_table::FindingKind::TooManyPins
            && finding.message.contains(&format!("more than {} rows", pin_table::MAX_ROWS))));
    }

    /// Snapshot state: empty dialog on open, showing only the empty-state prompt with no red findings.
    #[test]
    fn shot_empty_dialog() {
        let (h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        assert_no_findings_filter(&h);
        assert!(h.query_by_label("Previous").is_none());
        h.get_by_label("Paste or load a pin table to preview.");
        h.get_by_label("Nothing to stage yet.");
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Stage pin table").accesskit_node().is_disabled());
        assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").accesskit_node().is_disabled());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: pasted valid multi-unit CSV rows ready to stage.
    #[test]
    fn shot_pasted_valid_rows() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type,Unit,Side\n\
                              1,VCC,power_in,power,top\n\
                              2,GND,power_in,power,bottom\n\
                              3,SDA/GPIO2,I/O,io,left\n\
                              4,SCL/GPIO3,I/O,io,left\n\
                              5,TXD,output,io,right\n\
                              6,RXD,input,io,right\n"
                .into();
            draft.recompute();
            assert!(draft.preview.can_accept());
        }
        h.run();
        assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Stage pin table").accesskit_node().is_disabled());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: pasted CSV containing duplicate pin number, unknown electrical type and unknown side.
    #[test]
    fn shot_rows_with_errors() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type,Unit,Side\n\
                              1,VCC,power_in,main,top\n\
                              1,GND,power_in,main,bottom\n\
                              3,SDA/GPIO2,analog_magic,main,left\n\
                              4,SCL/GPIO3,I/O,main,diagonal\n"
                .into();
            draft.recompute();
            assert_eq!(draft.preview.rows_with_findings_count(), 3);
        }
        h.run();
        h.get_by_label("Show findings only (3)");
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Stage pin table").accesskit_node().is_disabled());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: table-level findings banner when Pin number is unmapped.
    #[test]
    fn shot_table_findings() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Signal,Dir,Bank\nTXD,output,io\nRXD,input,io\n".into();
            draft.recompute();
            assert_eq!(draft.mapping.number, None);
            assert_eq!(draft.preview.table_finding_count(), 1);
        }
        h.run();
        h.get_by_label("Table finding (1) — fix before staging:");
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Stage pin table").accesskit_node().is_disabled());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: Show findings only toggled on with a multi-page filtered table.
    #[test]
    fn shot_findings_only_paged() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = synthetic_40_rows_with_30_errors_csv();
            draft.recompute();
            assert_eq!(draft.preview.rows_with_findings_count(), 30);
        }
        h.run();
        click(&mut h, "Show findings only (30)");
        h.get_by_label("Page 1 of 2 · 30 shown");
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Stage pin table").accesskit_node().is_disabled());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: column mapping dropdown open on a reordered TSV table.
    #[test]
    fn shot_column_mapping() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Signal\tPin\tDir\tBank\tEdge\nTXD\t1\toutput\tio\tright\nRXD\t2\tinput\tio\tleft\n".into();
            draft.recompute();
            assert_eq!(draft.mapping.number, Some(1));
        }
        h.run();
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pin number").click();
        h.run();
        h.get_by_label("Unmapped (required)");
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: reopened dialog with a previously staged plan showing Clear staged plan in the footer.
    #[test]
    fn shot_reopen_staged() {
        let (mut h, doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type,Unit,Side\n1,VCC,power_in,power,top\n2,GND,power_in,power,bottom\n".into();
            draft.recompute();
        }
        h.run();
        h.get_by_label("Stage pin table").scroll_to_me();
        h.run();
        click(&mut h, "Stage pin table");
        assert!(matches!(h.state().dialog, Dialog::None));

        h.state_mut().begin_library_pin_table(doc_id).unwrap();
        h.run();
        h.get_by_label("Clear staged plan").scroll_to_me();
        h.run();
        assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Clear staged plan").accesskit_node().is_disabled());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: Loaded file caption and inline file I/O error line under Source table.
    #[test]
    fn shot_loaded_file_and_io_error() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n".into();
            draft.source_name = Some("mcu-pins.csv".into());
            draft.io_error = Some(format!("Selected file exceeds the {} KiB pin-table limit.", MAX_BYTES / 1024));
            draft.recompute();
        }
        h.run();
        h.get_by_label("Loaded file: mcu-pins.csv");
        h.get_by_label(&format!("Selected file exceeds the {} KiB pin-table limit.", MAX_BYTES / 1024));
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: confirm/stage error rendered inline beside the footer with Refresh revision action.
    #[test]
    fn shot_confirm_error() {
        let (mut h, doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n2,GND,power_in,main,bottom\n".into();
            draft.recompute();
        }
        h.run();
        h.state_mut().library_editors.edit(doc_id, 0, Action::AddUnit { unit: "aux".into() }).unwrap();
        h.get_by_label("Stage pin table").scroll_to_me();
        h.run();
        let before_refusal = history_fingerprint(&h, doc_id);
        click(&mut h, "Stage pin table");
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());
        assert_history_unchanged(&h, doc_id, &before_refusal);
        h.get_by_label("Refresh revision").scroll_to_me();
        h.run();
        h.get_by_label("Symbol changed while the pin-table dialog was open; click Refresh revision to stage on the current revision.");
        assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Refresh revision").accesskit_node().is_disabled());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: native file picker in flight, showing Loading file… and disabling Cancel/Stage.
    #[test]
    fn shot_loading() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        let (_hold_sender, receiver) = mpsc::sync_channel(1);
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.file_job = Some(receiver);
        }
        h.run_steps(2);
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Loading file…").accesskit_node().is_disabled());
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").accesskit_node().is_disabled());
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Stage pin table").accesskit_node().is_disabled());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    type FileJobSender = mpsc::SyncSender<Result<Option<LoadedFile>, String>>;

    const LOADED_CSV: &str = "Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n2,GND,power_in,main,bottom\n";
    const STALE_LOAD_ERROR: &str =
        "mcu-pins.csv was not loaded because the pin table text or column mapping changed while it was loading. Load it again to replace your edits.";

    /// Start a slow file load the way the Load file… button does, with the picker still open.
    fn begin_slow_file_load(h: &mut Harness<'_, PcbApp>) -> FileJobSender {
        let (sender, receiver) = mpsc::sync_channel(1);
        let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
            panic!("expected pin table dialog");
        };
        draft.begin_file_job(receiver);
        sender
    }

    /// Finish the pending file load with `mcu-pins.csv` and let the dialog poll it.
    fn finish_file_load(h: &mut Harness<'_, PcbApp>, sender: &FileJobSender) {
        sender.send(Ok(Some(LoadedFile { name: "mcu-pins.csv".into(), text: LOADED_CSV.into() }))).unwrap();
        h.run();
    }

    fn draft_of<'a>(h: &'a Harness<'_, PcbApp>) -> &'a Draft {
        let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
            panic!("expected pin table dialog");
        };
        draft
    }

    /// Text typed into the source field while a slow file load is pending is kept when the load
    /// finishes: the file is not applied, the user is told why, and Load file… is available again.
    #[test]
    fn slow_file_load_keeps_text_typed_while_loading() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        let sender = begin_slow_file_load(&mut h);
        h.run_steps(2);
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Loading file…").accesskit_node().is_disabled());

        type_multiline(&mut h, &["Pin,Name", "7,RESET"]);
        assert_eq!(draft_text(&h), "Pin,Name\n7,RESET");
        finish_file_load(&mut h, &sender);

        let draft = draft_of(&h);
        assert!(draft.file_job.is_none());
        assert_eq!(draft.raw_text, "Pin,Name\n7,RESET");
        assert_eq!(draft.source_name, None);
        assert_eq!(draft.io_error.as_deref(), Some(STALE_LOAD_ERROR));
        h.get_by_label(STALE_LOAD_ERROR);
        assert!(h.query_by_label("Loaded file: mcu-pins.csv").is_none());
        assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Load file…").accesskit_node().is_disabled());
    }

    /// A manual column mapping chosen while a slow file load is pending survives the load: the
    /// file is not applied and the explicit mapping stays.
    #[test]
    fn slow_file_load_keeps_manual_mapping_chosen_while_loading() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        let text = "Signal\tPin\tType\nTXD\t1\toutput\nRXD\t2\tinput\n";
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = text.into();
            draft.recompute();
        }
        h.run();
        let sender = begin_slow_file_load(&mut h);
        h.run();

        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pin number").click();
        h.run();
        click(&mut h, "Unmapped (required)");
        assert!(draft_of(&h).manual_mapping);
        finish_file_load(&mut h, &sender);

        let draft = draft_of(&h);
        assert!(draft.file_job.is_none());
        assert_eq!(draft.raw_text, text);
        assert!(draft.manual_mapping);
        assert_eq!(draft.mapping.number, None);
        assert_eq!(draft.source_name, None);
        assert_eq!(draft.io_error.as_deref(), Some(STALE_LOAD_ERROR));
        h.get_by_label(STALE_LOAD_ERROR);
    }

    /// A slow load into a draft left unchanged while it was pending still applies the file:
    /// the text is replaced, the earlier manual mapping is reset to auto-mapping, and the
    /// loaded table is ready to stage.
    #[test]
    fn slow_file_load_into_unchanged_draft_replaces_text_and_mapping() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Signal\tPin\tType\nTXD\t1\toutput\n".into();
            draft.recompute();
        }
        h.run();
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pin number").click();
        h.run();
        click(&mut h, "Unmapped (required)");
        assert!(draft_of(&h).manual_mapping);

        let sender = begin_slow_file_load(&mut h);
        h.run_steps(3);
        finish_file_load(&mut h, &sender);

        let draft = draft_of(&h);
        assert!(draft.file_job.is_none());
        assert!(draft.load_base.is_none());
        assert_eq!(draft.raw_text, LOADED_CSV);
        assert_eq!(draft.source_name.as_deref(), Some("mcu-pins.csv"));
        assert_eq!(draft.io_error, None);
        assert!(!draft.manual_mapping);
        assert_eq!(draft.mapping.number, Some(0));
        assert!(draft.preview.can_accept());
        h.get_by_label("Loaded file: mcu-pins.csv");
        assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Stage pin table").accesskit_node().is_disabled());
    }

    /// Snapshot state: short host viewport (960x420) scrolled to the dialog footer.
    #[test]
    fn shot_short_host_footer() {
        let viewport_size = egui::vec2(960.0, 420.0);
        let (mut h, _doc_id) = symbol_harness(viewport_size);
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n2,GND,power_in,main,bottom\n".into();
            draft.recompute();
        }
        h.run();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport_size);
        h.get_by_label("Cancel").scroll_to_me();
        h.run();
        assert!(viewport.contains_rect(h.get_by_label("Cancel").rect()));
        assert!(viewport.contains_rect(h.get_by_label("Stage pin table").rect()));
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }

    /// Snapshot state: inline Discard / Keep editing question after the header close button is
    /// clicked with unsaved text, with keyboard focus on Keep editing.
    #[test]
    fn shot_discard_question() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n2,GND,power_in,main,bottom\n".into();
            draft.recompute();
        }
        h.run();
        assert_footer_without_discard_question(&h);
        click(&mut h, "Close Pin table preview");
        assert_discard_question(&h, DISCARD_QUESTION);
    }

    /// Snapshot state: keyboard focus on the pin-table source text field reached by Tab from the first control.
    #[test]
    fn shot_keyboard_focus_first_field() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        let source_id = tab_to_source_field(&mut h);
        assert_eq!(h.ctx.memory(|mem| mem.focused()), Some(source_id));
        assert!(h.ctx.egui_wants_keyboard_input());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }
}
