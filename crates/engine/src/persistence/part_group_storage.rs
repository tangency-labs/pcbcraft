//! Compact membership storage. Dictionary positions are local compression references only;
//! decoding restores the same part-instance keys and group UUIDs used by shared commands.
use crate::{
    instances::Instances,
    part_groups::{MAX_GROUPS, MAX_MEMBERSHIPS, Membership, Memberships, PartInstanceKey},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// One sorted part-instance key and its sorted, nonempty dictionary references.
#[derive(Clone, Debug, Serialize)]
pub(super) struct CompactPartInstance {
    part_instance: PartInstanceKey,
    groups: Vec<u8>,
}

/// At most 256 UUIDs and 4096 rows, expanding to at most the supported membership count.
#[derive(Clone, Debug, Serialize)]
pub(super) struct CompactMemberships {
    groups: Vec<Uuid>,
    pub(super) part_instances: Vec<CompactPartInstance>,
}

/// Require JSON objects while leaving field parsing and duplicate-key rejection to Serde.
fn object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<T, D::Error> {
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Object<T> {
        type Value = T;
        /// State the native-record shape required by this adapter.
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a compact membership object")
        }
        /// Preserve the original key stream, including duplicate keys, for typed validation.
        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    de.deserialize_map(Object(std::marker::PhantomData))
}

impl<'de> Deserialize<'de> for CompactPartInstance {
    /// Decode a closed row object; positional JSON arrays are not record aliases.
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            part_instance: PartInstanceKey,
            groups: Vec<u8>,
        }
        let fields: Fields = object(de)?;
        Ok(Self { part_instance: fields.part_instance, groups: fields.groups })
    }
}

impl<'de> Deserialize<'de> for CompactMemberships {
    /// Decode a closed dictionary object without losing duplicate-key diagnostics.
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            groups: Vec<Uuid>,
            part_instances: Vec<CompactPartInstance>,
        }
        let fields: Fields = object(de)?;
        Ok(Self { groups: fields.groups, part_instances: fields.part_instances })
    }
}

impl CompactMemberships {
    /// Build a sorted dictionary from actual membership UUIDs, without using catalog positions.
    pub(super) fn from_memberships(memberships: &Memberships) -> Result<Self, String> {
        let groups: Vec<_> = memberships.iter().map(|record| record.group).collect::<BTreeSet<_>>().into_iter().collect();
        let part_instances = memberships
            .part_instances()
            .map(|part_instance| {
                let references = memberships
                    .groups_of(part_instance)
                    .iter()
                    .map(|group| {
                        groups
                            .binary_search(group)
                            .ok()
                            .and_then(|index| u8::try_from(index).ok())
                            .ok_or_else(|| "Part-group dictionary reference is out of range.".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CompactPartInstance { part_instance, groups: references })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self { groups, part_instances })
    }

    /// The UUID dictionary, written once before the incremental part-instance rows.
    pub(super) fn groups(&self) -> &[Uuid] {
        &self.groups
    }

    /// Refuse malformed dictionaries, repeated or unordered rows/references, unused dictionary
    /// entries and excess expansion before installing any decoded memberships.
    pub(super) fn expand(self) -> Result<Vec<Membership>, String> {
        if self.groups.is_empty()
            || self.groups.len() > MAX_GROUPS
            || self.groups.iter().any(Uuid::is_nil)
            || self.groups.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err("Compact part-group dictionary must contain sorted unique nonnil UUIDs and be bounded.".into());
        }
        if self.part_instances.is_empty()
            || self.part_instances.len() > crate::schematic::MAX_SYMBOLS
            || self.part_instances.windows(2).any(|pair| pair[0].part_instance >= pair[1].part_instance)
        {
            return Err("Compact part-group rows must be nonempty, bounded and sorted by unique part instance.".into());
        }
        let mut used = vec![false; self.groups.len()];
        let mut memberships = Vec::new();
        for row in self.part_instances {
            if row.groups.is_empty() || row.groups.len() > MAX_GROUPS || row.groups.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err("Compact part-group references must be nonempty, bounded, sorted and unique.".into());
            }
            if memberships.len() + row.groups.len() > MAX_MEMBERSHIPS {
                return Err("Compact part-group membership expansion exceeds its limit.".into());
            }
            for reference in row.groups {
                let index = usize::from(reference);
                let group = *self.groups.get(index).ok_or("Compact part-group dictionary reference is out of range.")?;
                used[index] = true;
                memberships.push(Membership { part_instance: row.part_instance, group });
            }
        }
        if used.contains(&false) {
            return Err("Compact part-group dictionary contains unused UUIDs.".into());
        }
        Ok(memberships)
    }
}

/// A present compact payload cannot be null or a differently shaped value.
pub(super) fn compact_value<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<CompactMemberships>, D::Error> {
    CompactMemberships::deserialize(de).map(Some)
}

/// Storage-only instance envelope; the public instance projection still uses UUID memberships.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredInstances {
    pub(super) circuit_uses: Vec<crate::instances::CircuitUse>,
    pub(super) sheet_occurrences: Vec<crate::instances::SheetOccurrence>,
    #[serde(default, deserialize_with = "value")]
    pub(super) net_registry: Option<crate::nets::Registry>,
    #[serde(default, deserialize_with = "crate::part_groups::stored_memberships")]
    pub(super) part_group_memberships: Option<Vec<Membership>>,
    #[serde(default, deserialize_with = "compact_value")]
    pub(super) compact_part_group_memberships: Option<CompactMemberships>,
}

/// Require a present registry to be an object rather than accepting null as omission.
fn value<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<crate::nets::Registry>, D::Error> {
    crate::nets::Registry::deserialize(de).map(Some)
}

impl StoredInstances {
    /// Select exactly the encoding promised by the project/instance headers, expanding UUIDs
    /// only for a compact envelope, including project/51 and /56 containing designator locks and project/64 with lifecycle assessments.
    /// The normal instance installer validates foreign identities.
    pub(super) fn into_records(self, compact: bool) -> Result<Instances, String> {
        if compact != self.compact_part_group_memberships.is_some() || (compact && self.part_group_memberships.is_some()) {
            return Err("Compact part-group memberships require project format 39, 36, 45, 48, 51, 56 or 64. When board-owned, use instances format 7 without pairs or 8 with pairs, and exactly one membership encoding.".into());
        }
        let part_group_memberships = match self.compact_part_group_memberships {
            Some(memberships) => Some(memberships.expand()?),
            None => self.part_group_memberships,
        };
        Ok(Instances {
            circuit_uses: self.circuit_uses,
            sheet_occurrences: self.sheet_occurrences,
            net_registry: self.net_registry,
            part_group_memberships,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// Pure compression round trips preserve identities rather than dictionary positions.
    #[test]
    fn compact_dictionary_round_trips_exact_membership_ids() {
        let records = vec![
            Membership { part_instance: 1, group: Uuid::from_u128(1) },
            Membership { part_instance: 1, group: Uuid::from_u128(2) },
            Membership { part_instance: 9, group: Uuid::from_u128(2) },
        ];
        let stored = CompactMemberships::from_memberships(&Memberships::from_records(&records)).unwrap();
        assert_eq!(stored.groups(), &[Uuid::from_u128(1), Uuid::from_u128(2)]);
        assert_eq!(
            serde_json::to_value(&stored).unwrap()["part_instances"],
            json!([
                {"part_instance":1,"groups":[0,1]}, {"part_instance":9,"groups":[1]}
            ])
        );
        assert_eq!(stored.expand().unwrap(), records);
    }

    /// Malformed dictionaries and references fail for their own condition, not a missing header.
    #[test]
    fn malformed_compact_dictionaries_and_rows_are_refused() {
        let records = [Membership { part_instance: 1, group: Uuid::from_u128(1) }, Membership { part_instance: 9, group: Uuid::from_u128(2) }];
        let stored = CompactMemberships::from_memberships(&Memberships::from_records(&records)).unwrap();
        let baseline = serde_json::to_value(stored).unwrap();
        let mut cases: Vec<(Value, &str)> = Vec::new();
        for groups in [
            json!([]),
            json!([Uuid::nil()]),
            json!([Uuid::from_u128(1), Uuid::from_u128(1)]),
            json!([Uuid::from_u128(2), Uuid::from_u128(1)]),
            json!((1..=257).map(Uuid::from_u128).collect::<Vec<_>>()),
        ] {
            let mut value = baseline.clone();
            value["groups"] = groups;
            cases.push((value, "dictionary must contain"));
        }
        for rows in [
            json!([]),
            json!([{"part_instance":1,"groups":[0]},{"part_instance":1,"groups":[1]}]),
            json!([{"part_instance":9,"groups":[0]},{"part_instance":1,"groups":[1]}]),
            json!((1..=4097).map(|key| json!({"part_instance":key,"groups":[0]})).collect::<Vec<_>>()),
        ] {
            let mut value = baseline.clone();
            value["part_instances"] = rows;
            cases.push((value, "rows must be"));
        }
        for references in [json!([]), json!([0, 0]), json!([1, 0]), json!(vec![0; 257])] {
            let mut value = baseline.clone();
            value["part_instances"][0]["groups"] = references;
            cases.push((value, "references must be"));
        }
        let mut out_of_range = baseline.clone();
        out_of_range["part_instances"][0]["groups"] = json!([2]);
        cases.push((out_of_range, "out of range"));
        let mut unused = baseline.clone();
        unused["part_instances"].as_array_mut().unwrap().pop();
        cases.push((unused, "unused UUIDs"));
        for (value, reason) in cases {
            let refusal = serde_json::from_value::<CompactMemberships>(value).unwrap().expand().unwrap_err();
            assert!(refusal.contains(reason), "expected {reason:?}, got {refusal:?}");
        }
        for reference in [json!(-1), json!(256), json!(0.0), json!(null), json!(true), json!("0")] {
            let mut value = baseline.clone();
            value["part_instances"][0]["groups"] = json!([reference]);
            let refusal = serde_json::from_value::<CompactMemberships>(value).unwrap_err().to_string();
            assert!(refusal.contains("u8"), "{refusal}");
        }
        for (pointer, field) in [("", "groups"), ("", "part_instances"), ("/part_instances/0", "part_instance"), ("/part_instances/0", "groups")] {
            let mut missing = baseline.clone();
            missing.pointer_mut(pointer).unwrap().as_object_mut().unwrap().remove(field);
            let refusal = serde_json::from_value::<CompactMemberships>(missing).unwrap_err().to_string();
            assert!(refusal.contains(&format!("missing field `{field}`")), "{refusal}");
        }
        for value in [json!(null), json!(false), json!(true), json!(0), json!(""), json!([baseline["groups"], baseline["part_instances"]])] {
            let refusal = serde_json::from_value::<CompactMemberships>(value).unwrap_err().to_string();
            assert!(refusal.contains("compact membership object"), "{refusal}");
        }
        for (pointer, value, reason) in [
            ("/groups", json!(null), "sequence"),
            ("/groups", json!([null]), "UUID"),
            ("/part_instances", json!(null), "sequence"),
            ("/part_instances/0", json!(null), "compact membership object"),
            ("/part_instances/0", json!([1, [0]]), "compact membership object"),
            ("/part_instances/0/part_instance", json!(null), "u64"),
            ("/part_instances/0/part_instance", json!(-1), "u64"),
            ("/part_instances/0/part_instance", json!(1.0), "u64"),
            ("/part_instances/0/groups", json!(null), "sequence"),
        ] {
            let mut changed = baseline.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            let refusal = serde_json::from_value::<CompactMemberships>(changed).unwrap_err().to_string();
            assert!(refusal.contains(reason), "expected {reason:?}, got {refusal:?}");
        }
        let raw = serde_json::to_string(&baseline).unwrap();
        for token in ["1.0", "1e0", "-0"] {
            let changed = raw.replacen("\"groups\":[0]", &format!("\"groups\":[{token}]"), 1);
            assert_ne!(changed, raw);
            let refusal = serde_json::from_str::<CompactMemberships>(&changed).unwrap_err().to_string();
            assert!(refusal.contains("u8"), "{refusal}");
        }
        let duplicate = raw.replacen("\"part_instance\":1", "\"part_instance\":1,\"part_instance\":1", 1);
        assert_ne!(duplicate, raw);
        assert!(serde_json::from_str::<CompactMemberships>(&duplicate).unwrap_err().to_string().contains("duplicate field"));
        let duplicate = raw.replacen("{\"groups\":[", "{\"groups\":[],\"groups\":[", 1);
        assert_ne!(duplicate, raw);
        assert!(serde_json::from_str::<CompactMemberships>(&duplicate).unwrap_err().to_string().contains("duplicate field `groups`"));
        for pointer in ["", "/part_instances/0"] {
            let mut unknown = baseline.clone();
            unknown.pointer_mut(pointer).unwrap()["extra"] = json!(true);
            let refusal = serde_json::from_value::<CompactMemberships>(unknown).unwrap_err().to_string();
            assert!(refusal.contains("unknown field `extra`"), "{refusal}");
        }
    }
}
