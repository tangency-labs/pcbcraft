//! Versions of the command contracts that discovery publishes.
//!
//! Every command, engine or window, has an integer `version` that starts at 1. It rises when the
//! command's parameters or result change meaning, so a client written against an older contract
//! can tell instead of silently getting a different result. Versions live in code, never in saved
//! projects. `docs/schemas/commands-v1.json` locks each command's version to a digest of its
//! described contract and keeps removed commands with their last version; [`check_lock`]
//! refuses a changed contract whose version did not rise and any version that falls.
//! Titles, summaries and descriptions are prose for people and may change freely.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Rises whenever any command's version rises, so a client can tell from one number that some
/// contract it relied on may have changed.
pub const CATALOG_VERSION: u32 = 1;

/// The optional protocol features this build offers, so a client can check before relying on
/// one. Features are only ever appended.
pub const CAPABILITIES: &[&str] = &["command_versions"];

/// Commands whose contract changed since they were first published. An unlisted command is
/// version 1; an entry is only ever raised, never removed.
const VERSIONS: &[(&str, u32)] = &[];

/// The current contract version of the command with this id, engine or window; 1 unless it changed.
pub fn version(id: &str) -> u32 {
    VERSIONS.iter().find(|(listed, _)| *listed == id).map_or(1, |(_, version)| *version)
}

/// The digest of a described command's contract: hex SHA-256 of its JSON with every object's
/// keys sorted and without its `version` or the prose (`title`, `summary`, `description`) of the
/// command and its parameter schemas. Data values in a schema (`const`, `default`, `enum`,
/// `examples`) are kept whole. It changes exactly when the id, scope or parameter schema does.
pub fn contract_digest(description: &Value) -> String {
    let mut unversioned = description.clone();
    if let Some(object) = unversioned.as_object_mut() {
        object.remove("version");
    }
    let digest = Sha256::digest(contract(&unversioned, Level::Schema).to_string().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Schema keywords whose object keys are parameter or definition names, not keywords.
const NAMED_MAPS: &[&str] = &["properties", "patternProperties", "$defs", "definitions"];

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

/// Drop prose and rebuild objects with sorted keys whatever map order
/// `serde_json` was built with. Only schema objects lose their prose: a parameter may itself
/// be called `title` or `description`, and a default value may be an object with such a key.
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

/// Why described commands do not match their lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockMismatch {
    /// A contract changed, a version or the catalog version fell, or versions rose without the
    /// catalog version: the versions in code must change before the lock may.
    Version(Vec<String>),
    /// Versions are consistent but the lock is out of date; `section` is what it should read.
    Stale { problems: Vec<String>, section: String },
}

/// Compare `described` commands (as discovery returns them, each with `id` and `version`) with
/// the `section` of a lock, given the catalog version in code. The lock is not changed.
///
/// Each described id is compared with its highest locked version over every section, removed
/// or not: below it is refused, the same version must keep its contract, and above it counts as
/// raised. A removed command stays in the lock marked `"removed": true` with its last version,
/// so an id that comes back, in this section or another one, never returns below it.
///
/// # Errors
///
/// [`LockMismatch::Version`] when a version in the lock is not a whole number of at least 1, a
/// contract changed without a version bump, a version fell
/// (also for a removed id that comes back, or an id that moves from another section),
/// `catalog_version` fell below the lock's, or a version
/// rose while `catalog_version` stayed at the lock's. Otherwise [`LockMismatch::Stale`] when a
/// command is added, removed, re-added, moved or re-versioned, or the lock's catalog version differs,
/// with the section text to commit.
pub fn check_lock(lock: &Value, section: &str, described: &[Value], catalog_version: u32) -> Result<(), LockMismatch> {
    let empty = Map::new();
    let locked = lock[section].as_object().unwrap_or(&empty);
    let unreadable = unreadable_versions(lock);
    if !unreadable.is_empty() {
        return Err(LockMismatch::Version(unreadable));
    }
    let locked_catalog = lock["catalog_version"].as_u64().unwrap_or(0);
    let mut version_problems = Vec::new();
    let mut stale = Vec::new();
    let mut raised = Vec::new();
    for command in described {
        let id = command["id"].as_str().unwrap_or_default();
        let version = command["version"].as_u64().unwrap_or(0);
        let Some((from, prior)) = prior_entry(lock, section, id) else {
            stale.push(format!("`{id}` is not in the lock"));
            continue;
        };
        let prior_version = prior["version"].as_u64().unwrap_or(0);
        let here = from == section;
        if version < prior_version {
            let was = if !here {
                format!("its version {prior_version} in `{from}`")
            } else if is_removed(prior) {
                format!("its version when it was removed {prior_version}")
            } else {
                format!("its locked version {prior_version}")
            };
            version_problems.push(format!("`{id}` is version {version}, below {was}; versions never fall"));
        } else if version > prior_version {
            raised.push(id);
        } else if prior["contract"].as_str() != Some(contract_digest(command).as_str()) {
            version_problems.push(format!("the contract of `{id}` changed; bump the version of `{id}` above {prior_version}"));
        } else if !here {
            stale.push(if locked.contains_key(id) { format!("`{id}` moved from `{from}`") } else { format!("`{id}` is not in the lock") });
        } else if is_removed(prior) {
            stale.push(format!("`{id}` was removed and is described again"));
        }
    }
    if u64::from(catalog_version) < locked_catalog {
        version_problems.push(format!("CATALOG_VERSION is {catalog_version}, below the lock's {locked_catalog}; the catalog version never falls"));
    } else if !raised.is_empty() && u64::from(catalog_version) == locked_catalog {
        version_problems.push(format!("{} raised its version; raise CATALOG_VERSION above {locked_catalog}", raised.join(", ")));
    }
    if !version_problems.is_empty() {
        return Err(LockMismatch::Version(version_problems));
    }
    stale.extend(raised.iter().map(|id| format!("`{id}` has a new version")));
    for (id, _) in locked.iter().filter(|(id, entry)| !is_removed(entry) && !described.iter().any(|command| command["id"] == id.as_str())) {
        stale.push(format!("`{id}` is no longer described; keep it in the lock as removed"));
    }
    if u64::from(catalog_version) != locked_catalog {
        stale.push(format!("the lock's catalog version is {locked_catalog}, not {catalog_version}"));
    }
    if stale.is_empty() { Ok(()) } else { Err(LockMismatch::Stale { problems: stale, section: lock_section(described, locked) }) }
}

/// Every version in the lock that is not a whole number of at least 1: the catalog version and
/// each entry of every section. Such a value is refused, never read as 0, so a lock edited by hand
/// (for example `3.0`, `"3"` or a missing field) cannot switch off a rule.
fn unreadable_versions(lock: &Value) -> Vec<String> {
    let shown = |value: &Value| if value.is_null() { "missing".to_owned() } else { value.to_string() };
    let readable = |value: &Value| value.as_u64().is_some_and(|version| version >= 1);
    let mut problems = Vec::new();
    if !readable(&lock["catalog_version"]) {
        problems.push(format!("the lock's catalog_version is {}, not a whole number of at least 1", shown(&lock["catalog_version"])));
    }
    for (name, entries) in lock.as_object().into_iter().flatten() {
        for (id, entry) in entries.as_object().into_iter().flatten() {
            if !readable(&entry["version"]) {
                problems.push(format!("`{id}` in `{name}` has version {}, not a whole number of at least 1", shown(&entry["version"])));
            }
        }
    }
    problems
}

/// The lock entry that `id` is checked against, with its section's name: the highest version
/// the lock records for `id` in any section, removed or not, preferring `section` on a tie.
/// Clients see one list of ids, so a command's version is one number whichever section
/// describes it.
fn prior_entry<'a>(lock: &'a Value, section: &str, id: &str) -> Option<(&'a str, &'a Value)> {
    lock.as_object()?
        .iter()
        .filter_map(|(name, entries)| Some((name.as_str(), entries.as_object()?.get(id)?)))
        .max_by_key(|(name, entry)| (entry["version"].as_u64().unwrap_or(0), *name == section))
}

/// Whether a lock entry records a command that is no longer described.
fn is_removed(entry: &Value) -> bool {
    entry["removed"] == Value::Bool(true)
}

/// The lock section for `described` commands, keeping every command of the `locked` section
/// that is no longer described as removed with its last version and contract. One line per
/// command, sorted by id, so a change shows as a one-line diff.
pub fn lock_section(described: &[Value], locked: &Map<String, Value>) -> String {
    let mut entries: BTreeMap<&str, String> = locked
        .iter()
        .map(|(id, entry)| {
            let removed = json!({"version": entry["version"], "contract": entry["contract"], "removed": true});
            (id.as_str(), removed.to_string())
        })
        .collect();
    for command in described {
        let entry = json!({"version": command["version"], "contract": contract_digest(command)});
        entries.insert(command["id"].as_str().unwrap_or_default(), entry.to_string());
    }
    let lines: Vec<String> = entries.iter().map(|(id, entry)| format!("    {}: {entry}", json!(id))).collect();
    format!("{{\n{}\n  }}", lines.join(",\n"))
}

#[cfg(test)]
mod tests;
