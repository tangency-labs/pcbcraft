//! The public command API: its compatibility version, the catalog revision and the committed
//! contract snapshot.
//!
//! Three things, kept apart. [`COMMAND_API_VERSION`] is one number for the whole command
//! contract; it rises only when a published change breaks existing callers. [`catalog_revision`]
//! fingerprints the described commands, so a client can drop cached discovery data; a new
//! revision alone does not mean callers are incompatible. Schemas and [`CAPABILITIES`] say what
//! the running build supports. None of them is the MCP protocol version, the build's version or a
//! native file-format number, and none is saved in projects.
//!
//! `docs/schemas/command-contracts.json` snapshots every command's contract. [`check_snapshot`]
//! compares the described commands with it and labels each difference compatible or breaking
//! ([`compare`]); a breaking one fails until `COMMAND_API_VERSION` is raised.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// The command API compatibility version. Raise it only by an explicit decision that a published
/// change breaks existing callers; it never falls.
pub const COMMAND_API_VERSION: u32 = 1;

/// The optional protocol features this build offers, so a client can check before relying on
/// one. Features are only ever appended.
pub const CAPABILITIES: &[&str] = &["command_api"];

/// A fingerprint of `described` commands as discovery returns them: hex SHA-256 of the list sorted
/// by id, every object's keys sorted, prose included. Any change a client could have cached
/// changes it: a command added or removed, a parameter changed, a title reworded. The live editor
/// and a headless session describe different commands, and builds differ, so each reports its own.
pub fn catalog_revision(described: &[Value]) -> String {
    let mut sorted: Vec<Value> = described.iter().map(sorted_keys).collect();
    sorted.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()).then_with(|| a.to_string().cmp(&b.to_string())));
    hex(&Value::Array(sorted).to_string())
}

/// Hex SHA-256 of `text`.
fn hex(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `value` rebuilt with every object's keys sorted, whatever map order `serde_json` was built with.
fn sorted_keys(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted: BTreeMap<&String, Value> = object.iter().map(|(key, value)| (key, sorted_keys(value))).collect();
            Value::Object(sorted.into_iter().map(|(key, value)| (key.clone(), value)).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted_keys).collect()),
        other => other.clone(),
    }
}

/// A described command's contract as the snapshot stores it: the description without `id` and
/// without the prose (`title`, `summary`, `description`) of the command and its schemas, keys
/// sorted. Data values (`const`, `default`, `enum`, `examples`) are kept whole, and parameter
/// names are kept even when a parameter is called `title`.
pub fn contract_of(description: &Value) -> Value {
    let mut contract = contract(description, Level::Schema);
    if let Some(object) = contract.as_object_mut() {
        object.remove("id");
    }
    contract
}

/// Schema keywords whose object keys are parameter or definition names, not keywords. Under
/// `dependentRequired`, `dependentSchemas` and `dependencies` every key names a parameter, so no
/// key there is prose.
const NAMED_MAPS: &[&str] = &["properties", "patternProperties", "$defs", "definitions", "dependentRequired", "dependentSchemas", "dependencies"];

/// Schema keywords whose values are data (allowed, default or sample values), not schemas.
const DATA_VALUES: &[&str] = &["const", "default", "enum", "examples"];

/// What a JSON value is within a described command.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    /// The command or a schema: its prose keys are dropped.
    Schema,
    /// A map from parameter or definition names to schemas: every key is kept.
    Names,
    /// A data value inside a schema: kept whole, even a key called `title`.
    Data,
}

/// Drop prose and rebuild objects with sorted keys. Only schema objects lose their prose: a
/// parameter may itself be called `title` or `description`, and a default value may be an object
/// with such a key.
fn contract(value: &Value, level: Level) -> Value {
    match value {
        Value::Object(object) => {
            let sorted: BTreeMap<&String, Value> = object
                .iter()
                .filter(|(key, _)| level != Level::Schema || !matches!(key.as_str(), "title" | "summary" | "description"))
                .map(|(key, value)| {
                    let inner = match level {
                        Level::Schema if NAMED_MAPS.contains(&key.as_str()) => Level::Names,
                        Level::Schema if DATA_VALUES.contains(&key.as_str()) => Level::Data,
                        Level::Schema | Level::Names => Level::Schema,
                        Level::Data => Level::Data,
                    };
                    (key, contract(value, inner))
                })
                .collect();
            Value::Object(sorted.into_iter().map(|(key, value)| (key.clone(), value)).collect::<Map<String, Value>>())
        }
        Value::Array(items) => {
            let inner = if level == Level::Data { Level::Data } else { Level::Schema };
            Value::Array(items.iter().map(|item| contract(item, inner)).collect())
        }
        other => other.clone(),
    }
}

/// One difference between a snapshot contract and the described one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Whether a caller valid under the old contract may fail or get a different result.
    pub breaking: bool,
    /// The command, the path in its contract and what changed, for example
    /// ``"`edit.undo` params.required: now requires `note`"``.
    pub what: String,
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", if self.breaking { "breaking" } else { "compatible" }, self.what)
    }
}

/// Lower bounds: a lower or dropped value accepts more.
const LOWER_BOUNDS: &[&str] = &["minimum", "exclusiveMinimum", "minLength", "minItems", "minProperties"];
/// Upper bounds: a higher or dropped value accepts more.
const UPPER_BOUNDS: &[&str] = &["maximum", "exclusiveMaximum", "maxLength", "maxItems", "maxProperties"];
/// Keywords compared only as a whole: any change under one is breaking, because whether it accepts
/// more or less depends on the schema around it (`not` inverts, `dependentRequired` and the other
/// dependency maps tie parameters together, `$ref` points elsewhere).
const WHOLE: &[&str] = &["not", "allOf", "$ref", "if", "then", "else", "patternProperties", "dependentRequired", "dependentSchemas", "dependencies"];
/// Restrictions: dropping one accepts more; adding or changing one may refuse a valid call.
const RESTRICTIONS: &[&str] = &["pattern", "format", "const", "multipleOf"];

/// Every difference between the `old` and `new` contract of the command `id`, labelled.
///
/// A difference is compatible when every call valid under `old` stays valid with the same meaning
/// as far as the schema can tell; otherwise it is breaking. By keyword, at every depth:
/// - `properties`: an added parameter is compatible only when the old object refused unknown
///   names (`additionalProperties: false` and no `patternProperties`), since otherwise a call
///   could already pass that name with any value; whether it is required is `required`'s
///   business. A removed parameter is breaking; common ones are compared in turn. In `$defs` and
///   `definitions` an added name is compatible, a removed one breaking.
/// - `default`: added, changed or dropped is breaking, since it may change what an omitted
///   parameter means (a new parameter's default is part of the parameter added).
/// - `required`: a newly required name is breaking; a name no longer required is compatible.
/// - `type`: compatible when every old type is still allowed (`integer` within `number`) or the
///   keyword is dropped; narrowed or added is breaking.
/// - `enum`: members added or the keyword dropped is compatible; a member removed or the keyword
///   added is breaking.
/// - lower and upper bounds (`minimum` .. `maxProperties`): loosened or dropped is compatible,
///   tightened or added is breaking.
/// - `uniqueItems`: dropped or set false is compatible, otherwise breaking.
/// - `pattern`, `format`, `const`, `multipleOf`: dropped is compatible; added or changed is
///   breaking.
/// - `items`, `additionalProperties`: a schema or `true`/`false`; loosened (from `false`, or to
///   accept anything) is compatible, tightened breaking, schemas compared in turn.
/// - `anyOf`: alternatives appended is compatible, removed breaking; common ones are compared.
/// - `oneOf`: any change is breaking, because a value that matched one alternative may then
///   match two, and `oneOf` refuses it; common alternatives are still compared for the path.
/// - `prefixItems`: compared item by item; a changed length is breaking.
/// - `examples`: an annotation, so compatible.
/// - `not`, `allOf`, `$ref`, `if`/`then`/`else`, `patternProperties`, `dependentRequired`,
///   `dependentSchemas`, `dependencies`: compared as a whole; any change is breaking.
/// - any other keyword: any change is breaking, because the classifier cannot tell.
///
/// Outside the parameter schema (for example `scope`), any change is breaking. A command only in
/// `new` is compatible; a command only in `old` is breaking (see [`check_snapshot`]).
pub fn compare(id: &str, old: &Value, new: &Value) -> Vec<Change> {
    let mut changes = Vec::new();
    let empty = Map::new();
    let (old_object, new_object) = (old.as_object().unwrap_or(&empty), new.as_object().unwrap_or(&empty));
    for key in old_object.keys().chain(new_object.keys()).collect::<BTreeSet<_>>() {
        let (before, after) = (old_object.get(key), new_object.get(key));
        if before == after {
            continue;
        }
        if key == "params" {
            schema(&mut Walk { id, out: &mut changes }, key, before.unwrap_or(&Value::Bool(true)), after.unwrap_or(&Value::Bool(true)));
        } else {
            Walk { id, out: &mut changes }.push(true, key, format!("changed from {} to {}", shown(before), shown(after)));
        }
    }
    changes
}

/// Where [`compare`] writes its findings for one command.
struct Walk<'a> {
    id: &'a str,
    out: &'a mut Vec<Change>,
}

impl Walk<'_> {
    fn push(&mut self, breaking: bool, path: &str, what: String) {
        self.out.push(Change { breaking, what: format!("`{}` {path}: {what}", self.id) });
    }
}

/// A JSON value for a message; a missing one reads `absent`.
fn shown(value: Option<&Value>) -> String {
    value.map_or_else(|| "absent".to_owned(), Value::to_string)
}

/// Whether a schema position accepts every value: absent, `true` or `{}`.
fn accepts_all(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Bool(true)) => true,
        Some(Value::Object(object)) => object.is_empty(),
        _ => false,
    }
}

/// Compare two schemas at `path`. Booleans and non-objects are compared as a whole: loosening to
/// accept everything is compatible, anything else that differs is breaking.
fn schema(walk: &mut Walk, path: &str, old: &Value, new: &Value) {
    if old == new {
        return;
    }
    let (Some(old_object), Some(new_object)) = (old.as_object(), new.as_object()) else {
        let loosened = old == &Value::Bool(false) || accepts_all(Some(new));
        walk.push(!loosened, path, format!("changed from {old} to {new}"));
        return;
    };
    let closed = old_object.get("additionalProperties") == Some(&Value::Bool(false)) && !old_object.contains_key("patternProperties");
    for key in old_object.keys().chain(new_object.keys()).collect::<BTreeSet<_>>() {
        let (before, after) = (old_object.get(key), new_object.get(key));
        if before != after {
            keyword(walk, &format!("{path}.{key}"), key, before, after, closed);
        }
    }
}

/// Classify one keyword that differs between two schemas; see [`compare`] for the rules.
/// `closed` says whether the old schema refused names it does not list.
fn keyword(walk: &mut Walk, path: &str, key: &str, before: Option<&Value>, after: Option<&Value>, closed: bool) {
    let change = format!("changed from {} to {}", shown(before), shown(after));
    match key {
        "properties" => named(walk, path, before, after, closed),
        "$defs" | "definitions" => named(walk, path, before, after, true),
        "required" => required(walk, path, before, after),
        "type" => {
            let compatible = match (types(before), types(after)) {
                (Some(old), Some(new)) => old.iter().all(|t| new.contains(t) || (*t == "integer" && new.contains("number"))),
                (Some(_), None) => after.is_none(),
                _ => false,
            };
            walk.push(!compatible, path, change);
        }
        "enum" => {
            let compatible = match (before.and_then(Value::as_array), after.and_then(Value::as_array)) {
                (Some(old), Some(new)) => old.iter().all(|member| new.contains(member)),
                _ => before.is_some() && after.is_none(),
            };
            walk.push(!compatible, path, change);
        }
        bound if LOWER_BOUNDS.contains(&bound) || UPPER_BOUNDS.contains(&bound) => {
            let compatible = match (before.and_then(Value::as_f64), after.and_then(Value::as_f64)) {
                (Some(old), Some(new)) => {
                    if LOWER_BOUNDS.contains(&bound) {
                        new <= old
                    } else {
                        new >= old
                    }
                }
                (Some(_), None) => after.is_none(),
                _ => false,
            };
            walk.push(!compatible, path, change);
        }
        "uniqueItems" => walk.push(!matches!(after, None | Some(Value::Bool(false))), path, change),
        restriction if RESTRICTIONS.contains(&restriction) => walk.push(after.is_some(), path, change),
        "default" => walk.push(true, path, change),
        "examples" => walk.push(false, path, change),
        "items" | "additionalProperties" => match (before, after) {
            (Some(old @ Value::Object(_)), Some(new @ Value::Object(_))) => schema(walk, path, old, new),
            _ => {
                let loosened = before == Some(&Value::Bool(false)) || accepts_all(after);
                walk.push(!loosened, path, change);
            }
        },
        "anyOf" | "oneOf" | "prefixItems" => alternatives(walk, path, key, before, after),
        whole if WHOLE.contains(&whole) => walk.push(true, path, change),
        _ => walk.push(true, path, change),
    }
}

/// The types a `type` keyword allows, or `None` when it is not a string or a list of strings.
fn types(value: Option<&Value>) -> Option<BTreeSet<&str>> {
    match value? {
        Value::String(name) => Some(BTreeSet::from([name.as_str()])),
        Value::Array(names) => names.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

/// Compare two maps of named schemas (`properties`, `$defs`, `definitions`): a name added is
/// compatible when `added_is_compatible` (else the old schema already accepted that name with any
/// value, so it is breaking), a name removed breaking, common names compared as schemas.
fn named(walk: &mut Walk, path: &str, before: Option<&Value>, after: Option<&Value>, added_is_compatible: bool) {
    let empty = Map::new();
    let old = before.and_then(Value::as_object).unwrap_or(&empty);
    let new = after.and_then(Value::as_object).unwrap_or(&empty);
    for name in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        let inner = format!("{path}.{name}");
        match (old.get(name), new.get(name)) {
            (Some(a), Some(b)) => schema(walk, &inner, a, b),
            (None, Some(_)) if added_is_compatible => walk.push(false, &inner, "added".to_owned()),
            (None, Some(_)) => walk.push(true, &inner, "added where the old schema accepted any name".to_owned()),
            (Some(_), None) => walk.push(true, &inner, "removed".to_owned()),
            (None, None) => {}
        }
    }
}

/// Compare two `required` lists: each name newly required is breaking, each name no longer
/// required compatible (unless the parameter itself was removed, which `properties` reports).
fn required(walk: &mut Walk, path: &str, before: Option<&Value>, after: Option<&Value>) {
    let names = |value: Option<&Value>| -> BTreeSet<String> {
        value.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
    };
    let (old, new) = (names(before), names(after));
    for name in new.difference(&old) {
        walk.push(true, path, format!("now requires `{name}`"));
    }
    for name in old.difference(&new) {
        walk.push(false, path, format!("no longer requires `{name}`"));
    }
}

/// Compare two lists of schemas under `anyOf`, `oneOf` or `prefixItems`. Common positions are
/// compared as schemas. Under `anyOf` appended alternatives are compatible; under `oneOf` every
/// difference is breaking, also a compatible one inside an alternative, since a value may then
/// match two alternatives; under `prefixItems` a changed length is breaking. A list added or
/// dropped is breaking.
fn alternatives(walk: &mut Walk, path: &str, key: &str, before: Option<&Value>, after: Option<&Value>) {
    let (Some(old), Some(new)) = (before.and_then(Value::as_array), after.and_then(Value::as_array)) else {
        walk.push(true, path, format!("changed from {} to {}", shown(before), shown(after)));
        return;
    };
    let mut inner = Vec::new();
    let mut nested = Walk { id: walk.id, out: &mut inner };
    for (index, (a, b)) in old.iter().zip(new).enumerate() {
        schema(&mut nested, &format!("{path}[{index}]"), a, b);
    }
    if new.len() > old.len() {
        nested.push(key != "anyOf", path, format!("{} alternative(s) appended", new.len() - old.len()));
    } else if new.len() < old.len() {
        nested.push(true, path, format!("{} alternative(s) removed", old.len() - new.len()));
    }
    for mut change in inner {
        if key == "oneOf" && !change.breaking {
            change.breaking = true;
            change.what.push_str(" (inside oneOf, a value may now match two alternatives)");
        }
        walk.out.push(change);
    }
}

/// Why described commands do not match the committed snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotMismatch {
    /// A breaking difference while `COMMAND_API_VERSION` stayed at the snapshot's, or an API
    /// version below the snapshot's or not a whole number: the code must change, not the snapshot.
    Breaking(Vec<String>),
    /// Nothing breaks unannounced, but the snapshot is out of date; `section` is what the section
    /// should read and `api_version` what `command_api_version` should read.
    Stale { changes: Vec<String>, section: String, api_version: u32 },
}

/// Compare `described` commands (as discovery returns them) with the `section` of a committed
/// `snapshot`, given the API version in code. The snapshot is not changed.
///
/// # Errors
///
/// [`SnapshotMismatch::Breaking`] when the snapshot's `command_api_version` is not a whole number
/// of at least 1, when `api_version` is below it, or when [`compare`] finds a breaking difference
/// (a command removed included) and `api_version` equals it; the list names every breaking
/// difference. Otherwise [`SnapshotMismatch::Stale`] when any difference remains (compatible ones,
/// or breaking ones after a raised `api_version`), with the section text to commit.
pub fn check_snapshot(snapshot: &Value, section: &str, described: &[Value], api_version: u32) -> Result<(), SnapshotMismatch> {
    let Some(locked_api) = snapshot["command_api_version"].as_u64().filter(|version| *version >= 1) else {
        let found = shown(snapshot.get("command_api_version"));
        return Err(SnapshotMismatch::Breaking(vec![format!("the snapshot's command_api_version is {found}, not a whole number of at least 1")]));
    };
    if u64::from(api_version) < locked_api {
        return Err(SnapshotMismatch::Breaking(vec![format!(
            "COMMAND_API_VERSION is {api_version}, below the snapshot's {locked_api}; it never falls"
        )]));
    }
    let empty = Map::new();
    let locked = snapshot[section].as_object().unwrap_or(&empty);
    let mut changes = Vec::new();
    for command in described {
        let id = command["id"].as_str().unwrap_or_default();
        match locked.get(id) {
            Some(old) => changes.extend(compare(id, old, &contract_of(command))),
            None => changes.push(Change { breaking: false, what: format!("`{id}`: added") }),
        }
    }
    for id in locked.keys().filter(|id| !described.iter().any(|command| command["id"] == id.as_str())) {
        changes.push(Change { breaking: true, what: format!("`{id}`: removed") });
    }
    let raised = u64::from(api_version) > locked_api;
    if !raised && changes.iter().any(|change| change.breaking) {
        let mut problems: Vec<String> = changes.iter().filter(|change| change.breaking).map(ToString::to_string).collect();
        problems.push(format!("raise COMMAND_API_VERSION above {locked_api} if this break is intended, or keep the old contract"));
        return Err(SnapshotMismatch::Breaking(problems));
    }
    if changes.is_empty() && !raised {
        return Ok(());
    }
    let mut changes: Vec<String> = changes.iter().map(ToString::to_string).collect();
    if raised {
        changes.push(format!("the snapshot's command_api_version is {locked_api}, not {api_version}"));
    }
    Err(SnapshotMismatch::Stale { changes, section: snapshot_section(described), api_version })
}

/// The snapshot section for `described` commands: one line per command, sorted by id, mapping
/// the id to its [`contract_of`], so a change shows as a one-line diff.
pub fn snapshot_section(described: &[Value]) -> String {
    let entries: BTreeMap<&str, String> =
        described.iter().map(|command| (command["id"].as_str().unwrap_or_default(), contract_of(command).to_string())).collect();
    if entries.is_empty() {
        return "{}".to_owned();
    }
    let lines: Vec<String> = entries.iter().map(|(id, contract)| format!("    {}: {contract}", Value::String((*id).to_owned()))).collect();
    format!("{{\n{}\n  }}", lines.join(",\n"))
}

#[cfg(test)]
mod tests;
