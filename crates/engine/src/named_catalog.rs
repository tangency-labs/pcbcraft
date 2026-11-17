//! Identity rules shared by the project's named catalogs (net classes and part groups). Each
//! kind keeps its own versioned record and vocabulary; only the revisioned UUID/name envelope is
//! common. `noun` is the kind's own word in diagnostics, `class` or `group`.
use std::collections::BTreeSet;
use uuid::Uuid;

/// The identity envelope of one catalog entry.
pub(crate) trait Entry {
    fn id(&self) -> Uuid;
    fn revision(&self) -> u32;
    fn name(&self) -> &str;
}

/// Write `noun` with a capital first letter for the start of a diagnostic.
fn capitalized(noun: &str) -> String {
    let mut chars = noun.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Names are presentation only, but blank, untrimmed and control-containing names are invalid.
pub(crate) fn validate_name(noun: &str, name: &str) -> Result<(), String> {
    if name.trim() != name || name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
        return Err(format!("{} name must contain 1–200 trimmed characters without control characters.", capitalized(noun)));
    }
    Ok(())
}

/// Validate a catalog's format, bound and identities. Duplicate IDs never overwrite records.
/// `label` names the record in the diagnostic, for example `net-class`.
pub(crate) fn validate<D: Entry>(format: &str, expected: &str, label: &str, noun: &str, entries: &[D]) -> Result<(), String> {
    if format != expected || entries.len() > crate::net_classes::MAX_CLASSES {
        return Err(format!("Unsupported {label} format or {noun} limit exceeded."));
    }
    let mut ids = BTreeSet::new();
    for entry in entries {
        if entry.id().is_nil() || !ids.insert(entry.id()) || entry.revision() == 0 {
            return Err(format!("{} IDs must be unique non-nil UUIDs with positive revisions.", capitalized(noun)));
        }
        validate_name(noun, entry.name())?;
    }
    Ok(())
}

/// Resolve an exact identity. Names and vector positions are never lookup authority.
pub(crate) fn get<D: Entry>(entries: &[D], id: Uuid) -> Option<&D> {
    entries.iter().find(|entry| entry.id() == id)
}
