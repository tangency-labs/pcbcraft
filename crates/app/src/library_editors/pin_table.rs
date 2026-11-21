//! Symbol Editor dialog for importing, mapping, validating and previewing a CSV/TSV pin table.
//!
//! Staging a valid preview stores an immutable [`ImportPlan`] (with its source text and base
//! revision) on the active symbol document without creating pins or altering document/project
//! history. Multi-unit symbol generation (P2) and re-import diffing (P3) consume that staged plan.

use super::*;
use pcbcraft_chrome::{
    Tokens, controls,
    library_workspace::{self, TableEntry},
    shell,
};
#[cfg(not(target_arch = "wasm32"))]
use pcbcraft_engine::authoring::pin_table::MAX_BYTES;
use pcbcraft_engine::authoring::pin_table::{self, ColumnMapping, ColumnRole, Delimiter, ImportPlan, PinSide, Preview, PreviewRow};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Number of preview rows displayed per table page to keep large 256+ pin tables responsive.
const ROWS_PER_PAGE: usize = 25;
/// Dialog width in logical points, giving all seven preview columns room without horizontal cramping.
const DIALOG_WIDTH: f32 = 880.0;

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

/// Dialog-owned state for pin-table import and preview; never touches symbol history until P2 consumes it.
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
    io_error: Option<String>,
    confirm_error: Option<String>,
    revision_stale: bool,
}

impl Draft {
    /// Start a pin-table import draft bound to one open symbol document revision, restoring any
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
            io_error: None,
            confirm_error: None,
            revision_stale: false,
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
    }

    /// Count rows matching the active All / Findings-only filter.
    fn visible_row_count(&self) -> usize {
        if self.findings_only { self.preview.rows_with_findings_count() } else { self.preview.rows.len() }
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
        match result {
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
                Ok(_) => {
                    self.file_job = Some(receive);
                    self.io_error = None;
                }
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

/// Render one labelled column-mapping dropdown and return whether the selection changed.
fn column_selector(ui: &mut Ui, role: ColumnRole, headers: &[String], column_count: usize, value: &mut Option<usize>, required: bool) -> bool {
    let selected_text = match *value {
        Some(index) => column_option_label(headers, index),
        None if required => "Unmapped (required)".to_owned(),
        None => "Not mapped".to_owned(),
    };
    let mut changed = false;
    controls::Choice::from_label(role.label()).selected_text(selected_text).width(184.0).show_ui(ui, |ui| {
        let none_label = if required { "Unmapped (required)" } else { "Not mapped" };
        if ui.selectable_label(value.is_none(), none_label).clicked() && value.is_some() {
            *value = None;
            changed = true;
        }
        for index in 0..column_count {
            let label = column_option_label(headers, index);
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
/// and Status uses compact short labels so cells stay readable without hover-only truncation.
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
    /// Open the pin-table import dialog for an open symbol document without changing its revision.
    pub(super) fn begin_library_pin_table(&mut self, document: u64) -> Result<(), String> {
        let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
        if doc.session.snapshot().kind != Kind::Symbol {
            return Err("Pin-table import applies only to symbol definitions.".into());
        }
        let revision = doc.session.revision();
        let staged = doc.staged_pin_table.as_ref();
        self.dialog = Dialog::LibraryPinTable(Box::new(Draft::new(document, revision, staged)));
        Ok(())
    }

    /// Stage a validated [`ImportPlan`] on the owning symbol document as the P2/P3 hand-off seam.
    /// Never mutates symbol geometry or project undo history.
    pub(super) fn stage_library_pin_table(&mut self, draft: &Draft, plan: ImportPlan) -> Result<(), String> {
        let doc = self.library_editors.documents.get_mut(&draft.document).ok_or("Symbol document was closed before staging.")?;
        if doc.session.snapshot().kind != Kind::Symbol {
            return Err("Pin-table import applies only to symbol definitions.".into());
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

    /// Render the modal pin-table import and preview dialog in the Symbol Editor.
    pub(crate) fn library_pin_table_dialog(&mut self, ctx: &Context, mut draft: Box<Draft>) {
        draft.poll_file_job();
        let tokens = Tokens::get();
        let has_staged = self.library_editors.documents.get(&draft.document).is_some_and(|doc| doc.staged_pin_table.is_some());
        let mut load_file = false;
        let mut clear_text = false;
        let mut auto_map = false;
        let mut changed = false;
        let mut stage = false;
        let mut clear_staged = false;
        let mut refresh_revision = false;
        let mut cancel = false;
        let loading = draft.file_job.is_some();
        let is_empty = draft.raw_text.trim().is_empty();
        let can_stage = !is_empty && draft.preview.can_accept() && !loading;

        let dismissed = shell::scrolling_dialog_width(ctx, "library_pin_table", "Import pin table", DIALOG_WIDTH, |ui| {
            ui.label(shell::dim(
                "Paste a CSV or TSV pin table or load a UTF-8 file. Staging validates and saves the plan on this symbol without creating pins yet.",
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
            let source_label = ui.label(shell::dim("Pin table text (CSV or TSV, up to 256 KiB):"));
            egui::ScrollArea::vertical().id_salt("pin_table_source_scroll").max_height(132.0).show(ui, |ui| {
                let editor = egui::TextEdit::multiline(&mut draft.raw_text)
                    .id_salt("pin_table_source_text")
                    .code_editor()
                    .desired_rows(5)
                    .desired_width(f32::INFINITY)
                    .hint_text("Number,Name,Type,Unit,Side\n1,VCC,power_in,main,top\n2,SDA/GPIO2,I/O,main,left")
                    .show(ui);
                let response = editor.response.labelled_by(source_label.id);
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
                ui.horizontal_wrapped(|ui| {
                    if column_selector(ui, ColumnRole::Number, &draft.preview.headers, draft.preview.column_count, &mut draft.mapping.number, true) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                    if column_selector(ui, ColumnRole::Name, &draft.preview.headers, draft.preview.column_count, &mut draft.mapping.name, false) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                    if column_selector(
                        ui,
                        ColumnRole::ElectricalType,
                        &draft.preview.headers,
                        draft.preview.column_count,
                        &mut draft.mapping.electrical_type,
                        false,
                    ) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                    if column_selector(ui, ColumnRole::Unit, &draft.preview.headers, draft.preview.column_count, &mut draft.mapping.unit, false) {
                        draft.manual_mapping = true;
                        changed = true;
                    }
                    if column_selector(ui, ColumnRole::Side, &draft.preview.headers, draft.preview.column_count, &mut draft.mapping.side, false) {
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

            let readiness = if loading {
                "Loading pin-table file…".to_owned()
            } else if can_stage {
                format!(
                    "Ready to stage {total_rows} {} across {unit_count} {}.",
                    plural(total_rows, "pin", "pins"),
                    plural(unit_count, "unit", "units"),
                )
            } else if is_empty {
                "Paste or load a pin table to preview.".to_owned()
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
        });

        if (cancel || dismissed) && !loading {
            return;
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
                }
            }
        }

        self.dialog = Dialog::LibraryPinTable(draft);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::tests::click;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

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

    /// Empty dialog opens without any red validation error; pasting a 256-pin CSV with 2
    /// duplicates disables staging, lets the user toggle the findings-only filter, and after
    /// fixing duplicates clicking "Stage pin table" stages an `ImportPlan` without mutating
    /// symbol revision, exposes it in `inspect()`, and restores it on reopen.
    #[test]
    fn dialog_256_pin_csv_with_two_duplicates_filter_fix_and_stage() {
        let (mut h, doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        let revision_before = h.state().library_editors.documents.get(&doc_id).unwrap().session.revision();

        // Empty state suppresses validation findings and pager.
        assert!(h.query_by_label("Show findings only (1)").is_none());
        assert!(h.query_by_label("Previous").is_none());

        // Paste 256-pin CSV with 2 duplicates.
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
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = synthetic_256_csv(false);
            draft.recompute();
        }
        h.run();
        h.get_by_label("Stage pin table").scroll_to_me();
        h.run();
        click(&mut h, "Stage pin table");

        assert!(matches!(h.state().dialog, Dialog::None));
        let doc = h.state().library_editors.documents.get(&doc_id).unwrap();
        assert_eq!(doc.session.revision(), revision_before);
        let staged = doc.staged_pin_table.as_ref().expect("staged pin table");
        assert_eq!(staged.plan.base_revision, revision_before);
        assert_eq!(staged.plan.pins.len(), 256);
        assert_eq!(staged.plan.units.len(), 4);
        assert_eq!(staged.plan.pins[0].name.as_deref(), Some("SIG_1/ALT_1"));
        assert!(h.state().library_editors.notice.as_deref().unwrap_or_default().contains("256 pins across 4 units"));
        let inspected = h.state().library_editors.inspect();
        assert_eq!(inspected["documents"][0]["staged_pin_table"]["pin_count"], 256);
        assert_eq!(inspected["documents"][0]["staged_pin_table"]["unit_count"], 4);

        // Reopening restores the staged source text and exposes Clear staged plan.
        h.state_mut().begin_library_pin_table(doc_id).unwrap();
        h.run();
        h.get_by_label("Clear staged plan").scroll_to_me();
        h.run();
        click(&mut h, "Clear staged plan");
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());
    }

    /// Column remapping via the labelled ComboBox updates the preview live; Escape closes an open
    /// ComboBox popup before dismissing the modal, and Cancel/Escape discards cleanly.
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

        // Click Auto-map columns to restore valid header mapping.
        click(&mut h, "Auto-map columns");
        {
            let Dialog::LibraryPinTable(draft) = &h.state().dialog else {
                panic!("expected pin table dialog");
            };
            assert_eq!(draft.mapping.number, Some(1));
            assert!(draft.preview.can_accept());
        }

        // Open Pin number ComboBox again: first Escape closes popup only, second Escape closes dialog.
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pin number").click();
        h.run();
        h.get_by_label("Unmapped (required)");
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
        assert!(h.query_by_label("Unmapped (required)").is_none());

        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());
    }

    /// Short-host viewport (L7): scrolling the modal brings footer buttons cleanly inside the
    /// viewport; revision mismatch reports inline beside the footer with a Refresh revision button,
    /// and non-symbol documents refuse pin-table import.
    #[test]
    fn dialog_short_host_footer_reachable_revision_refresh_and_non_symbol_refusal() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let fp_id = app.library_editors.active.unwrap();
        assert!(app.begin_library_pin_table(fp_id).is_err());

        let viewport_size = egui::vec2(960.0, 420.0);
        let (mut h, doc_id) = symbol_harness(viewport_size);
        {
            let Dialog::LibraryPinTable(draft) = &mut h.state_mut().dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Number,Name,Type\n1,VCC,power_in\n".into();
            draft.recompute();
        }
        h.run();

        // Scroll footer controls into view and assert both lie inside the 960x420 viewport.
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport_size);
        h.get_by_label("Cancel").scroll_to_me();
        h.run();
        assert!(viewport.contains_rect(h.get_by_label("Cancel").rect()));
        assert!(viewport.contains_rect(h.get_by_label("Stage pin table").rect()));

        // Advance the underlying symbol revision behind the dialog's back.
        h.state_mut().library_editors.edit(doc_id, 0, Action::AddUnit { unit: "aux".into() }).unwrap();
        click(&mut h, "Stage pin table");
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));

        // Refresh revision clears the error and lets Stage pin table succeed without losing the paste.
        h.get_by_label("Refresh revision").scroll_to_me();
        h.run();
        click(&mut h, "Refresh revision");
        h.get_by_label("Stage pin table").scroll_to_me();
        h.run();
        click(&mut h, "Stage pin table");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(h.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.as_ref().unwrap().plan.base_revision, 1);
    }

    /// Snapshot state: empty dialog on open, showing only the empty-state prompt with no red findings.
    #[test]
    fn shot_empty_dialog() {
        let (h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        assert!(h.query_by_label("Show findings only (1)").is_none());
        assert!(h.query_by_label("Previous").is_none());
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
        click(&mut h, "Stage pin table");
        h.get_by_label("Refresh revision").scroll_to_me();
        h.run();
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

    /// Snapshot state: keyboard focus inside the first text field (pin-table source editor).
    #[test]
    fn shot_keyboard_focus_first_field() {
        let (mut h, _doc_id) = symbol_harness(egui::vec2(1024.0, 820.0));
        h.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
        h.run();
        assert!(!h.get_by_role(egui::accesskit::Role::MultilineTextInput).accesskit_node().is_disabled());
        assert!(h.ctx.egui_wants_keyboard_input());
        assert!(matches!(h.state().dialog, Dialog::LibraryPinTable(_)));
    }
}
