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
    #[serde(default, skip_serializing_if = "PairState::is_active", deserialize_with = "stored_state")]
    pub state: PairState,
}

/// Native records and review states are keyed objects, never positional aliases.
/// Preserve the key stream for Serde's missing, duplicate and unknown field checks.
fn stored_object<'de, D: de::Deserializer<'de>, T: Deserialize<'de>>(de: D, expected: &'static str) -> Result<T, D::Error> {
    struct Object<T>(&'static str, std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> de::Visitor<'de> for Object<T> {
        type Value = T;

        /// Describe the only allowed shape.
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }

        /// Decode the closed typed value directly from its map.
        fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }
    de.deserialize_map(Object(expected, std::marker::PhantomData))
}

/// A present state must use the keyed native representation; omission defaults separately.
fn stored_state<'de, D: de::Deserializer<'de>>(de: D) -> Result<PairState, D::Error> {
    stored_object(de, "a differential pair state object")
}

/// Decode list members using the keyed native record shape.
struct StoredPair(Pair);

impl<'de> Deserialize<'de> for StoredPair {
    /// Refuse list items encoded as positional data.
    fn deserialize<D: de::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        stored_object(de, "a differential pair object").map(Self)
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

/// Validate one stored pair at a time, keeping the canonical order and member claims
/// across batches. The caller supplies the identities of the unmodified net rows.
#[derive(Default)]
pub(super) struct PairValidation {
    members: BTreeSet<Uuid>,
    previous: Option<Uuid>,
}

impl PairValidation {
    /// Reject one malformed or conflicting relationship without traversing other rows.
    pub(super) fn row(&mut self, pair: &Pair, contains_net: impl Fn(Uuid) -> bool) -> Result<(), String> {
        if pair.id.is_nil() || self.previous.is_some_and(|id| id >= pair.id) || contains_net(pair.id) {
            return Err("Pairs require unique non-nil UUIDs in canonical order, distinct from net identities.".into());
        }
        if pair.positive == pair.negative || !contains_net(pair.positive) || !contains_net(pair.negative) {
            return Err("Pair members must be distinct persistent nets in the owning registry.".into());
        }
        if !self.members.insert(pair.positive) || !self.members.insert(pair.negative) {
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
        self.previous = Some(pair.id);
        Ok(())
    }
}

impl Registry {
    /// Reserve both net and pair identities against UUID collisions, independently of capacity.
    pub(super) fn reserved_ids(&self) -> BTreeSet<Uuid> {
        self.records.iter().map(|record| record.id).chain(self.pairs.iter().map(|pair| pair.id)).collect()
    }

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
        let mut validation = PairValidation::default();
        for pair in self.pairs.iter() {
            validation.row(pair, |id| nets.contains(&id))?;
        }
        Ok(())
    }
}
