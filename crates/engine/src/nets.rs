//! Sparse persistent electrical identities; the authored schematic remains the connectivity authority.

mod anchor;
mod commands;
mod graph;
mod reconcile;
mod validation;
pub(crate) use anchor::schema as anchor_schema;
pub use anchor::{Anchor, AnchorKind, Scope, ScopeKind};
pub use commands::{PROMOTE, RESOLVE_AMBIGUITY};

use crate::board::update::SourceCapture;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum retained live, ambiguous and retired identities in one registry.
pub const MAX_RECORDS: usize = 4096;
/// Bound registry payloads independently from sheets and pinned assets.
pub const MAX_REGISTRY_BYTES: usize = 8 * 1024 * 1024;

/// A source net within an exact frozen capture, without saving its complete membership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lineage {
    /// Fresh capture identity and canonical content digest of the immutable source bundle.
    pub snapshot: SourceCapture,
    /// Persistent source UUID when present; None denotes a derived source in that capture.
    #[serde(deserialize_with = "required_nullable")]
    pub net: Option<Uuid>,
    /// One typed anchor identifying that source network within its frozen graph.
    pub anchor: Anchor,
}

/// Require a historical identity field while allowing an explicit null for a derived source.
fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(de)
}

/// Why an identity became historical; retirement never authorizes allocation of the same UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retirement {
    /// No meaningful electrical continuation remains.
    Removed,
    /// The selected continuation joined another meaningful network under a fresh identity.
    Merged,
    /// Explicit ambiguity resolution assigned its branch to the preserved source identity.
    Resolution,
}

/// Current identity authority or explicitly historical/review context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum State {
    /// The primary and fallback anchors resolve to one current meaningful network.
    Active {},
    /// No unique anchored continuation was established; source anchors are historical.
    Ambiguous {
        /// Immutable source capture for the unresolved choice.
        snapshot: SourceCapture,
        /// Current candidate identities, without duplicating their memberships.
        branches: Vec<Uuid>,
    },
    /// A reserved historical identity with no current authoritative network.
    Retired {
        /// Immutable source capture at retirement.
        snapshot: SourceCapture,
        /// Reason the ID was retired rather than silently reassigned.
        reason: Retirement,
        /// Resulting identities when present; these references may themselves later retire.
        successors: Vec<Uuid>,
    },
}

/// One bounded identity record; anchor order is persisted rather than refreshed from membership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// Independently allocated net UUID, never derived from a name, anchor or graph membership.
    pub id: Uuid,
    /// Primary live anchor, or explicitly historical evidence for inactive states.
    pub primary: Anchor,
    /// At most three distinct ordered fallback anchors.
    pub fallbacks: Vec<Anchor>,
    /// Snapshot-qualified split/merge sources, independent of current graph membership.
    pub lineage: Vec<Lineage>,
    /// Active authority, unresolved choice or historical reservation.
    pub state: State,
}

impl Record {
    /// Iterate primary then persisted fallback order, without recomputing graph membership.
    pub(crate) fn anchors(&self) -> impl Iterator<Item = &Anchor> {
        std::iter::once(&self.primary).chain(&self.fallbacks)
    }
}

/// Optional instance-owned identity registry, sorted by UUID and absent until first promotion.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// Active records and retained ambiguous/retired identities in canonical UUID order.
    pub records: Vec<Record>,
}

impl Registry {
    /// Reserve numeric identities referenced by live or historical anchors, even after deletion.
    pub(crate) fn highest_object_id(&self) -> u64 {
        self.records
            .iter()
            .flat_map(|record| record.anchors().chain(record.lineage.iter().map(|source| &source.anchor)))
            .map(|a| a.object)
            .max()
            .unwrap_or(0)
    }
    /// Resolve an exact identity, including historical reservations, without promoting a net.
    pub fn record(&self, id: Uuid) -> Option<&Record> {
        self.records.iter().find(|record| record.id == id)
    }
}

#[cfg(test)]
mod anchor_tests;
#[cfg(test)]
mod tests;
