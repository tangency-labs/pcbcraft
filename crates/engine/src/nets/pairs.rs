//! Differential relationships reference identities in their owning registry, not net names.

use super::Registry;
use serde::{Deserialize, Serialize, de};
use std::{collections::BTreeSet, fmt, sync::Arc};
use uuid::Uuid;

/// Each member belongs to at most one pair within the registry's 4,096 net identities.
pub const MAX_PAIRS: usize = super::MAX_RECORDS / 2;
/// Pair display names use the same character bound as authored net names.
pub const MAX_PAIR_NAME_CHARS: usize = 200;

/// Stored review intent, independent of physical analysis or routing qualification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PairState {
    /// Authored relationship with no stored request for review; not proof of electrical validity.
    Active {},
    /// Retained intent awaiting explicit review; member identities are not silently replaced.
    NeedsReview {
        /// Human-readable review context; trimmed, control-free and at most 200 characters.
        reason: String,
    },
}

impl Default for PairState {
    /// Omitted review state is active intent, with no extra state fields accepted on disk.
    fn default() -> Self {
        Self::Active {}
    }
}

impl PairState {
    /// Canonical records omit the default state without changing its meaning on reopen.
    fn is_active(&self) -> bool {
        matches!(self, Self::Active {})
    }
}

/// One relationship in the registry's project and schematic/board scope.
/// Positive and negative are persistent net UUIDs, including retained historical identities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    /// Independent relationship identity, stable through member/display-name edits.
    pub id: Uuid,
    /// Positive member's persistent identity in this registry.
    pub positive: Uuid,
    /// Negative member's persistent identity in this registry.
    pub negative: Uuid,
    /// Display name, not an identity or suffix-derived membership rule.
    pub name: String,
    /// Omission means active; review context survives save/reopen without physical qualification.
    #[serde(default, skip_serializing_if = "PairState::is_active")]
    pub state: PairState,
}

/// Native pair records are objects, not positional aliases for the named fields.
struct StoredPair(Pair);

impl<'de> Deserialize<'de> for StoredPair {
    /// Preserve the key stream for Serde's missing, duplicate and unknown field checks.
    fn deserialize<D: de::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct RecordObject;
        impl<'de> de::Visitor<'de> for RecordObject {
            type Value = StoredPair;

            /// Describe the only allowed record shape.
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a differential pair object")
            }

            /// Decode the closed typed record directly from its map.
            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                Pair::deserialize(de::value::MapAccessDeserializer::new(map)).map(StoredPair)
            }
        }
        de.deserialize_map(RecordObject)
    }
}

/// An absent pair list is the only empty encoding; bound the number of retained decoded rows.
pub(super) fn stored_pairs<'de, D: de::Deserializer<'de>>(de: D) -> Result<Arc<Vec<Arc<Pair>>>, D::Error> {
    struct PairList;
    impl<'de> de::Visitor<'de> for PairList {
        type Value = Arc<Vec<Arc<Pair>>>;

        /// Describe the native list shape for type errors.
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a list of differential pair objects")
        }

        /// Decode up to the record limit, refusing even a single excess or an explicit empty list.
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let invalid_length = || de::Error::custom("Pairs must be omitted or nonempty and bounded.");
            let mut pairs = Vec::new();
            while pairs.len() < MAX_PAIRS {
                let Some(StoredPair(pair)) = seq.next_element()? else {
                    return if pairs.is_empty() { Err(invalid_length()) } else { Ok(Arc::new(pairs)) };
                };
                pairs.push(Arc::new(pair));
            }
            if seq.next_element::<de::IgnoredAny>()?.is_some() {
                return Err(invalid_length());
            }
            Ok(Arc::new(pairs))
        }
    }
    de.deserialize_seq(PairList)
}

/// Names and review explanations are bounded authored display text, never identity keys.
fn valid_text(text: &str) -> bool {
    !text.is_empty() && text.trim() == text && text.chars().count() <= MAX_PAIR_NAME_CHARS && !text.chars().any(char::is_control)
}

impl Registry {
    /// Sparse relationship intent selects the pair-capable native envelope only when present.
    pub(crate) fn has_pairs(&self) -> bool {
        !self.pairs.is_empty()
    }

    /// Validate relationships within this registry; ownership comes from its single instance home.
    /// Historical members stay addressable without claiming a current routing/evaluation result.
    pub(super) fn validate_pairs(&self) -> Result<(), String> {
        if self.pairs.is_empty() {
            return Ok(());
        }
        if self.pairs.len() > MAX_PAIRS {
            return Err("Differential pair count exceeds its limit.".into());
        }
        let nets: BTreeSet<_> = self.records.iter().map(|record| record.id).collect();
        let mut members = BTreeSet::new();
        let mut previous = None;
        for pair in self.pairs.iter() {
            if pair.id.is_nil() || previous.is_some_and(|id| id >= pair.id) || nets.contains(&pair.id) {
                return Err("Pairs require unique non-nil UUIDs in canonical order, distinct from net identities.".into());
            }
            if pair.positive == pair.negative || !nets.contains(&pair.positive) || !nets.contains(&pair.negative) {
                return Err("Pair members must be distinct persistent nets in the owning registry.".into());
            }
            if !members.insert(pair.positive) || !members.insert(pair.negative) {
                return Err("A net may belong to at most one differential pair.".into());
            }
            if !valid_text(&pair.name) {
                return Err("Pair names must be trimmed, nonempty, control-free and at most 200 characters.".into());
            }
            if let PairState::NeedsReview { reason } = &pair.state
                && !valid_text(reason)
            {
                return Err("Pair review reasons must be trimmed, nonempty, control-free and at most 200 characters.".into());
            }
            previous = Some(pair.id);
        }
        Ok(())
    }
}
