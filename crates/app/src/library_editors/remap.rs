//! A captured remap review is shared by the form and live commands, never by project history.
use super::*;
#[cfg(not(target_arch = "wasm32"))]
use pcbcraft_engine::authoring::remap::Scalar;
use pcbcraft_engine::authoring::{
    Primitive,
    remap::{self, PinRemap},
};
use std::sync::Arc;

/// A review binds one immutable report to both document and project runtime authority.
#[derive(Debug)]
pub(super) struct Review {
    token: u64,
    pub(super) document: u64,
    revision: u64,
    project: Option<Arc<pcbcraft_engine::Project>>,
    project_revision: u64,
    pins: Vec<PinRemap>,
    report: Value,
}
impl Review {
    /// A form can display and reuse only its own exact capture.
    fn owns(&self, draft: &Draft) -> bool {
        self.document == draft.document && self.revision == draft.revision && Some(self.token) == draft.token
    }

    /// Inspection exposes the exact report that Apply accepts, not a recomputed guess.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inspect(&self) -> Value {
        json!({"token":self.token.to_string(),"document":self.document.to_string(),"expected_revision":self.revision.to_string(),
            "project_revision":self.project_revision.to_string(),"report":self.report})
    }
}

/// Unapplied pin numbers belong to the dialog, not the library item or the project.
#[derive(Debug)]
pub(crate) struct Draft {
    document: u64,
    revision: u64,
    rows: Vec<Row>,
    token: Option<u64>,
    page: usize,
    error: Option<String>,
}
#[derive(Debug)]
struct Row {
    pin: PinRemap,
    before: String,
    name: String,
}
impl Draft {
    /// Tests can distinguish an unreviewed form from one that captured a token without exposing it.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn has_captured_review(&self) -> bool {
        self.token.is_some()
    }

    /// Include each logical terminal once even when multiple units display it.
    fn new(document: u64, revision: u64, item: &Item) -> Self {
        let mut seen = std::collections::BTreeSet::new();
        let mut rows = Vec::new();
        for (unit, objects) in std::iter::once((&item.unit, &item.objects)).chain(item.other_units.iter()) {
            for (object, primitive) in objects {
                if let Primitive::Pin { number, name, .. } = primitive
                    && seen.insert(number.clone())
                {
                    rows.push(Row {
                        pin: PinRemap { unit: unit.clone(), object: *object, number: number.clone() },
                        before: number.clone(),
                        name: name.clone().unwrap_or_default(),
                    });
                }
            }
        }
        Self { document, revision, rows, token: None, page: 0, error: None }
    }
    /// Only changed logical pins are submitted; simultaneous swaps need no temporary number.
    fn pins(&self) -> Vec<PinRemap> {
        self.rows.iter().filter(|row| row.before != row.pin.number).map(|row| row.pin.clone()).collect()
    }
}

/// Build displayed rows from the captured evidence, keeping draft names separate from
/// pinned-before-number names. Known current rows with empty nets mean unconnected;
/// unavailable associations remain unknown.
pub(super) fn report_rows(report: &Value, page: usize) -> Vec<pcbcraft_chrome::library_workspace::TableEntry> {
    use pcbcraft_chrome::library_workspace::TableEntry;
    let instances = report["instances"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut rows = Vec::new();
    let mut add = |owner: String, pins: &[Value]| {
        for pin in pins {
            let list = |key: &str| pin[key].as_array().map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
            let nets = list("current_nets").filter(|value| !value.is_empty()).map_or_else(
                || {
                    match pin["status"].as_str() {
                        Some("current") => "Unconnected",
                        Some("unplaced_terminal") => "Unit not placed",
                        Some("missing_terminal") => "Unknown: pinned terminal absent",
                        Some("different_revision") => "Unknown: no matching pinned revision",
                        Some("conflicting_definition") => "Unknown: conflicting published definition",
                        _ => "Unknown: no associated placed copy",
                    }
                    .into()
                },
                |names| format!("Net: {names}"),
            );
            let pinned_name = if matches!(pin["status"].as_str(), Some("current" | "unplaced_terminal")) {
                pin["current_name"].as_str().unwrap_or("(unnamed)")
            } else {
                "Unknown"
            };
            rows.push(TableEntry {
                key: format!("{owner}:{}:{}", pin["unit"], pin["object"]),
                selected: false,
                branch: None,
                nested: false,
                cells: vec![
                    format!("{owner} · {} / {}", pin["unit"].as_str().unwrap_or_default(), pin["object"].as_str().unwrap_or_default()),
                    format!("{} → {}", pin["before"].as_str().unwrap_or_default(), pin["after"].as_str().unwrap_or_default()),
                    pin["name"].as_str().unwrap_or("(unnamed)").into(),
                    pinned_name.into(),
                    nets,
                    list("current_pads").unwrap_or_else(|| {
                        if matches!(pin["status"].as_str(), Some("different_revision" | "missing_terminal" | "conflicting_definition")) {
                            "Unknown".into()
                        } else if matches!(pin["status"].as_str(), Some("current" | "unplaced_terminal")) {
                            "No selected package".into()
                        } else {
                            "Unknown: no associated placed copy".into()
                        }
                    }),
                ],
            });
        }
    };
    if instances.is_empty() {
        add("Definition".into(), report["changes"].as_array().map(Vec::as_slice).unwrap_or_default());
    } else {
        for instance in instances.iter().skip(page * 20).take(20) {
            add(
                format!("{} (r{})", instance["reference"].as_str().unwrap_or_default(), instance["pinned_revision"].as_str().unwrap_or_default()),
                instance["pins"].as_array().map(Vec::as_slice).unwrap_or_default(),
            );
        }
    }
    rows
}

/// A dedicated persisted type keeps the report generation separate from other saved UI state.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct ReportScrollGeneration(u64);

/// Review tokens restart with the app, but eframe restores scroll state. Allocate a fresh
/// generation on first use in each context; the cached active generation is not saved.
fn report_table_salt(ctx: &Context, token: u64) -> (&'static str, u64, u64) {
    let key = egui::Id::unique("pin_remap_scroll_generation");
    let generation = ctx.data_mut(|data| {
        if let Some(current) = data.get_temp::<u64>(key) {
            return current;
        }
        let next = data.get_persisted::<ReportScrollGeneration>(key).map_or(1, |last| last.0.wrapping_add(1));
        data.insert_persisted(key, ReportScrollGeneration(next));
        data.insert_temp(key, next);
        next
    });
    ("pin_remap_report", generation, token)
}

/// Paint the captured page without recomputing nets. Only the same review keeps its scroll position.
fn report_table(ui: &mut Ui, report: &Value, page: usize, token: u64) {
    let rows = report_rows(report, page);
    ui.push_id(report_table_salt(ui.ctx(), token), |ui| {
        pcbcraft_chrome::library_workspace::table(
            ui,
            &[
                "Instance / pin",
                "Before → after",
                "Draft name (unchanged)",
                "Pinned before-number name",
                "Current pinned nets / status",
                "Current pads",
            ],
            &rows,
            |_| {},
        )
    });
}

/// Phase-specific required/forbidden fields supplement the shared scalar schemas.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn phase_schema() -> Value {
    json!([
        {"properties":{"phase":{"const":"preview"}},"required":["pins"],"not":{"required":["token"]}},
        {"properties":{"phase":{"enum":["apply","cancel"]}},"required":["token"],"not":{"required":["pins"]}}
    ])
}

/// Refuse missing or substituted review inputs before either frontend changes editor state.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn check_phase(params: &Value) -> Result<(), String> {
    match params["phase"].as_str() {
        Some("preview") if params.get("token").is_some() => Err("Preview takes pins, not a review token.".into()),
        Some("preview") if params.get("pins").is_none() => Err("Preview requires pins.".into()),
        Some("preview") => Ok(()),
        Some("apply" | "cancel") if params.get("pins").is_some() => Err("Apply/cancel takes the reviewed token, not replacement pins.".into()),
        Some("apply" | "cancel") if params.get("token").is_none() => Err("A pin remap review token is required.".into()),
        Some("apply" | "cancel") => Ok(()),
        _ => Err("Choose preview, apply or cancel for the pin remap.".into()),
    }
}

impl PcbApp {
    /// Native transport uses the form's review handlers; conditional fields are checked without
    /// coercion. Preview refuses an open remap form; apply/cancel address its captured review.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn library_pin_remap_command(&mut self, document: u64, params: &Value) -> Result<(), String> {
        check_phase(params)?;
        let revision = Scalar::Runtime.parse(&params["expected_revision"])?;
        match params["phase"].as_str() {
            Some("preview") => {
                if matches!(self.dialog, Dialog::LibraryPinRemap(_)) {
                    return Err("Apply or cancel the open pin remap form before starting another live preview.".into());
                }
                let pins = remap::decode(&params["pins"])?;
                let token = self.preview_library_pin_remap(document, revision, pins)?;
                let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
                let mut draft = Draft::new(document, revision, &doc.session.snapshot());
                // The caller may choose any appearance of a shared terminal. Populate the form's
                // representative from the resolved logical change, never from local object IDs.
                let review = self.library_editors.pin_remap.as_ref().ok_or("Missing pin remap review.")?;
                for row in &mut draft.rows {
                    if let Some(change) =
                        review.report["changes"].as_array().and_then(|changes| changes.iter().find(|change| change["before"] == row.before))
                        && let Some(number) = change["after"].as_str()
                    {
                        row.pin.number = number.into();
                    }
                }
                draft.token = Some(token);
                self.dialog = Dialog::LibraryPinRemap(Box::new(draft));
            }
            Some(phase @ ("apply" | "cancel")) => {
                let token = Scalar::Token.parse(&params["token"])?;
                let review = self.library_editors.pin_remap.as_ref().ok_or(if phase == "apply" {
                    "Preview the pin remap before applying it."
                } else {
                    "No pin remap review is open."
                })?;
                if review.document != document || review.revision != revision || review.token != token {
                    return Err("Pin remap review does not match this document, revision or token.".into());
                }
                if !matches!(&self.dialog, Dialog::LibraryPinRemap(draft) if draft.document == document && draft.revision == revision && draft.token == Some(token))
                {
                    return Err("Pin remap review has no matching form owner; inspect and preview again.".into());
                }
                if phase == "apply" {
                    self.apply_library_pin_remap(document, revision, token)?;
                } else {
                    self.cancel_library_pin_remap(document, revision, token)?;
                }
                if matches!(&self.dialog, Dialog::LibraryPinRemap(draft) if draft.document == document && draft.revision == revision && draft.token == Some(token))
                {
                    self.dialog = Dialog::None;
                }
            }
            _ => return Err("Choose preview, apply or cancel for the pin remap.".into()),
        }
        Ok(())
    }

    /// Displacing a form retires only its captured authority, never a newer review.
    /// Closing or switching project dialogs must call this before replacing the owner.
    pub(crate) fn dismiss_library_pin_remap(&mut self) {
        let Dialog::LibraryPinRemap(draft) = &self.dialog else { return };
        if let Some(token) = draft.token {
            let (document, revision) = (draft.document, draft.revision);
            let _ = self.cancel_library_pin_remap(document, revision, token);
        }
        self.dialog = Dialog::None;
    }

    /// Open a private form without allocating history or renumbering any placed copy.
    pub(super) fn begin_library_pin_remap(&mut self, document: u64) -> Result<(), String> {
        if !matches!(self.dialog, Dialog::None) || self.library_editors.busy() {
            return Err("Finish the current dialog or library operation before remapping pins.".into());
        }
        let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
        if doc.pending_any() || doc.session.snapshot().kind != Kind::Symbol {
            return Err("Remap requires a symbol with no pending property fields.".into());
        }
        self.dialog = Dialog::LibraryPinRemap(Box::new(Draft::new(document, doc.session.revision(), &doc.session.snapshot())));
        Ok(())
    }

    /// Both frontends capture the same validated report and monotonic review token. A failed preview
    /// leaves the previous review intact; project snapshots are retained by Arc, not copied to undo.
    /// Native capture checks the prospective complete live reply's encoded byte limit, not the
    /// graph/report preparation cost or peak heap, before publishing any new token or report.
    pub(super) fn preview_library_pin_remap(&mut self, document: u64, revision: u64, pins: Vec<PinRemap>) -> Result<u64, String> {
        if self.library_editors.busy() {
            return Err("Finish the library operation before reviewing a remap.".into());
        }
        let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
        if doc.session.revision() != revision || doc.pending_any() {
            return Err("Library document changed or has pending fields; inspect before reviewing.".into());
        }
        let project = self.session.snapshot();
        let report = remap::preview(
            &doc.session.snapshot(),
            &pins,
            doc.session.published().map(|address| address.revision),
            project.as_deref(),
            Some(self.session.libraries()),
        )?;
        let token = self.library_editors.next_remap.checked_add(1).ok_or("Pin remap review ID limit reached.")?;
        let review = Review { token, document, revision, project, project_revision: self.session.revision(), pins, report };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let inspection = crate::window_commands::inspect_editor_with_remap(self, Some(review.inspect()), true);
            pcbcraft_mcp::live::check_reply_size(&inspection).map_err(|error| format!("Pin remap review cannot be captured: {error}"))?;
        }
        self.library_editors.pin_remap = Some(review);
        self.library_editors.next_remap = token;
        Ok(token)
    }

    /// Apply only the report the caller reviewed. Reopen, undo, project edits, definition edits and
    /// unapplied fields invalidate it; refusals preserve both histories and the review for inspection.
    pub(super) fn apply_library_pin_remap(&mut self, document: u64, revision: u64, token: u64) -> Result<(), String> {
        if self.library_editors.busy() {
            return Err("Finish the library operation before applying a remap.".into());
        }
        let review = self.library_editors.pin_remap.as_ref().ok_or("Preview the pin remap before applying it.")?;
        if review.document != document || review.revision != revision || review.token != token {
            return Err("Pin remap review does not match this document, revision or token.".into());
        }
        let same_project = match (&review.project, self.session.snapshot()) {
            (None, None) => true,
            (Some(before), Some(now)) => Arc::ptr_eq(before, &now),
            _ => false,
        };
        if !same_project || review.project_revision != self.session.revision() {
            return Err("Project changed; preview current nets again before applying.".into());
        }
        let doc = self.library_editors.documents.get(&document).ok_or("Unknown library document ID.")?;
        if doc.pending_any() {
            return Err("Apply or reset all pending property fields before remapping.".into());
        }
        let pins = review.pins.clone();
        self.library_editors.edit(document, revision, Action::RemapPins { pins })?;
        self.library_editors.pin_remap = None;
        Ok(())
    }

    /// Cancel only the addressed review; stale clients cannot dismiss a newer person's report.
    pub(super) fn cancel_library_pin_remap(&mut self, document: u64, revision: u64, token: u64) -> Result<(), String> {
        let review = self.library_editors.pin_remap.as_ref().ok_or("No pin remap review is open.")?;
        if review.document != document || review.revision != revision || review.token != token {
            return Err("Pin remap review does not match this document, revision or token.".into());
        }
        self.library_editors.pin_remap = None;
        Ok(())
    }

    /// Current nets are paged for painting only: the captured report retains every affected instance.
    /// Editing a correspondence withdraws its review, so Apply can never use an unseen replacement.
    pub(crate) fn library_pin_remap_dialog(&mut self, ctx: &Context, mut draft: Box<Draft>) {
        let mut preview = false;
        let mut apply = false;
        let mut cancel = false;
        let mut changed = false;
        let dismissed = shell::scrolling_dialog_width(ctx, "library_pin_remap", "Remap symbol pins", 860.0, |ui| {
            ui.label("Renumber the library definition only. Placed copies, wires and native part maps stay unchanged.");
            for row in &mut draft.rows {
                ui.push_id((&row.pin.unit, row.pin.object), |ui| {
                    ui.label(format!("{} · {} · {}", row.pin.unit, row.before, row.name));
                    changed |= properties::text(ui, &format!("Pin {}", row.before), &mut row.pin.number).changed();
                });
            }
            if !changed && let Some(review) = self.library_editors.pin_remap.as_ref().filter(|review| review.owns(&draft)) {
                ui.separator();
                ui.label("Before → after · current placed nets (not predicted reconnection)");
                ui.label("Pin names are unchanged by renumbering.");
                let instances = review.report["instances"].as_array().map(Vec::as_slice).unwrap_or_default();
                if instances.is_empty() {
                    ui.label(if review.project.is_some() {
                        "No placed copies of this library identity in the open project."
                    } else {
                        "No project is open; placed names, nets and pads are unavailable."
                    });
                }
                let pages = instances.len().div_ceil(20).max(1);
                draft.page = draft.page.min(pages - 1);
                ui.horizontal(|ui| {
                    if pcbcraft_chrome::controls::enabled_button(ui, draft.page > 0, "Previous instances").clicked() {
                        draft.page -= 1;
                    }
                    ui.label(format!("Page {} of {} · {} instances", draft.page + 1, pages, instances.len()));
                    if pcbcraft_chrome::controls::enabled_button(ui, draft.page + 1 < pages, "Next instances").clicked() {
                        draft.page += 1;
                    }
                });
                ui.allocate_ui(egui::vec2(ui.available_width(), 260.0), |ui| report_table(ui, &review.report, draft.page, review.token));
            }
            if let Some(error) = &draft.error {
                ui.label(shell::error(error));
            }
            ui.horizontal(|ui| {
                preview = pcbcraft_chrome::controls::button(ui, "Preview current nets").clicked();
                apply = pcbcraft_chrome::controls::enabled_button(ui, draft.token.is_some() && !changed, "Apply remap").clicked();
                cancel = pcbcraft_chrome::controls::button(ui, "Cancel").clicked();
            });
        });
        if changed {
            if let Some(token) = draft.token.take() {
                let _ = self.cancel_library_pin_remap(draft.document, draft.revision, token);
            }
            draft.error = None;
        }
        if cancel || dismissed {
            if let Some(token) = draft.token {
                let _ = self.cancel_library_pin_remap(draft.document, draft.revision, token);
            }
            return;
        }
        if preview {
            let pins = self
                .library_editors
                .pin_remap
                .as_ref()
                .filter(|review| review.owns(&draft))
                .map_or_else(|| draft.pins(), |review| review.pins.clone());
            let result = if pins.is_empty() {
                Err("Change at least one pin number before previewing.".into())
            } else {
                self.preview_library_pin_remap(draft.document, draft.revision, pins)
            };
            match result {
                Ok(token) => {
                    draft.token = Some(token);
                    draft.page = 0;
                    draft.error = None;
                }
                Err(error) => {
                    draft.error = Some(error);
                }
            }
        } else if apply && let Some(token) = draft.token {
            // A refresh takes precedence; the replacement report must be displayed before Apply.
            match self.apply_library_pin_remap(draft.document, draft.revision, token) {
                Ok(()) => {
                    self.repaint_editors(ctx);
                    return;
                }
                Err(error) => draft.error = Some(error),
            }
        }
        self.dialog = Dialog::LibraryPinRemap(draft);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// A fresh capture starts at the top, including when the app restores a scrolled capture's memory and reuses its token.
    #[test]
    fn pin_remap_report_starts_at_top_for_new_review() {
        let pins: Vec<_> =
            (1..=40).map(|object| json!({"unit":"main","object":object.to_string(),"before":"1","after":"2","name":"Test pin"})).collect();
        let report = json!({"changes":pins});
        let draw = |ui: &mut Ui, (token, offset, id): &mut (u64, f32, egui::Id)| {
            report_table(ui, &report, 0, *token);
            // UiBuilder and ScrollArea both hash their supplied salts before passing them to egui.
            let scope = ui.make_persistent_id(egui::IdSalt::new(report_table_salt(ui.ctx(), *token)));
            *id = scope.with(egui::IdSalt::new("library_table_vertical"));
            *offset = egui::scroll_area::State::load(ui.ctx(), *id).expect("report scroll state").clamped_offset().y;
        };
        let builder = || egui_kittest::Harness::builder().with_size(egui::vec2(760.0, 260.0));
        let initial = (1, 0.0, egui::Id::NULL);
        let mut h = builder().with_context_setup(pcbcraft_chrome::install).build_ui_state(draw, initial);
        h.run();
        assert_eq!(h.state().1, 0.0);
        h.hover_at(egui::pos2(300.0, 130.0));
        h.run();
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -200.0),
            modifiers: egui::Modifiers::NONE,
        });
        h.run();
        assert!(h.state().1 > 0.0, "first review should actually scroll");
        h.hover_at(egui::pos2(770.0, 270.0));
        h.run();
        assert!(h.state().1 > 0.0, "same review should retain its offset");
        let old_scroll_id = h.state().2;
        let saved = h.ctx.data(|data| serde_json::to_vec(data).unwrap());
        h.state_mut().0 = 2;
        h.run();
        assert_eq!(h.state().1, 0.0, "new review must start at the top");

        let mut restarted = builder()
            .with_context_setup(move |ctx| {
                pcbcraft_chrome::install(ctx);
                ctx.data_mut(|data| *data = serde_json::from_slice(&saved).unwrap());
                let old = egui::scroll_area::State::load(ctx, old_scroll_id).expect("saved report scroll state");
                assert!(old.clamped_offset().y > 0.0, "the restarted context must contain the stale offset");
            })
            .build_ui_state(draw, initial);
        restarted.run();
        assert_ne!(restarted.state().2, old_scroll_id);
        assert_eq!(restarted.state().1, 0.0, "reused token after restart must start at the top");
    }

    /// Live previews cannot replace either typed input or a captured review, even for the same
    /// document. Refusal preserves both histories and leaves addressed cancellation available.
    #[test]
    fn pin_remap_live_preview_preserves_an_open_form_and_review() {
        for reviewed in [false, true] {
            for other_document in [false, true] {
                let mut app = PcbApp::default();
                app.session.execute("project.new", &json!({})).unwrap();
                app.new_library_item(Kind::Symbol);
                let document = app.library_editors.active.unwrap();
                app.new_library_item(Kind::Symbol);
                let other = app.library_editors.active.unwrap();
                assert_ne!(document, other);
                let ctx = Context::default();
                let pins = json!([{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}]);
                if reviewed {
                    app.run_window_command(
                        &ctx,
                        "app.library_pin_remap",
                        &json!({"document":document.to_string(),"expected_revision":"0","phase":"preview","pins":pins}),
                    )
                    .unwrap();
                } else {
                    app.begin_library_pin_remap(document).unwrap();
                    let Dialog::LibraryPinRemap(draft) = &mut app.dialog else { panic!("remap form") };
                    draft.rows[0].pin.number = "7".into();
                }
                let form = format!("{:?}", app.dialog);
                let editors = app.library_editors.inspect();
                let counter = app.library_editors.next_remap;
                let project = app.session.inspect();
                let snapshot = app.session.snapshot().unwrap();
                let bytes: Vec<_> = app.library_editors.documents.values().map(|doc| doc.session.snapshot().encode().unwrap()).collect();
                let target = if other_document { other } else { document };
                let error = app
                    .run_window_command(
                        &ctx,
                        "app.library_pin_remap",
                        &json!({"document":target.to_string(),"expected_revision":"0","phase":"preview","pins":pins}),
                    )
                    .unwrap_err();
                assert_eq!(error, "Apply or cancel the open pin remap form before starting another live preview.");
                assert_eq!(format!("{:?}", app.dialog), form);
                assert_eq!(app.library_editors.inspect(), editors);
                assert_eq!(app.library_editors.next_remap, counter);
                assert_eq!(app.session.inspect(), project);
                assert!(Arc::ptr_eq(&snapshot, &app.session.snapshot().unwrap()));
                assert_eq!(app.library_editors.documents.values().map(|doc| doc.session.snapshot().encode().unwrap()).collect::<Vec<_>>(), bytes);
                if reviewed {
                    app.run_window_command(
                        &ctx,
                        "app.library_pin_remap",
                        &json!({"document":document.to_string(),"expected_revision":"0","phase":"cancel","token":editors["pin_remap"]["token"]}),
                    )
                    .unwrap();
                    assert!(matches!(app.dialog, Dialog::None));
                    assert!(app.library_editors.pin_remap.is_none());
                    assert_eq!(app.library_editors.next_remap, counter);
                    assert_eq!(app.session.inspect(), project);
                }
            }
        }
    }

    /// A live caller may select a nonrepresentative shared appearance. The visible form must
    /// show that same logical change, rather than the old number on its chosen representative.
    #[test]
    fn pin_remap_shared_appearance_populates_the_visible_form() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Symbol);
        let document = app.library_editors.active.unwrap();
        let pin = app.library_editors.documents[&document].session.snapshot().objects[&1].clone();
        app.library_editors.edit(document, 0, Action::AddUnit { unit: "aux".into() }).unwrap();
        app.library_editors.edit(document, 1, Action::Insert { primitive: pin }).unwrap();
        app.library_pin_remap_command(
            document,
            &json!({"expected_revision":"2","phase":"preview","pins":[
                {"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}
            ]}),
        )
        .unwrap();
        let Dialog::LibraryPinRemap(draft) = &app.dialog else { panic!("remap dialog") };
        let row = draft.rows.iter().find(|row| row.before == "1").unwrap();
        assert_eq!(row.pin.unit, "aux");
        assert_eq!(row.pin.number, "2");
        assert_eq!(app.library_editors.documents[&document].session.revision(), 2);
    }
}
