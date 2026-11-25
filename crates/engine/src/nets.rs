//! Persistent electrical identities and net-owned presentation numbers; the authored schematic remains the connectivity authority.

mod anchor;
mod appearance;
mod classes;
mod commands;
pub use appearance::{Appearance, DEFAULT_WIDTH_CP, INSPECT_APPEARANCE, MAX_WIDTH_CP, MIN_WIDTH_CP, SET_APPEARANCE};
pub(crate) use appearance::{appearance_param_schema, validate_appearance_patch};
pub(crate) use classes::target_schema as class_target_schema;
pub use classes::{BoundResolution, ClassBinding, ClassTarget, INSPECT_CLASSES, NetAddress, SET_CLASSES};
pub mod display;
mod graph;
mod numbering;
mod pairs;
pub use pairs::{MAX_PAIR_NAME_CHARS, MAX_PAIRS, Pair, PairState};
mod properties;
mod reconcile;
mod selection;
mod validation;
pub(crate) use anchor::schema as anchor_schema;
pub use anchor::{Anchor, AnchorKind, Scope, ScopeKind};
pub use commands::{PROMOTE, RESOLVE_AMBIGUITY};
pub use properties::{MAX_PROPERTY_TARGETS, PropertyValues, SET_PROPERTIES};
pub(crate) use properties::{property_changes_schema, property_targets_schema, validate_property_changes, validate_property_targets};

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
    /// Net-owned presentation number, retained through continuation and retired with its ID.
    /// Older records adopt their read-only projected number in the next topology transaction.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "numbering::stored_number")]
    pub generated_number: Option<u32>,
    /// Primary live anchor, or explicitly historical evidence for inactive states.
    pub primary: Anchor,
    /// At most three distinct ordered fallback anchors.
    pub fallbacks: Vec<Anchor>,
    /// Snapshot-qualified split/merge sources, independent of current graph membership.
    pub lineage: Vec<Lineage>,
    /// Active authority, unresolved choice or historical reservation.
    pub state: State,
    /// Dimensional class intent and originating assigning nets, without saved endpoint membership.
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "classes::stored_bindings")]
    pub class_bindings: Vec<ClassBinding>,
    /// Schematic wire drawing override, owned by the net identity rather than by a wire or name.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "appearance::stored_appearance")]
    pub appearance: Option<Appearance>,
}

impl Record {
    /// Iterate primary then persisted fallback order, without recomputing graph membership.
    pub(crate) fn anchors(&self) -> impl Iterator<Item = &Anchor> {
        std::iter::once(&self.primary).chain(&self.fallbacks)
    }
}

/// Instance-owned identity registry sorted by UUID; untouched legacy projects may omit it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// Active records and retained ambiguous/retired identities in canonical UUID order.
    pub records: Vec<Record>,
    /// UUID-sorted relationships share their list and individual records across history snapshots.
    #[serde(default, skip_serializing_if = "pair_list_empty", deserialize_with = "pairs::stored_pairs")]
    pub pairs: std::sync::Arc<Vec<std::sync::Arc<Pair>>>,
}

/// Keep registries without relationship intent byte-compatible with the earlier vocabulary.
fn pair_list_empty(pairs: &std::sync::Arc<Vec<std::sync::Arc<Pair>>>) -> bool {
    pairs.is_empty()
}

impl Registry {
    /// Numbered identities select the newer native envelope; old unnumbered files stay exact.
    pub(crate) fn has_generated_numbers(&self) -> bool {
        self.records.iter().any(|record| record.generated_number.is_some())
    }

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
