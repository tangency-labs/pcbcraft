//! Stable presentation numbers share persistent net ownership rather than component references.
use super::{Anchor, Record, Registry, State, graph::Network};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Reject explicit null and zero; omission remains the exact older-record vocabulary.
pub(super) fn stored_number<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<u32>, D::Error> {
    let number = <u32 as serde::Deserialize>::deserialize(de)?;
    if number == 0 {
        return Err(serde::de::Error::custom("Generated net numbers must be positive."));
    }
    Ok(Some(number))
}

/// Reserve a new number beyond every live and historical number; overflow refuses publication.
pub(super) fn allocate(last: &mut u32) -> Result<u32, String> {
    *last = last.checked_add(1).ok_or("Generated net number space is exhausted.")?;
    Ok(*last)
}

/// A read-only adoption plan. Legacy numbers are deterministic until an authored transaction
/// persists them; that transaction plans against its before graph before any anchors disappear.
pub(super) struct Plan {
    pub records: BTreeMap<Uuid, u32>,
    pub networks: Vec<Option<u32>>,
}

/// Number existing identities first, preserving all stored values, then legacy derived networks
/// in canonical anchor order. No registry mutation, ID allocation or document adoption occurs.
pub(super) fn plan(registry: &Registry, networks: &[Network]) -> Result<Plan, String> {
    let mut used = BTreeSet::new();
    for number in registry.records.iter().filter_map(|record| record.generated_number) {
        if number == 0 || !used.insert(number) {
            return Err("Generated net numbers must be positive and unique, including retired identities.".into());
        }
    }
    let mut last = used.last().copied().unwrap_or(0);
    let mut records = BTreeMap::new();
    for record in registry.records.iter() {
        let number = match record.generated_number {
            Some(number) => number,
            None => allocate(&mut last)?,
        };
        records.insert(record.id, number);
    }
    let active: BTreeMap<&Anchor, &Record> =
        registry.records.iter().filter(|record| matches!(record.state, State::Active {})).map(|r| (&r.primary, r)).collect();
    let mut order: Vec<_> = networks.iter().enumerate().filter(|(_, network)| network.meaningful()).collect();
    order.sort_by_key(|(_, network)| network.anchors.first());
    let mut numbered = vec![None; networks.len()];
    for (index, network) in order {
        numbered[index] = Some(match network.anchors.iter().find_map(|anchor| active.get(anchor)) {
            Some(record) => records[&record.id],
            None => allocate(&mut last)?,
        });
    }
    Ok(Plan { records, networks: numbered })
}

/// A short neutral label whose identity never encodes component references or pin names.
pub(super) fn name(number: u32) -> String {
    format!("Net-{number:04}")
}
