//! Symbol Editor dialog for importing, mapping, validating and previewing a CSV/TSV pin table.
//!
//! Confirming a valid preview stages an immutable [`ImportPlan`] on the active symbol document
//! without creating pins or altering document/project history. Multi-unit symbol generation (P2)
//! and re-import diffing (P3) consume that staged plan directly.

use super::*;
use pcbcraft_chrome::{
    Tokens, controls,
    library_workspace::{self, TableEntry},
    shell,
};
use pcbcraft_engine::authoring::pin_table::{self, ColumnMapping, ColumnRole, Delimiter, ImportPlan, MAX_BYTES, PinSide, Preview, PreviewRow};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// Number of preview rows displayed per table page to keep large 256+ pin tables responsive.
const ROWS_PER_PAGE: usize = 25;
/// Maximum number of inline findings listed below the preview table before summarizing overflow.
const MAX_INLINE_FINDINGS: usize = 20;
/// Dialog width in logical points, giving all seven preview columns room without horizontal cramping.
const DIALOG_WIDTH: f32 = 880.0;

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
}

impl Draft {
    /// Start an empty pin-table import draft bound to one open symbol document revision.
    pub(super) fn new(document: u64, revision: u64) -> Self {
        let preview = pin_table::preview("", Delimiter::Auto, true, None);
        let mapping = preview.mapping;
        Self {
            document,
            revision,
            raw_text: String::new(),
            source_name: None,
            delimiter: Delimiter::Auto,
            has_header: true,
            mapping,
            manual_mapping: false,
            preview,
            findings_only: false,
            page: 0,
            file_job: None,
            io_error: None,
        }
    }

    /// Re-run pure engine parsing and validation after any text, delimiter, header or mapping edit.
    fn recompute(&mut self) {
        let override_map = self.manual_mapping.then_some(self.mapping);
        self.preview = pin_table::preview(&self.raw_text, self.delimiter, self.has_header, override_map);
        self.mapping = self.preview.mapping;
        if self.preview.findings.is_empty() {
            self.findings_only = false;
        }
        let filtered_len = self.visible_row_count();
        let pages = filtered_len.div_ceil(ROWS_PER_PAGE).max(1);
        self.page = self.page.min(pages.saturating_sub(1));
    }

    /// Count rows matching the active All / Findings-only filter.
    fn visible_row_count(&self) -> usize {
        if self.findings_only { self.preview.rows.iter().filter(|row| !row.is_valid()).count() } else { self.preview.rows.len() }
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

/// Format a nanometre span as millimetres with one decimal place for unit body size previews.
fn format_mm(nm: i64) -> String {
    format!("{:.1} mm", nm as f64 / 1_000_000.0)
}

/// Describe a column option in the mapping dropdowns using its 1-based column index and header text.
fn column_option_label(headers: &[String], index: usize) -> String {
    match headers.get(index).map(|text| text.trim()).filter(|text| !text.is_empty()) {
        Some(header) => format!("Col {}: {header}", index + 1),
        None => format!("Col {}", index + 1),
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
    controls::Choice::from_label(role.label()).selected_text(selected_text).width(180.0).show_ui(ui, |ui| {
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

/// Build one page of preview table rows with inline status text (never color- or hover-only).
fn preview_table_entries(preview: &Preview, findings_only: bool, page: usize) -> Vec<TableEntry> {
    let filtered = preview.rows.iter().filter(|row| !findings_only || !row.is_valid());
    filtered.skip(page.saturating_mul(ROWS_PER_PAGE)).take(ROWS_PER_PAGE).map(|row| row_to_table_entry(preview, row)).collect()
}

/// Convert one [`PreviewRow`] into a shared chrome [`TableEntry`].
fn row_to_table_entry(preview: &Preview, row: &PreviewRow) -> TableEntry {
    let number = row.number.as_deref().unwrap_or(row.raw_number.as_str());
    let name = row.name.as_deref().unwrap_or("—");
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
            .map(|finding| finding.message.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    };
    TableEntry {
        key: format!("row-{}", row.row_index),
        selected: !row.is_valid(),
        branch: None,
        nested: false,
        cells: vec![row.row_index.to_string(), number.to_owned(), name.to_owned(), etype, row.unit.clone(), side, status],
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
        self.dialog = Dialog::LibraryPinTable(Box::new(Draft::new(document, revision)));
        Ok(())
    }

    /// Stage a validated [`ImportPlan`] on the owning symbol document as the P2/P3 hand-off seam.
    /// Never mutates symbol geometry or project undo history.
    pub(super) fn confirm_library_pin_table(&mut self, document: u64, revision: u64, plan: ImportPlan) -> Result<(), String> {
        let doc = self.library_editors.documents.get_mut(&document).ok_or("Symbol document was closed before import was confirmed.")?;
        if doc.session.snapshot().kind != Kind::Symbol {
            return Err("Pin-table import applies only to symbol definitions.".into());
        }
        if doc.session.revision() != revision {
            return Err("Symbol changed while the pin-table dialog was open; reopen Import pin table to confirm.".into());
        }
        let pin_count = plan.pins.len();
        let unit_count = plan.units.len();
        doc.staged_pin_table = Some(plan);
        self.library_editors.notice = Some(format!(
            "Pin table staged ({pin_count} pin{} across {unit_count} unit{}). Symbol generation is not yet enabled; no pins were created.",
            if pin_count == 1 { "" } else { "s" },
            if unit_count == 1 { "" } else { "s" },
        ));
        Ok(())
    }

    /// Render the modal pin-table import and preview dialog in the Symbol Editor.
    pub(crate) fn library_pin_table_dialog(&mut self, ctx: &Context, mut draft: Box<Draft>) {
        draft.poll_file_job();
        let tokens = Tokens::get();
        let mut load_file = false;
        let mut clear_text = false;
        let mut auto_map = false;
        let mut changed = false;
        let mut confirm = false;
        let mut cancel = false;
        let loading = draft.file_job.is_some();
        let can_confirm = draft.preview.can_accept() && !loading;

        // Allow Cmd/Ctrl+Enter to confirm when the preview is valid, no popup is open and no file job is in flight.
        let shortcut_confirm =
            !egui::Popup::is_any_open(ctx) && can_confirm && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));

        let dismissed = shell::scrolling_dialog_width(ctx, "library_pin_table", "Import pin table", DIALOG_WIDTH, |ui| {
            ui.label(shell::dim("Paste a CSV or TSV pin table or load a UTF-8 file. Preview validates every row before anything is staged."));
            ui.add_space(tokens.space_sm);

            // Section 1: Source table
            library_workspace::heading(ui, "1. Source table");
            ui.horizontal_wrapped(|ui| {
                let load_label = if loading { "Loading file…" } else { "Load file…" };
                if controls::enabled_button(ui, !loading && !IN_BROWSER, load_label).clicked() {
                    load_file = true;
                }
                if controls::enabled_button(ui, !draft.raw_text.is_empty() && !loading, "Clear").clicked() {
                    clear_text = true;
                }
                ui.add_space(tokens.space_sm);
                let detected_suffix = if draft.delimiter == Delimiter::Auto && !draft.raw_text.is_empty() {
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
            ui.add_space(tokens.space_xs);
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

            ui.add_space(tokens.space_md);

            // Section 2: Column mapping
            ui.horizontal(|ui| {
                library_workspace::heading(ui, "2. Column mapping");
                ui.add_space(tokens.space_sm);
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

            ui.add_space(tokens.space_md);

            // Section 3: Live preview
            library_workspace::heading(ui, "3. Live preview");
            let total_rows = draft.preview.rows.len();
            let valid_rows = draft.preview.valid_row_count;
            let finding_count = draft.preview.findings.len();
            let unit_count = draft.preview.units.len();
            ui.label(format!(
                "{total_rows} row{} · {valid_rows} valid · {finding_count} finding{} · {unit_count} unit{}",
                if total_rows == 1 { "" } else { "s" },
                if finding_count == 1 { "" } else { "s" },
                if unit_count == 1 { "" } else { "s" },
            ));
            if !draft.preview.units.is_empty() {
                let unit_line = draft
                    .preview
                    .units
                    .iter()
                    .take(6)
                    .map(|unit| {
                        format!(
                            "{} ({} pin{}, {} × {})",
                            unit.unit,
                            unit.pin_count,
                            if unit.pin_count == 1 { "" } else { "s" },
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

            ui.horizontal_wrapped(|ui| {
                let filter_label = format!("Show findings only ({finding_count})");
                ui.add_enabled_ui(finding_count > 0, |ui| {
                    if controls::checkbox(ui, &mut draft.findings_only, filter_label).changed() {
                        draft.page = 0;
                    }
                });
                ui.add_space(tokens.space_sm);
                if controls::enabled_button(ui, draft.page > 0, "Previous").clicked() {
                    draft.page = draft.page.saturating_sub(1);
                }
                ui.label(shell::dim(format!("Page {} of {} · {} shown", draft.page + 1, pages, visible_count)));
                if controls::enabled_button(ui, draft.page + 1 < pages, "Next").clicked() {
                    draft.page = draft.page.saturating_add(1);
                }
            });

            if total_rows > 0 {
                let table_rows = preview_table_entries(&draft.preview, draft.findings_only, draft.page);
                ui.allocate_ui(egui::vec2(ui.available_width(), 240.0), |ui| {
                    ui.push_id(("pin_table_preview_rows", draft.findings_only, draft.page), |ui| {
                        library_workspace::table(ui, &["Row", "Number", "Name", "Electrical type", "Unit", "Side", "Status"], &table_rows, |_| {});
                    });
                });
            }

            if !draft.preview.findings.is_empty() {
                ui.add_space(tokens.space_xs);
                ui.label(shell::error(format!("Validation findings ({finding_count}) — fix these before confirming:")));
                for finding in draft.preview.findings.iter().take(MAX_INLINE_FINDINGS) {
                    ui.label(shell::error(format!("• {}", finding.message)));
                }
                if draft.preview.findings.len() > MAX_INLINE_FINDINGS {
                    ui.label(shell::dim(format!(
                        "… and {} more finding(s). Filter to findings only or edit the source table to inspect all.",
                        draft.preview.findings.len() - MAX_INLINE_FINDINGS
                    )));
                }
            }

            ui.add_space(tokens.space_md);
            ui.separator();
            ui.horizontal(|ui| {
                let readiness = if loading {
                    "Loading pin-table file…".to_owned()
                } else if can_confirm {
                    format!(
                        "Ready to stage {total_rows} pin{} across {unit_count} unit{}.",
                        if total_rows == 1 { "" } else { "s" },
                        if unit_count == 1 { "" } else { "s" },
                    )
                } else if total_rows == 0 {
                    "Paste or load a pin table to preview.".to_owned()
                } else {
                    format!("Resolve {finding_count} finding{} before confirming.", if finding_count == 1 { "" } else { "s" },)
                };
                ui.label(shell::dim(readiness));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if controls::button(ui, "Cancel").clicked() {
                        cancel = true;
                    }
                    let confirm_response = ui.add_enabled(can_confirm, controls::action("Confirm import", true));
                    if confirm_response.clicked() {
                        confirm = true;
                    }
                });
            });
        });

        if cancel || dismissed {
            return;
        }
        if load_file {
            draft.start_file_load(ctx);
        }
        if clear_text {
            draft.raw_text.clear();
            draft.source_name = None;
            draft.io_error = None;
            draft.manual_mapping = false;
            draft.page = 0;
            draft.recompute();
        } else if auto_map {
            draft.manual_mapping = false;
            draft.recompute();
        } else if changed {
            draft.recompute();
        }

        if (confirm || shortcut_confirm) && draft.preview.can_accept() && draft.file_job.is_none() {
            match draft.preview.into_import_plan().and_then(|plan| self.confirm_library_pin_table(draft.document, draft.revision, plan)) {
                Ok(()) => return,
                Err(error) => draft.io_error = Some(error),
            }
        }

        self.dialog = Dialog::LibraryPinTable(draft);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

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

    /// Opening the dialog on a symbol document, pasting a 256-pin CSV with 2 duplicates,
    /// filtering to findings, fixing the duplicates and confirming stages an `ImportPlan`
    /// without mutating symbol revision or creating pins.
    #[test]
    fn dialog_256_pin_csv_with_two_duplicates_filter_fix_and_confirm() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Symbol);
        let doc_id = app.library_editors.active.unwrap();
        let revision_before = app.library_editors.documents.get(&doc_id).unwrap().session.revision();

        app.begin_library_pin_table(doc_id).unwrap();
        {
            let Dialog::LibraryPinTable(draft) = &mut app.dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = synthetic_256_csv(true);
            draft.recompute();
            assert_eq!(draft.preview.rows.len(), 256);
            assert_eq!(draft.preview.findings.len(), 2);
            assert!(!draft.preview.can_accept());

            draft.findings_only = true;
            assert_eq!(draft.visible_row_count(), 2);
            let entries = preview_table_entries(&draft.preview, true, 0);
            assert_eq!(entries.len(), 2);
            assert!(entries[0].cells[6].contains("duplicate pin number '10'"));
            assert!(entries[1].cells[6].contains("duplicate pin number '20'"));
        }

        // Render one frame with shared chrome tokens to verify layout and table rendering.
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1024.0, 760.0))
            .with_context_setup(pcbcraft_chrome::install)
            .build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        harness.run();
        assert!(matches!(harness.state().dialog, Dialog::LibraryPinTable(_)));

        // Fix the duplicates and confirm.
        {
            let app = harness.state_mut();
            let Dialog::LibraryPinTable(draft) = &mut app.dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = synthetic_256_csv(false);
            draft.recompute();
            assert!(draft.preview.can_accept());
            assert!(!draft.findings_only);
            let plan = draft.preview.into_import_plan().unwrap();
            let revision = draft.revision;
            app.confirm_library_pin_table(doc_id, revision, plan).unwrap();
            app.dialog = Dialog::None;
        }

        let app = harness.state();
        let doc = app.library_editors.documents.get(&doc_id).unwrap();
        assert_eq!(doc.session.revision(), revision_before);
        let staged = doc.staged_pin_table.as_ref().expect("staged ImportPlan");
        assert_eq!(staged.pins.len(), 256);
        assert_eq!(staged.units.len(), 4);
        assert_eq!(staged.pins[0].name.as_deref(), Some("SIG_1/ALT_1"));
        assert!(app.library_editors.notice.as_deref().unwrap_or_default().contains("256 pins across 4 units"));
    }

    /// Manual column remapping updates the preview immediately, Auto-map restores header mapping,
    /// and Escape dismisses the dialog without staging a plan.
    #[test]
    fn dialog_column_remapping_and_escape_cancel() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Symbol);
        let doc_id = app.library_editors.active.unwrap();
        app.begin_library_pin_table(doc_id).unwrap();

        {
            let Dialog::LibraryPinTable(draft) = &mut app.dialog else {
                panic!("expected pin table dialog");
            };
            draft.raw_text = "Signal\tPin\tType\nTXD\t1\toutput\nRXD\t2\tinput\n".into();
            draft.recompute();
            assert_eq!(draft.mapping.number, Some(1));
            assert_eq!(draft.mapping.name, Some(0));
            assert!(draft.preview.can_accept());

            // Simulate user unmapping pin number.
            draft.mapping.number = None;
            draft.manual_mapping = true;
            draft.recompute();
            assert!(!draft.preview.can_accept());

            // Auto-map restores valid mapping.
            draft.manual_mapping = false;
            draft.recompute();
            assert!(draft.preview.can_accept());
            assert_eq!(draft.mapping.number, Some(1));
        }

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1024.0, 720.0))
            .with_context_setup(pcbcraft_chrome::install)
            .build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        harness.run();
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(matches!(harness.state().dialog, Dialog::None));
        assert!(harness.state().library_editors.documents.get(&doc_id).unwrap().staged_pin_table.is_none());
    }

    /// Short-host viewport (L7): scrolling_dialog_width keeps the dialog scrollable without clipping
    /// or panicking even when host height is small (420 px).
    #[test]
    fn dialog_short_host_scroll_keeps_footer_reachable() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Symbol);
        let doc_id = app.library_editors.active.unwrap();
        app.begin_library_pin_table(doc_id).unwrap();
        if let Dialog::LibraryPinTable(draft) = &mut app.dialog {
            draft.raw_text = synthetic_256_csv(false);
            draft.recompute();
        }
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(960.0, 420.0))
            .with_context_setup(pcbcraft_chrome::install)
            .build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        harness.run();
        assert!(matches!(harness.state().dialog, Dialog::LibraryPinTable(_)));
    }
}
