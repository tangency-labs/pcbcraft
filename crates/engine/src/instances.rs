//! Stable identities for the existing single-use sheet hierarchy, independent of storage home.

use crate::{CommandError, Project, SheetTarget, sheets::SchematicSheet};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use uuid::Uuid;

/// One use shared by every ordinary sheet in the current single-use project model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircuitUse {
    /// Stable use identity; components retain their existing numeric instance identities.
    pub id: Uuid,
}

/// One use of a sheet definition, with its explicitly validated hierarchy relationship.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetOccurrence {
    /// Stable identity retained through rename, reparenting, save and reopen.
    pub id: Uuid,
    /// Existing sheet definition UUID, independent of its name and file path.
    pub definition: Uuid,
    /// The project's sole circuit use; distinct uses of a definition remain unsupported.
    pub circuit_use: Uuid,
    /// Parent occurrence, or explicit null for a root.
    #[serde(deserialize_with = "nullable")]
    pub parent: Option<Uuid>,
    /// Subsheet-symbol numeric ID in the parent definition, or explicit null for a root.
    #[serde(deserialize_with = "nullable")]
    pub parent_slot: Option<u64>,
}

/// Logical instance records with exactly one home: project schematic scope or the sole board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instances {
    /// Exactly one circuit use until repeated-instance editing is implemented.
    pub circuit_uses: Vec<CircuitUse>,
    /// Exactly one occurrence per sheet, serialized in occurrence UUID order.
    pub sheet_occurrences: Vec<SheetOccurrence>,
}

/// Require the field while permitting explicit null for a root's absent relationship.
fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(de)
}

/// Domain-separated legacy identities use raw RFC-order UUID bytes and the first digest half.
/// The fixed role includes its terminal zero byte; UUID version/variant bits are set explicitly.
fn legacy_id(role: &[u8], project: Uuid, sheet: Option<Uuid>) -> Uuid {
    let mut hash = Sha256::new();
    hash.update(role);
    hash.update(project.as_bytes());
    if let Some(sheet) = sheet {
        hash.update(sheet.as_bytes());
    }
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

impl Project {
    /// Whether occurrence metadata has been adopted for current-format saving.
    /// Legacy inspection derives the same compatibility identities without adopting them.
    pub fn has_sheet_occurrences(&self) -> bool {
        self.circuit_use.is_some()
    }

    /// Resolve this project's shared use without deriving identity from any hierarchy path.
    fn resolved_circuit_use(&self) -> Uuid {
        self.circuit_use.unwrap_or_else(|| legacy_id(b"pcbcraft:legacy-circuit-use:1\0", self.id, None))
    }

    /// Resolve one sheet's stable occurrence, including the untouched legacy compatibility case.
    pub fn sheet_occurrence(&self, target: SheetTarget) -> Option<SheetOccurrence> {
        if target.project != self.id {
            return None;
        }
        self.sheet_instances().sheet_occurrences.into_iter().find(|record| record.definition == target.sheet)
    }

    /// Inspect single-use identities without changing revision, dirty state or saved bytes.
    /// Parent/slot relationships come from authored hierarchy and never independently retarget it.
    pub fn sheet_instances(&self) -> Instances {
        let circuit_use = self.resolved_circuit_use();
        let ids: BTreeMap<_, _> = self.sheets.iter().map(|sheet| (sheet.sheet_id, self.occurrence_id(sheet))).collect();
        let parents: BTreeMap<_, _> =
            self.sheets.iter().flat_map(|sheet| sheet.subsheets.iter().map(move |slot| (slot.child, (sheet.sheet_id, slot.id)))).collect();
        let mut sheet_occurrences: Vec<_> = self
            .sheets
            .iter()
            .map(|sheet| {
                let parent = parents.get(&sheet.sheet_id);
                SheetOccurrence {
                    id: ids[&sheet.sheet_id],
                    definition: sheet.sheet_id,
                    circuit_use,
                    parent: parent.and_then(|(sheet, _)| ids.get(sheet).copied()),
                    parent_slot: parent.map(|(_, slot)| *slot),
                }
            })
            .collect();
        sheet_occurrences.sort_by_key(|record| record.id);
        Instances { circuit_uses: vec![CircuitUse { id: circuit_use }], sheet_occurrences }
    }

    /// Return a sheet's stored ID or its deterministic compatibility ID before first Save.
    fn occurrence_id(&self, sheet: &SchematicSheet) -> Uuid {
        sheet.occurrence_id.unwrap_or_else(|| legacy_id(b"pcbcraft:legacy-sheet-occurrence:1\0", self.id, Some(sheet.sheet_id)))
    }

    /// Prepare occurrence persistence within the current board-folder layout, without editing
    /// geometry, component identities or electrical history. Invalid collisions are refused.
    pub fn with_sheet_occurrences(&self) -> Result<Self, String> {
        if self.layout.is_none() {
            return Err("Sheet occurrences require the current board-folder layout.".into());
        }
        let mut next = self.clone();
        let records = self.sheet_instances();
        next.install_sheet_instances(records)?;
        Ok(next)
    }

    /// Validate the exact single-use hierarchy before installing decoded or migrated identities.
    /// No missing record, duplicate definition or foreign relationship is repaired by inference.
    pub(crate) fn install_sheet_instances(&mut self, records: Instances) -> Result<(), String> {
        if records.circuit_uses.len() != 1 || records.sheet_occurrences.len() != self.sheets.len() {
            return Err("Instance records require one circuit use and one occurrence per sheet.".into());
        }
        let circuit_use = records.circuit_uses[0].id;
        let mut ids: BTreeSet<_> = self.sheets.iter().map(|sheet| sheet.sheet_id).chain([self.id]).collect();
        if let Some(board) = self.board_target() {
            ids.insert(board.board);
        }
        if circuit_use.is_nil() || !ids.insert(circuit_use) {
            return Err("Invalid or colliding circuit-use identity.".into());
        }
        let mut occurrences = BTreeMap::new();
        for record in &records.sheet_occurrences {
            if record.id.is_nil()
                || !ids.insert(record.id)
                || record.circuit_use != circuit_use
                || !self.sheets.iter().any(|sheet| sheet.sheet_id == record.definition)
                || occurrences.insert(record.definition, record).is_some()
            {
                return Err("Invalid, duplicate or foreign sheet occurrence.".into());
            }
        }
        for sheet in self.sheets.iter() {
            let record = occurrences[&sheet.sheet_id];
            let parent = self
                .sheets
                .iter()
                .find_map(|parent| parent.subsheets.iter().find(|slot| slot.child == sheet.sheet_id).map(|slot| (parent.sheet_id, slot.id)));
            let expected = parent.map(|(parent, slot)| (occurrences[&parent].id, slot));
            if record.parent.zip(record.parent_slot) != expected || record.parent.is_some() != record.parent_slot.is_some() {
                return Err("Occurrence parent/slot does not match the sheet hierarchy.".into());
            }
        }
        self.circuit_use = Some(circuit_use);
        for sheet in Arc::make_mut(&mut self.sheets) {
            sheet.occurrence_id = Some(occurrences[&sheet.sheet_id].id);
        }
        Ok(())
    }

    /// Allocate an independent occurrence for a new sheet after adoption; legacy documents keep
    /// their compatibility representation until first Save. Failure publishes no sheet or ID.
    pub(crate) fn new_sheet_record(&self, id: Uuid, name: String) -> Result<SchematicSheet, CommandError> {
        let mut sheet = SchematicSheet::empty(id, name);
        sheet.occurrence_id = self.circuit_use.map(|_| crate::fresh_id()).transpose()?;
        Ok(sheet)
    }

    /// Adopt saved occurrence metadata across history while preserving later created sheets and
    /// current hierarchy. A sheet outside the saved capture receives its pre-adoption legacy ID.
    pub(crate) fn adopt_occurrence_metadata(&mut self, saved: &Self) -> bool {
        let Some(circuit_use) = saved.circuit_use else { return false };
        if self.circuit_use.is_some() {
            return false;
        }
        let records = self.sheet_instances();
        let saved_ids: BTreeMap<_, _> = saved.sheet_instances().sheet_occurrences.into_iter().map(|record| (record.definition, record.id)).collect();
        let ids: BTreeMap<_, _> = records
            .sheet_occurrences
            .into_iter()
            .map(|record| (record.definition, saved_ids.get(&record.definition).copied().unwrap_or(record.id)))
            .collect();
        self.circuit_use = Some(circuit_use);
        for sheet in Arc::make_mut(&mut self.sheets) {
            sheet.occurrence_id = Some(ids[&sheet.sheet_id]);
        }
        true
    }
}

#[cfg(test)]
mod tests;
