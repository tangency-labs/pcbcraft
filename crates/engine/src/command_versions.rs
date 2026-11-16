//! Versions of the command contracts that discovery publishes.
//!
//! Every command, engine or window, has an integer `version` that starts at 1. It rises when the
//! command's parameters or result change meaning, so a client written against an older contract
//! can tell instead of silently getting a different result. Versions live in code, never in saved
//! projects. `docs/schemas/commands-v1.json` locks each command's version to a digest of its
//! described contract; [`check_lock`] refuses a changed contract whose version did not rise.
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
    /// A contract changed, a version fell, or versions rose without the catalog version: the
    /// command versions in code must change before the lock may.
    Version(Vec<String>),
    /// Versions are consistent but the lock is out of date; `section` is what it should read.
    Stale { problems: Vec<String>, section: String },
}

/// Compare `described` commands (as discovery returns them, each with `id` and `version`) with
/// the `section` of a lock, given the catalog version in code. The lock is not changed.
///
/// # Errors
///
/// [`LockMismatch::Version`] when a contract changed without a version bump, a version fell, or
/// a version rose while `catalog_version` stayed at the lock's. Otherwise
/// [`LockMismatch::Stale`] when a command is added, removed or re-versioned, or the lock's
/// catalog version differs, with the section text to commit.
pub fn check_lock(lock: &Value, section: &str, described: &[Value], catalog_version: u32) -> Result<(), LockMismatch> {
    let empty = Map::new();
    let locked = lock[section].as_object().unwrap_or(&empty);
    let locked_catalog = lock["catalog_version"].as_u64().unwrap_or(0);
    let mut version_problems = Vec::new();
    let mut stale = Vec::new();
    let mut raised = Vec::new();
    for command in described {
        let id = command["id"].as_str().unwrap_or_default();
        let version = command["version"].as_u64().unwrap_or(0);
        let Some(entry) = locked.get(id) else {
            stale.push(format!("`{id}` is not in the lock"));
            continue;
        };
        let locked_version = entry["version"].as_u64().unwrap_or(0);
        if version < locked_version {
            version_problems.push(format!("`{id}` is version {version}, below its locked version {locked_version}; versions never fall"));
        } else if version == locked_version && entry["contract"].as_str() != Some(contract_digest(command).as_str()) {
            version_problems.push(format!("the contract of `{id}` changed; bump the version of `{id}` above {locked_version}"));
        } else if version > locked_version {
            raised.push(id);
        }
    }
    if !raised.is_empty() && u64::from(catalog_version) <= locked_catalog {
        version_problems.push(format!("{} raised its version; raise CATALOG_VERSION above {locked_catalog}", raised.join(", ")));
    }
    if !version_problems.is_empty() {
        return Err(LockMismatch::Version(version_problems));
    }
    stale.extend(raised.iter().map(|id| format!("`{id}` has a new version")));
    for id in locked.keys().filter(|id| !described.iter().any(|command| command["id"] == id.as_str())) {
        stale.push(format!("`{id}` is in the lock but no longer described"));
    }
    if u64::from(catalog_version) != locked_catalog {
        stale.push(format!("the lock's catalog version is {locked_catalog}, not {catalog_version}"));
    }
    if stale.is_empty() { Ok(()) } else { Err(LockMismatch::Stale { problems: stale, section: lock_section(described) }) }
}

/// The lock section for `described` commands: one line per command, sorted by id, so a change
/// shows as a one-line diff.
pub fn lock_section(described: &[Value]) -> String {
    let entries: BTreeMap<&str, String> = described
        .iter()
        .map(|command| {
            let entry = json!({"version": command["version"], "contract": contract_digest(command)});
            (command["id"].as_str().unwrap_or_default(), entry.to_string())
        })
        .collect();
    let lines: Vec<String> = entries.iter().map(|(id, entry)| format!("    {}: {entry}", json!(id))).collect();
    format!("{{\n{}\n  }}", lines.join(",\n"))
}

#[cfg(test)]
mod tests;
