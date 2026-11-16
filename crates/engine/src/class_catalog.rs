//! Identity rules shared by every kind of project class catalog. Each kind keeps its own
//! versioned record and vocabulary; only the revisioned UUID/name envelope is common.
use std::collections::BTreeSet;
use uuid::Uuid;

/// The identity envelope of one class definition.
pub(crate) trait Entry {
    fn id(&self) -> Uuid;
    fn revision(&self) -> u32;
    fn name(&self) -> &str;
}

/// Names are presentation only, but blank, untrimmed and control-containing names are invalid.
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.trim() != name || name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
        return Err("Class name must contain 1–200 trimmed characters without control characters.".into());
    }
    Ok(())
}

/// Validate a catalog's format, bound and identities. Duplicate IDs never overwrite records.
/// `label` names the kind in the diagnostic, for example `net-class`.
pub(crate) fn validate<D: Entry>(format: &str, expected: &str, label: &str, definitions: &[D]) -> Result<(), String> {
    if format != expected || definitions.len() > crate::net_classes::MAX_CLASSES {
        return Err(format!("Unsupported {label} format or class limit exceeded."));
    }
    let mut ids = BTreeSet::new();
    for class in definitions {
        if class.id().is_nil() || !ids.insert(class.id()) || class.revision() == 0 {
            return Err("Class IDs must be unique non-nil UUIDs with positive revisions.".into());
        }
        validate_name(class.name())?;
    }
    Ok(())
}

/// Resolve an exact class identity. Names and vector positions are never lookup authority.
pub(crate) fn get<D: Entry>(definitions: &[D], id: Uuid) -> Option<&D> {
    definitions.iter().find(|class| class.id() == id)
}
