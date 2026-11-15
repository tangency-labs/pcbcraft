//! Parameters: named values kept on the project and on each sheet, and the `@scope.Name`
//! references that texts use to show them.
//!
//! A reference always names its scope (`@project`, `@sheet` or `@instance`) and never falls back
//! to another scope. A value may hold references itself; they are resolved a few levels deep, so
//! a loop ends as an unknown reference. `@@` writes a literal `@`, and an `@` that is not
//! followed by a scope and a dot stays plain text, so an e-mail address in a note stays as typed.
//!
//! Every sheet has the standard parameters of [`SHEET_DEFAULTS`] without storing them: `Title`
//! shows the sheet name and the others show the project's value of the same name, so a value is
//! set once for the project and each sheet may override it. The project's standard parameters
//! ([`PROJECT_STANDARD`]) default to empty. Only values that differ from those defaults are
//! stored, so a project that never sets one saves the same bytes as before.
//!
//! Ported from Coline (`crates/format/src/parameter.rs`, `crates/model/src/params.rs` and
//! `crates/engine/src/commands/parameters.rs`, MIT OR Apache-2.0); see `ATTRIBUTION.md`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::{CommandError, Param, ParamKind, Project, Session, SheetTarget};

/// Set, rename or remove one project or sheet parameter as one undo step.
pub const SET: &str = "parameter.set";
/// List a project's or sheet's parameters with their effective values.
pub const LIST: &str = "parameter.list";
/// Replace the parameter references in a text as a sheet would show it.
pub const RESOLVE: &str = "parameter.resolve";

/// A list of parameters: name to value, sorted by name.
pub type Parameters = BTreeMap<String, String>;

/// The most parameters one list may hold.
pub const MAX_PARAMETERS: usize = 256;
/// The longest parameter name, in bytes.
pub const MAX_NAME_LEN: usize = 64;
/// The longest parameter value, in bytes.
pub const MAX_VALUE_LEN: usize = 1000;
/// The longest text `parameter.resolve` accepts, in bytes.
pub const MAX_TEXT_LEN: usize = 4096;
/// How deep references inside parameter values are followed.
const MAX_DEPTH: usize = 8;

/// The standard project parameters. They exist on every project and are empty until set.
pub const PROJECT_STANDARD: [&str; 4] = ["Revision", "Date", "Company", "DrawnBy"];

/// The standard sheet parameters and the value each has until the sheet sets its own.
pub const SHEET_DEFAULTS: [(&str, &str); 5] = [
    ("Title", "@sheet.Name"),
    ("Revision", "@project.Revision"),
    ("Date", "@project.Date"),
    ("Company", "@project.Company"),
    ("DrawnBy", "@project.DrawnBy"),
];

pub(crate) const SCOPE: Param =
    Param { required: true, name: "scope", summary: "Where the parameter list lives: project or sheet", kind: ParamKind::ParameterScope };
pub(crate) const SHEET: Param = Param {
    required: false,
    name: "sheet",
    summary: "Sheet UUID from project inspection; required for the sheet scope, and for @sheet and @instance references",
    kind: ParamKind::SheetId,
};
pub(crate) const NAME: Param = Param {
    required: true,
    name: "name",
    summary: "Parameter name: an ASCII letter, then letters, digits and _ (at most 64 bytes)",
    kind: ParamKind::ParameterName,
};
pub(crate) const VALUE: Param = Param {
    required: false,
    name: "value",
    summary: "The new value, which may hold @scope.Name references; null removes the parameter. Omit only when renaming",
    kind: ParamKind::ParameterValue,
};
pub(crate) const RENAME: Param =
    Param { required: false, name: "rename", summary: "A new name, keeping the value unless `value` is given", kind: ParamKind::ParameterName };
pub(crate) const TEXT: Param =
    Param { required: true, name: "text", summary: "Text such as `Rev @project.Revision`", kind: ParamKind::ParameterText };

/// Why a parameter list was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParameterError {
    /// The list is too long.
    TooMany,
    /// A name breaks the naming rule.
    Name(String),
    /// A value is too long.
    Value(String),
}

impl std::fmt::Display for ParameterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany => write!(f, "more than {MAX_PARAMETERS} parameters"),
            Self::Name(name) => write!(f, "parameter name `{name}`: use 1 to {MAX_NAME_LEN} letters, digits and `_`, starting with a letter"),
            Self::Value(name) => write!(f, "parameter `{name}`: the value is longer than {MAX_VALUE_LEN} bytes"),
        }
    }
}

/// Whether `name` can name a parameter: an ASCII letter, then letters, digits and `_`, at most
/// [`MAX_NAME_LEN`] bytes. The rule keeps a reference unambiguous inside running text.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_NAME_LEN && chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Check every name and value of a stored list, and its length. Decoding refuses a list that
/// fails, so stored lists always pass.
pub fn check(parameters: &Parameters) -> Result<(), ParameterError> {
    if parameters.len() > MAX_PARAMETERS {
        return Err(ParameterError::TooMany);
    }
    for (name, value) in parameters {
        if !valid_name(name) {
            return Err(ParameterError::Name(name.clone()));
        }
        if value.len() > MAX_VALUE_LEN {
            return Err(ParameterError::Value(name.clone()));
        }
    }
    Ok(())
}

/// The levels a reference can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// The whole project.
    Project,
    /// One sheet.
    Sheet,
    /// The instance of a sheet that is shown: its name and its page number.
    Instance,
}

impl Scope {
    /// Every scope, in the order completion lists them.
    pub const ALL: [Self; 3] = [Self::Project, Self::Sheet, Self::Instance];

    /// The word written after `@`.
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Sheet => "sheet",
            Self::Instance => "instance",
        }
    }

    /// The scope a keyword names; `None` for any other word.
    pub fn from_keyword(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.keyword() == word)
    }

    /// The read-only parameters every item of this scope has, with what they say.
    pub const fn builtins(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Project => &[("Name", "The project name")],
            Self::Sheet => &[("Name", "The sheet name")],
            Self::Instance => {
                &[("Name", "The sheet name of this instance"), ("Page", "The page number, in Browser order"), ("Pages", "The number of pages")]
            }
        }
    }

    /// Whether `name` is one of this scope's read-only built-ins, ignoring case.
    fn is_builtin(self, name: &str) -> bool {
        self.builtins().iter().any(|(builtin, _)| builtin.eq_ignore_ascii_case(name))
    }
}

/// One piece of a text: plain text or a reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    /// Text drawn as it is.
    Text(&'a str),
    /// A reference, `@scope.Name`.
    Reference {
        /// The scope.
        scope: Scope,
        /// The parameter name.
        name: &'a str,
        /// The reference as typed, for messages and unknown references.
        raw: &'a str,
    },
}

/// Split a text into plain text and references. `@@` becomes a plain `@`; an `@` that is not
/// followed by a known scope, a dot and a name starting with a letter stays plain text.
pub fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut plain_from = 0;
    let mut i = 0;
    while let Some(offset) = text.get(i..).and_then(|rest| rest.find('@')) {
        let at = i + offset;
        let after = text.get(at + 1..).unwrap_or("");
        if after.starts_with('@') {
            push_text(&mut out, text.get(plain_from..=at));
            i = at + 2;
            plain_from = i;
            continue;
        }
        match reference_at(after) {
            Some((scope, name, len)) => {
                push_text(&mut out, text.get(plain_from..at));
                out.push(Piece::Reference { scope, name, raw: text.get(at..at + 1 + len).unwrap_or("") });
                i = at + 1 + len;
                plain_from = i;
            }
            None => i = at + 1,
        }
    }
    push_text(&mut out, text.get(plain_from..));
    out
}

/// Append a nonempty plain-text piece.
fn push_text<'a>(out: &mut Vec<Piece<'a>>, text: Option<&'a str>) {
    if let Some(text) = text.filter(|t| !t.is_empty()) {
        out.push(Piece::Text(text));
    }
}

/// Read `scope.Name` at the start of `text`; returns the scope, the name and how many bytes the
/// reference takes. `None` when the scope is unknown or the name does not start with a letter.
fn reference_at(text: &str) -> Option<(Scope, &str, usize)> {
    let dot = text.find('.')?;
    let scope = Scope::from_keyword(text.get(..dot)?)?;
    let rest = text.get(dot + 1..)?;
    let len = rest.char_indices().find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_')).map_or(rest.len(), |(i, _)| i);
    let name = rest.get(..len)?;
    name.chars().next().filter(char::is_ascii_alphabetic).map(|_| (scope, name, dot + 1 + len))
}

/// Whether a text holds at least one reference.
pub fn has_references(text: &str) -> bool {
    text.contains('@') && pieces(text).iter().any(|piece| matches!(piece, Piece::Reference { .. }))
}

/// A text with its references replaced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    /// The text to show.
    pub text: String,
    /// The references that named nothing, as typed, each once. They stay in the text as typed.
    pub unknown: Vec<String>,
}

/// Where a parameter came from, for listings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Read-only and derived from the project, such as `Name`.
    BuiltIn,
    /// A standard parameter that the list has not set, showing its default.
    Default,
    /// Stored in this list.
    Own,
}

impl Source {
    /// The listing spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
            Self::Default => "default",
            Self::Own => "own",
        }
    }
}

/// One row of a scope's parameters: its name, its unresolved value and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The parameter name.
    pub name: String,
    /// The value as stored or defaulted, before references are resolved.
    pub value: String,
    /// Where the value came from.
    pub source: Source,
}

/// Which list a `parameter.set` or `parameter.list` addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The project's own list.
    Project,
    /// One sheet's own list.
    Sheet(SheetTarget),
}

impl Target {
    /// The scope whose references read this list.
    pub const fn scope(self) -> Scope {
        match self {
            Self::Project => Scope::Project,
            Self::Sheet(_) => Scope::Sheet,
        }
    }

    /// Read `scope` and `sheet` command parameters. The sheet scope needs a sheet of this
    /// project, named by UUID or implied when the project has one sheet.
    fn parse(project: &Project, params: &Value) -> Result<Self, CommandError> {
        match params["scope"].as_str() {
            Some("project") => Ok(Self::Project),
            Some("sheet") => Ok(Self::Sheet(project.resolve_sheet(params["sheet"].as_str())?)),
            _ => Err(CommandError::InvalidParams("scope must be project or sheet".into())),
        }
    }
}

impl Project {
    /// The project's own stored parameters, without built-ins or defaults.
    pub fn parameters(&self) -> &Parameters {
        &self.parameters
    }

    /// One sheet's own stored parameters, without built-ins or defaults. `None` for a sheet that
    /// is not in this project.
    pub fn sheet_parameters(&self, sheet: SheetTarget) -> Option<&Parameters> {
        (sheet.project == self.id).then_some(())?;
        self.sheets.iter().find(|s| s.sheet_id == sheet.sheet).map(|s| &*s.parameters)
    }

    /// Sheets in Browser order: each root sheet in project order, followed depth first by the
    /// sheets placed on it. Page numbers count in this order.
    pub fn page_order(&self) -> Vec<SheetTarget> {
        let mut order = Vec::with_capacity(self.sheets.len());
        let visit = |root: SheetTarget, order: &mut Vec<SheetTarget>| {
            let mut stack = vec![root];
            while let Some(target) = stack.pop() {
                if order.contains(&target) {
                    continue;
                }
                order.push(target);
                let children: Vec<_> = self.sheets().filter(|s| self.parent_sheet(s.target) == Some(target)).map(|s| s.target).collect();
                stack.extend(children.into_iter().rev());
            }
        };
        for root in self.sheets().filter(|s| self.parent_sheet(s.target).is_none()).map(|s| s.target).collect::<Vec<_>>() {
            visit(root, &mut order);
        }
        // A sheet reachable from no root (a malformed cycle) still gets a page at the end.
        for info in self.sheets().map(|s| s.target).collect::<Vec<_>>() {
            visit(info, &mut order);
        }
        order
    }

    /// Every parameter `scope` offers on `sheet`, in listing order: built-ins, then standard
    /// parameters (own value or default), then the other own parameters by name. The project
    /// scope ignores `sheet`; the sheet and instance scopes offer nothing without one.
    pub fn parameter_rows(&self, scope: Scope, sheet: Option<SheetTarget>) -> Vec<Row> {
        let mut rows = Vec::new();
        let sheet = sheet.and_then(|target| self.sheets.iter().find(|s| target.project == self.id && s.sheet_id == target.sheet));
        let builtin = |name: &str, value: String| Row { name: name.to_owned(), value, source: Source::BuiltIn };
        let list = |own: &Parameters, standard: &[(&str, &str)], rows: &mut Vec<Row>| {
            for (name, default) in standard {
                rows.push(match own.get(*name) {
                    Some(value) => Row { name: (*name).to_owned(), value: value.clone(), source: Source::Own },
                    None => Row { name: (*name).to_owned(), value: (*default).to_owned(), source: Source::Default },
                });
            }
            for (name, value) in own.iter().filter(|(name, _)| !standard.iter().any(|(s, _)| s == name)) {
                rows.push(Row { name: name.clone(), value: value.clone(), source: Source::Own });
            }
        };
        match scope {
            Scope::Project => {
                rows.push(builtin("Name", self.name.clone()));
                list(&self.parameters, &PROJECT_STANDARD.map(|name| (name, "")), &mut rows);
            }
            Scope::Sheet => {
                if let Some(sheet) = sheet {
                    rows.push(builtin("Name", sheet.sheet_name.clone()));
                    list(&sheet.parameters, &SHEET_DEFAULTS, &mut rows);
                }
            }
            Scope::Instance => {
                if let Some(sheet) = sheet {
                    let order = self.page_order();
                    let page = order.iter().position(|t| t.sheet == sheet.sheet_id).map_or(0, |n| n + 1);
                    rows.push(builtin("Name", sheet.sheet_name.clone()));
                    rows.push(builtin("Page", page.to_string()));
                    rows.push(builtin("Pages", order.len().to_string()));
                }
            }
        }
        rows
    }

    /// The unresolved value of one parameter as `scope` offers it on `sheet`; `None` when it does
    /// not exist there. Names match exactly first, then ignoring case.
    pub fn parameter_value(&self, scope: Scope, sheet: Option<SheetTarget>, name: &str) -> Option<String> {
        let rows = self.parameter_rows(scope, sheet);
        rows.iter().find(|row| row.name == name).or_else(|| rows.iter().find(|row| row.name.eq_ignore_ascii_case(name))).map(|row| row.value.clone())
    }

    /// Replace every reference in `text` with its value as shown on `sheet` (`None` for a text
    /// that belongs to no sheet, which can name only `@project`). References that name nothing,
    /// or that nest deeper than a few levels, stay as typed and are listed in
    /// [`Resolved::unknown`].
    pub fn resolve_text(&self, sheet: Option<SheetTarget>, text: &str) -> Resolved {
        let mut out = Resolved::default();
        self.resolve_into(sheet, text, 0, &mut out);
        out
    }

    /// Append `text` to `out` with its references resolved `depth` levels deep.
    fn resolve_into(&self, sheet: Option<SheetTarget>, text: &str, depth: usize, out: &mut Resolved) {
        for piece in pieces(text) {
            match piece {
                Piece::Text(text) => out.text.push_str(text),
                Piece::Reference { scope, name, raw } => match self.parameter_value(scope, sheet, name).filter(|_| depth < MAX_DEPTH) {
                    Some(value) => self.resolve_into(sheet, &value, depth + 1, out),
                    None => {
                        out.text.push_str(raw);
                        if !out.unknown.iter().any(|u| u == raw) {
                            out.unknown.push(raw.to_owned());
                        }
                    }
                },
            }
        }
    }

    /// The stored list `target` names, for editing.
    fn parameters_mut(&mut self, target: Target) -> Result<&mut Parameters, CommandError> {
        match target {
            Target::Project => Ok(Arc::make_mut(&mut self.parameters)),
            Target::Sheet(sheet) => {
                let index = self.sheets().position(|s| s.target == sheet).ok_or_else(|| CommandError::InvalidParams("unknown sheet ID".into()))?;
                Ok(Arc::make_mut(&mut Arc::make_mut(&mut self.sheets)[index].parameters))
            }
        }
    }
}

/// The spelling a name takes in `target`'s list: a standard or already stored name that matches
/// ignoring case keeps its spelling, so `revision` edits `Revision` instead of adding a twin.
fn canonical(project: &Project, target: Target, name: &str) -> Result<String, CommandError> {
    if !valid_name(name) {
        return Err(CommandError::InvalidParams(format!(
            "`{name}` is not a parameter name: use 1 to {MAX_NAME_LEN} letters, digits and _, starting with a letter"
        )));
    }
    if target.scope().is_builtin(name) {
        return Err(CommandError::InvalidParams(format!("`{name}` is built in and read only")));
    }
    let standard: Vec<&str> = match target {
        Target::Project => PROJECT_STANDARD.to_vec(),
        Target::Sheet(_) => SHEET_DEFAULTS.map(|(name, _)| name).to_vec(),
    };
    let stored = match target {
        Target::Project => Some(project.parameters()),
        Target::Sheet(sheet) => project.sheet_parameters(sheet),
    };
    Ok(standard
        .into_iter()
        .map(str::to_owned)
        .chain(stored.into_iter().flat_map(|list| list.keys().cloned()))
        .find(|known| known.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| name.to_owned()))
}

/// Change one list: set `name` to `value`, remove it (`value` `None` without a rename) or rename
/// it, keeping its value unless a new one is given. Refuses renaming a missing parameter or onto
/// another one, and growing past [`MAX_PARAMETERS`]. Returns whether the list changed.
fn apply(list: &mut Parameters, name: &str, value: Option<String>, rename: Option<String>) -> Result<bool, CommandError> {
    let before = list.clone();
    match rename {
        Some(new) => {
            let old = list.remove(name).ok_or_else(|| CommandError::InvalidParams(format!("there is no parameter `{name}` to rename")))?;
            if new != name && list.contains_key(&new) {
                return Err(CommandError::InvalidParams(format!("there already is a parameter `{new}`")));
            }
            list.insert(new, value.unwrap_or(old));
        }
        None => match value {
            Some(value) => {
                if !list.contains_key(name) && list.len() >= MAX_PARAMETERS {
                    return Err(CommandError::Unavailable(format!("a list holds at most {MAX_PARAMETERS} parameters")));
                }
                list.insert(name.to_owned(), value);
            }
            None => {
                list.remove(name);
            }
        },
    }
    Ok(*list != before)
}

/// The listing shape shared by `parameter.list` and inspection.
fn rows_json(rows: &[Row]) -> Value {
    json!(rows.iter().map(|row| json!({"name":row.name,"value":row.value,"source":row.source.as_str()})).collect::<Vec<_>>())
}

impl Session {
    /// Set, rename or remove one parameter of the project or a sheet as one undo step. Setting a
    /// value equal to the current one changes nothing and records no step. Project parameters
    /// adopt the current storage layout first, like net classes, because only project/22 stores
    /// them.
    pub(crate) fn set_parameter(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let target = Target::parse(original, params)?;
        let name = canonical(original, target, params["name"].as_str().unwrap_or_default())?;
        let rename = params.get("rename").and_then(Value::as_str).map(|new| canonical(original, target, new)).transpose()?;
        let value = match params.get("value") {
            None if rename.is_none() => {
                return Err(CommandError::InvalidParams("give a value, null to remove the parameter, or a new name".into()));
            }
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_str().ok_or_else(|| CommandError::InvalidParams("value must be a string or null".into()))?.to_owned()),
        };
        if value.as_ref().is_some_and(|value| value.len() > MAX_VALUE_LEN) {
            return Err(CommandError::InvalidParams(format!("a parameter value is at most {MAX_VALUE_LEN} bytes")));
        }
        let mut candidate = original.clone();
        if !apply(candidate.parameters_mut(target)?, &name, value, rename)? {
            return Ok(());
        }
        if target == Target::Project && !candidate.parameters.is_empty() {
            if candidate.layout.is_none() {
                candidate = candidate.with_board_folders(".").map_err(CommandError::InvalidParams)?;
            }
            if !candidate.has_sheet_occurrences() {
                candidate = candidate.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
            }
        }
        candidate.sheet_index = 0;
        self.change(candidate)
    }

    /// Answer `parameter.list` and `parameter.resolve` without changing the session.
    pub(crate) fn parameter_query(&self, command: &str, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        if command == LIST {
            let target = Target::parse(project, params)?;
            let (own, sheet) = match target {
                Target::Project => (project.parameters(), None),
                Target::Sheet(sheet) => {
                    (project.sheet_parameters(sheet).ok_or_else(|| CommandError::InvalidParams("unknown sheet ID".into()))?, Some(sheet))
                }
            };
            let rows = project.parameter_rows(target.scope(), sheet);
            return Ok(json!({"revision":self.revision(),"parameters":own,"effective":rows_json(&rows)}));
        }
        let sheet = params.get("sheet").map(|sheet| project.resolve_sheet(sheet.as_str())).transpose()?;
        let text = params["text"].as_str().unwrap_or_default();
        let resolved = project.resolve_text(sheet, text);
        Ok(json!({"revision":self.revision(),"text":resolved.text,"unknown":resolved.unknown}))
    }
}

#[cfg(test)]
mod tests;
