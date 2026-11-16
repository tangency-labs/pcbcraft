//! The Parameters table that Properties shows for the current sheet when nothing is selected.
//!
//! It lists the sheet's and the project's parameters as editable values. A standard value that
//! has not been set shows its inherited value as a dim hint. Leaving a field submits one
//! `parameter.set`, the same command MCP uses; clearing a field removes the value, so a standard
//! sheet parameter returns to its default. This module holds only presentation and drafts; the
//! host runs the returned [`Change`].
use egui::Ui;
use pcbcraft_chrome::properties as chrome;
use pcbcraft_engine::{
    Project, SheetTarget,
    parameters::{Row, Scope, Source},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// A field: its scope and parameter name.
type Field = (Scope, String);

/// Text drafts for one sheet. A new revision updates only the fields whose values changed; a new
/// sheet replaces them all.
#[derive(Debug, Default)]
pub(crate) struct Drafts {
    key: Option<(SheetTarget, u64)>,
    values: BTreeMap<Field, String>,
    /// Each own value as last loaded, so a field being typed in can be told from a stale one.
    stored: BTreeMap<Field, String>,
    /// The value set elsewhere while a field held unsaved text, until that field is left.
    conflicts: BTreeMap<Field, String>,
    new_name: BTreeMap<Scope, String>,
    /// Why the New name of a scope was not added, until it is changed and left again.
    new_error: BTreeMap<Scope, String>,
}

/// One edit for the host to submit as `parameter.set`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    scope: Scope,
    name: String,
    /// `None` removes the parameter.
    value: Option<String>,
}

impl Change {
    /// The `parameter.set` parameters for `sheet` at `revision`.
    pub(crate) fn params(&self, sheet: SheetTarget, revision: u64) -> Value {
        let mut params = json!({"name":self.name,"value":self.value,"expected_revision":revision});
        if self.scope == Scope::Sheet {
            params["scope"] = json!("sheet");
            params["sheet"] = json!(sheet.id());
        } else {
            params["scope"] = json!("project");
        }
        params
    }
}

impl Drafts {
    /// Bring the drafts up to date when the sheet or revision differs from the last frame.
    ///
    /// Another sheet reloads every field and clears the New fields and their errors, so an
    /// unfinished name is never added to a sheet it wasn't typed for. A new revision of the same
    /// sheet reloads only fields without unsaved text, so an edit made elsewhere (such as over
    /// MCP) never discards what is being typed in another field. When the value under unsaved
    /// text changes, the text stays and the new value is kept in `conflicts` to be shown. A field
    /// whose parameter no longer exists is dropped with its text.
    fn sync(&mut self, project: &Project, sheet: SheetTarget, revision: u64) {
        if self.key == Some((sheet, revision)) {
            return;
        }
        if self.key.is_none_or(|(previous, _)| previous != sheet) {
            self.new_name.clear();
            self.new_error.clear();
            self.values.clear();
            self.stored.clear();
            self.conflicts.clear();
        }
        self.key = Some((sheet, revision));
        let mut stored = BTreeMap::new();
        let mut present = BTreeSet::new();
        for scope in [Scope::Sheet, Scope::Project] {
            for row in project.parameter_rows(scope, Some(sheet)).into_iter().filter(|row| row.source != Source::BuiltIn) {
                if row.source == Source::Own {
                    stored.insert((scope, row.name.clone()), row.value);
                }
                present.insert((scope, row.name));
            }
        }
        let fields: BTreeSet<Field> = self.values.keys().chain(stored.keys()).cloned().collect();
        for field in fields {
            let old = self.stored.get(&field).map_or("", String::as_str);
            let new = stored.get(&field).map_or("", String::as_str);
            let draft = self.values.get(&field).map_or("", String::as_str);
            if !present.contains(&field) {
                self.values.remove(&field);
                self.conflicts.remove(&field);
            } else if draft == old || draft == new {
                // Unedited, or already saved: show the current value.
                self.conflicts.remove(&field);
                self.values.insert(field, new.to_owned());
            } else if old != new {
                self.conflicts.insert(field, new.to_owned());
            }
        }
        self.stored = stored;
    }
}

/// What a field shows when empty: the resolved inherited value, or the reference it inherits
/// when that resolves to nothing, so `@project.Revision` tells where an empty value comes from.
fn hint(project: &Project, sheet: SheetTarget, row: &Row) -> String {
    if row.source != Source::Default {
        return String::new();
    }
    let resolved = project.resolve_text(Some(sheet), &row.value).text;
    if resolved.is_empty() { row.value.clone() } else { resolved }
}

/// The note under a field whose value `external` was set elsewhere while it held unsaved text.
fn conflict_note(external: &str) -> String {
    if external.is_empty() {
        "Removed elsewhere. Leaving this field saves your text instead.".into()
    } else {
        format!("Changed elsewhere to “{external}”. Leaving this field saves your text instead.")
    }
}

/// The edit a field implies when it loses focus: clearing a nonempty own value removes it, and a
/// different value sets it. `None` when nothing changed, including leaving an empty own value
/// (such as one New just added) empty.
fn commit(scope: Scope, row: &Row, draft: &str) -> Option<Change> {
    let own = (row.source == Source::Own).then_some(row.value.as_str());
    match (own, draft.is_empty()) {
        (Some(current), true) if !current.is_empty() => Some(Change { scope, name: row.name.clone(), value: None }),
        (_, true) => None,
        (current, false) if current != Some(draft) => Some(Change { scope, name: row.name.clone(), value: Some(draft.to_owned()) }),
        _ => None,
    }
}

/// The edit the New field implies when it loses focus with `name` typed: an empty parameter, or
/// an error when `rows` (built-ins included) already holds that name ignoring case, because
/// setting it would overwrite the existing value.
fn add(scope: Scope, rows: &[Row], name: &str) -> Result<Change, String> {
    match rows.iter().find(|row| row.name.eq_ignore_ascii_case(name)) {
        Some(row) => Err(format!("There already is a parameter `{}`; edit its field instead.", row.name)),
        None => Ok(Change { scope, name: name.to_owned(), value: Some(String::new()) }),
    }
}

/// Draw one scope's editable rows and a New field that adds an empty parameter when left. A name
/// that already exists stays in the field with an error. Returns the edit to submit, if any.
fn scope_rows(ui: &mut Ui, project: &Project, sheet: SheetTarget, scope: Scope, drafts: &mut Drafts) -> Option<Change> {
    let mut change = None;
    let rows = project.parameter_rows(scope, Some(sheet));
    for row in rows.iter().filter(|row| row.source != Source::BuiltIn) {
        let field = (scope, row.name.clone());
        let draft = drafts.values.entry(field.clone()).or_default();
        // A stable ID per scope and name keeps focus with its parameter when rows are added
        // or removed; positional IDs would hand a leaving field's focus to its neighbour.
        let response = ui.push_id((scope, &row.name), |ui| chrome::text_hint(ui, &row.name, draft, &hint(project, sheet, row))).inner;
        let response = if pcbcraft_engine::parameters::has_references(draft) {
            response.on_hover_text(project.resolve_text(Some(sheet), draft).text)
        } else {
            response
        };
        if response.lost_focus() {
            drafts.conflicts.remove(&field);
            change = change.or_else(|| commit(scope, row, draft));
        } else if let Some(external) = drafts.conflicts.get(&field) {
            ui.label(pcbcraft_chrome::shell::error(conflict_note(external)));
        }
    }
    let name = drafts.new_name.entry(scope).or_default();
    // A space can't appear in a parameter name, so no row can share the New field's ID.
    if ui.push_id((scope, "New field"), |ui| chrome::text_hint(ui, "New", name, "Add a parameter by name")).inner.lost_focus()
        && !name.trim().is_empty()
    {
        match add(scope, &rows, name.trim()) {
            Ok(added) => {
                name.clear();
                drafts.new_error.remove(&scope);
                change = change.or(Some(added));
            }
            Err(error) => {
                drafts.new_error.insert(scope, error);
            }
        }
    }
    if let Some(error) = drafts.new_error.get(&scope) {
        ui.label(pcbcraft_chrome::shell::error(error.as_str()));
    }
    change
}

/// Draw the sheet heading and the sheet and project parameter groups. Returns the edit to
/// submit; the host runs it and reports errors.
pub(crate) fn show(ui: &mut Ui, project: &Project, sheet: SheetTarget, revision: u64, drafts: &mut Drafts) -> Option<Change> {
    drafts.sync(project, sheet, revision);
    let page = project.resolve_text(Some(sheet), "Page @instance.Page of @instance.Pages").text;
    let name = project.sheets().find(|info| info.target == sheet).map_or("", |info| info.name);
    chrome::identity(ui, name, &page);
    let sheet_change = chrome::group(ui, "Sheet parameters", |ui| scope_rows(ui, project, sheet, Scope::Sheet, drafts));
    let project_change = chrome::group(ui, "Project parameters", |ui| scope_rows(ui, project, sheet, Scope::Project, drafts));
    sheet_change.or(project_change)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: &str, source: Source) -> Row {
        Row { name: "Revision".into(), value: value.into(), source }
    }

    /// Clearing an own value removes it, clearing a default does nothing, and only a different
    /// value sets one.
    #[test]
    fn leaving_a_field_commits_only_real_changes() {
        let own = row("B", Source::Own);
        assert_eq!(commit(Scope::Sheet, &own, ""), Some(Change { scope: Scope::Sheet, name: "Revision".into(), value: None }));
        assert_eq!(commit(Scope::Sheet, &own, "B"), None);
        assert_eq!(commit(Scope::Sheet, &own, "C").and_then(|c| c.value), Some("C".into()));
        let default = row("@project.Revision", Source::Default);
        assert_eq!(commit(Scope::Sheet, &default, ""), None);
        assert_eq!(commit(Scope::Sheet, &default, "A").and_then(|c| c.value), Some("A".into()));
        // An empty own value left empty, as after New, stays.
        assert_eq!(commit(Scope::Sheet, &row("", Source::Own), ""), None);
    }

    /// Switching sheets drops the New fields' unfinished names and errors; a new revision on the
    /// same sheet keeps them.
    #[test]
    fn new_drafts_belong_to_their_sheet() {
        let mut session = pcbcraft_engine::Session::default();
        session.apply("project.new", &serde_json::json!({})).unwrap();
        session.apply(pcbcraft_engine::sheets::CREATE_SHEET, &serde_json::json!({"name":"Aux","expected_revision":session.revision()})).unwrap();
        let project = session.snapshot().unwrap();
        let [sheet, other] = [0, 1].map(|n| project.sheets().nth(n).unwrap().target);
        let mut drafts = Drafts::default();
        drafts.sync(&project, sheet, 1);
        drafts.new_name.insert(Scope::Sheet, "Gai".into());
        drafts.new_error.insert(Scope::Sheet, "error".into());
        drafts.sync(&project, sheet, 2);
        assert_eq!(drafts.new_name.get(&Scope::Sheet).map(String::as_str), Some("Gai"));
        drafts.sync(&project, other, 2);
        assert!(drafts.new_name.is_empty() && drafts.new_error.is_empty());
    }

    /// A new revision keeps unsaved text in fields whose values did not change, reloads the
    /// others, and keeps the text with a conflict when the value under it changed elsewhere.
    #[test]
    fn unsaved_text_survives_unrelated_edits() {
        let mut session = pcbcraft_engine::Session::default();
        session.apply("project.new", &serde_json::json!({})).unwrap();
        let set = |session: &mut pcbcraft_engine::Session, name: &str, value: &str| {
            let params = serde_json::json!({"scope":"project","name":name,"value":value,"expected_revision":session.revision()});
            session.apply(pcbcraft_engine::parameters::SET, &params).unwrap();
        };
        set(&mut session, "Company", "Acme");
        set(&mut session, "Revision", "A");
        let sheet = session.snapshot().unwrap().sheet_target();
        let field = |name: &str| (Scope::Project, name.to_owned());
        let mut drafts = Drafts::default();
        drafts.sync(&session.snapshot().unwrap(), sheet, session.revision());
        drafts.values.insert(field("Revision"), "B".into());
        // Company changes elsewhere: Revision keeps its unsaved text, Company reloads.
        set(&mut session, "Company", "Other");
        drafts.sync(&session.snapshot().unwrap(), sheet, session.revision());
        assert_eq!(drafts.values[&field("Revision")], "B");
        assert_eq!(drafts.values[&field("Company")], "Other");
        assert!(drafts.conflicts.is_empty());
        // Revision itself changes elsewhere: the text stays and the new value is reported.
        set(&mut session, "Revision", "C");
        drafts.sync(&session.snapshot().unwrap(), sheet, session.revision());
        assert_eq!(drafts.values[&field("Revision")], "B");
        assert_eq!(drafts.conflicts.get(&field("Revision")).map(String::as_str), Some("C"));
        // Saving the text clears the conflict.
        set(&mut session, "Revision", "B");
        drafts.sync(&session.snapshot().unwrap(), sheet, session.revision());
        assert!(drafts.conflicts.is_empty());
        assert_eq!(drafts.values[&field("Revision")], "B");
    }

    /// New refuses any existing name ignoring case, built-ins included, and adds an empty
    /// parameter otherwise.
    #[test]
    fn new_refuses_existing_names() {
        let rows = [row("B", Source::Own), Row { name: "Name".into(), value: "Main".into(), source: Source::BuiltIn }];
        assert!(add(Scope::Sheet, &rows, "Revision").is_err());
        assert!(add(Scope::Sheet, &rows, "revision").is_err());
        assert!(add(Scope::Sheet, &rows, "name").is_err());
        assert_eq!(add(Scope::Sheet, &rows, "Checked"), Ok(Change { scope: Scope::Sheet, name: "Checked".into(), value: Some(String::new()) }));
    }
}
