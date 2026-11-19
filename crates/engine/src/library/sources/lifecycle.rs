//! Publisher assertions and project-owned lifecycle policy. Neither is definition content.

use super::{
    super::{check_id, check_text},
    Key, Reference,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

mod adoption;
pub(crate) use adoption::COMMANDS;
pub use adoption::{ADOPT, Assessments, INSPECT, LOOKUP};

/// Native source document format for an immutable publisher assertion revision.
pub const FORMAT: &str = "pcbcraft-lifecycle-assertion/1";
/// Flat source directory for assertions, separate from item definitions.
pub(super) const DIRECTORY: &str = "lifecycle-assertions";
const MAX_STATES: usize = 64;
const MAX_LABEL_BYTES: usize = 256;
const MAX_NOTE_BYTES: usize = 4096;

/// Project interpretation of a state; scheme IDs and display labels do not imply a disposition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Draft,
    Released,
    Deprecated,
    Obsolete,
}

/// A stable scheme state, with a presentation label independent of its policy meaning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub id: String,
    pub label: String,
    pub disposition: Disposition,
}

/// An exact scheme revision and its project-authored state mapping in presentation order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub scheme: Reference,
    pub states: Vec<State>,
}

impl Policy {
    /// The default mapping is applied only to this reserved exact scheme revision.
    pub fn standard() -> Self {
        use Disposition::{Deprecated, Draft, Obsolete, Released};
        Self {
            scheme: Reference { library: "pcbcraft".into(), id: "lifecycle".into(), revision: 1 },
            states: [
                ("draft", "Draft", Draft),
                ("released", "Released", Released),
                ("deprecated", "Deprecated", Deprecated),
                ("obsolete", "Obsolete", Obsolete),
            ]
            .into_iter()
            .map(|(id, label, disposition)| State { id: id.into(), label: label.into(), disposition })
            .collect(),
        }
    }

    /// Require one to 64 distinct stable IDs and bounded single-line labels.
    pub fn validate(&self) -> Result<(), String> {
        self.scheme.validate()?;
        if !(1..=MAX_STATES).contains(&self.states.len()) {
            return Err(format!("A lifecycle policy must contain 1 to {MAX_STATES} states"));
        }
        let mut ids = BTreeSet::new();
        for state in &self.states {
            check_state_id(&state.id)?;
            check_line("lifecycle state label", &state.label, MAX_LABEL_BYTES)?;
            if !ids.insert(state.id.as_str()) {
                return Err(format!("Duplicate lifecycle state ID {:?}", state.id));
            }
        }
        Ok(())
    }

    /// Resolve a stable ID, leaving unknown states unclassified.
    pub fn disposition(&self, id: &str) -> Option<Disposition> {
        self.states.iter().find(|state| state.id == id).map(|state| state.disposition)
    }
}

/// Source attribution retained with each assertion. It is not a source registration or path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub publisher: String,
}

/// An immutable assertion and its exact target. A state change requires a new assertion revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    pub format: String,
    pub address: Reference,
    pub target: Key,
    pub scheme: Reference,
    pub state: String,
    pub reason: String,
    pub provenance: Provenance,
}

impl Assertion {
    /// Decode a single supported publisher document; refuse unknown fields and invalid addresses.
    pub fn read(text: &str) -> Result<Self, String> {
        if text.len() > super::MAX_FILE_BYTES {
            return Err("Lifecycle assertion exceeds the native document limit".into());
        }
        let assertion: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        assertion.validate()?;
        Ok(assertion)
    }

    /// Encode a validated assertion; no source I/O or publication is performed.
    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| error.to_string())
    }

    /// Fingerprint canonical assertion content, not the definition or its source path.
    pub fn digest(&self) -> Result<String, String> {
        Ok(format!("{:x}", Sha256::digest(self.encode()?.as_bytes())))
    }

    /// Validate supported version, exact revision identities, ownership, reason and attribution.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err("Unsupported lifecycle assertion format".into());
        }
        self.address.validate()?;
        check_id(&self.target.library)?;
        check_id(&self.target.id)?;
        if self.target.revision == 0 || self.target.library != self.address.library {
            return Err("Lifecycle assertion target must be an exact revision in the publishing library".into());
        }
        self.scheme.validate()?;
        check_state_id(&self.state)?;
        check_line("lifecycle reason", &self.reason, MAX_NOTE_BYTES)?;
        check_line("lifecycle publisher", &self.provenance.publisher, MAX_LABEL_BYTES)
    }
}

/// Stable IDs are lowercase ASCII slugs; labels may be localized without changing identity.
fn check_state_id(id: &str) -> Result<(), String> {
    if !(1..=MAX_LABEL_BYTES).contains(&id.len())
        || !id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || id.starts_with('-')
        || id.ends_with('-')
    {
        return Err("A lifecycle state ID must be 1 to 256 lowercase letters, digits and inner hyphens".into());
    }
    Ok(())
}

/// Apply both native one-line text rules and the UTF-8 byte limit.
fn check_line(field: &str, text: &str, limit: usize) -> Result<(), String> {
    if text.len() > limit {
        return Err(format!("`{field}` exceeds {limit} UTF-8 bytes"));
    }
    check_text(field, text, limit)
}

#[cfg(test)]
mod tests;
