//! Emit large native net rows without treating historical membership as one work unit.

use super::{Record, State};
use serde::Serialize;
use std::io::Write;

/// Cursor of the compact byte meter (`RegistrySizeProbe`). It uses the ordinary Serde
/// form for small rows and for each item of a large row.
#[derive(Default)]
pub(crate) struct RecordEncoding {
    phase: u8,
    item: usize,
}

impl RecordEncoding {
    pub(crate) fn at_start(&self) -> bool {
        self.phase == 0
    }

    /// Write one compact record fragment or one member of its large history arrays.
    pub(crate) fn step(&mut self, out: &mut impl Write, row: &Record) -> Result<bool, String> {
        // Exhaustively destructure `Record` so adding a persisted field fails to compile
        // until both the incremental encoder and its parity test account for it.
        let Record { id, generated_number, primary, fallbacks, lineage, state, class_bindings } = row;
        let successors = match state {
            State::Active {} => 0,
            State::Ambiguous { branches, .. } => branches.len(),
            State::Retired { successors, .. } => successors.len(),
        };
        match self.phase {
            0 if lineage.len() + successors <= 8 => {
                json(out, row)?;
                return Ok(true);
            }
            0 => {
                emit(out, b"{\"id\":")?;
                json(out, id)?;
                if let Some(number) = generated_number {
                    emit(out, b",\"generated_number\":")?;
                    json(out, number)?;
                }
                emit(out, b",\"primary\":")?;
                json(out, primary)?;
                emit(out, b",\"fallbacks\":")?;
                json(out, fallbacks)?;
                emit(out, b",\"lineage\":[")?;
                self.phase = 1;
            }
            1 => {
                if let Some(source) = lineage.get(self.item) {
                    array_item(out, self.item, source)?;
                    self.item += 1;
                } else {
                    emit(out, b"],\"state\":")?;
                    self.item = 0;
                    match state {
                        State::Active {} => {
                            json(out, state)?;
                            self.phase = 3;
                        }
                        State::Ambiguous { snapshot, .. } => {
                            emit(out, b"{\"kind\":\"ambiguous\",\"snapshot\":")?;
                            json(out, snapshot)?;
                            emit(out, b",\"branches\":[")?;
                            self.phase = 2;
                        }
                        State::Retired { snapshot, reason, .. } => {
                            emit(out, b"{\"kind\":\"retired\",\"snapshot\":")?;
                            json(out, snapshot)?;
                            emit(out, b",\"reason\":")?;
                            json(out, reason)?;
                            emit(out, b",\"successors\":[")?;
                            self.phase = 2;
                        }
                    }
                }
            }
            2 => {
                let ids = match state {
                    State::Ambiguous { branches, .. } => branches,
                    State::Retired { successors, .. } => successors,
                    State::Active {} => return Err("active net record has no history list".into()),
                };
                if let Some(successor) = ids.get(self.item) {
                    array_item(out, self.item, successor)?;
                    self.item += 1;
                } else {
                    emit(out, b"]}")?;
                    self.phase = 3;
                }
            }
            3 => {
                if !class_bindings.is_empty() {
                    emit(out, b",\"class_bindings\":")?;
                    json(out, class_bindings)?;
                }
                emit(out, b"}")?;
                *self = Self::default();
                return Ok(true);
            }
            _ => return Err("invalid net record encoding phase".into()),
        }
        Ok(false)
    }
}

fn emit(out: &mut impl Write, bytes: &[u8]) -> Result<(), String> {
    out.write_all(bytes).map_err(|error| error.to_string())
}

fn json(out: &mut impl Write, value: &impl Serialize) -> Result<(), String> {
    serde_json::to_writer(out, value).map_err(|error| error.to_string())
}

/// Emit a comma-separated JSON array element.
fn array_item(out: &mut impl Write, index: usize, value: &impl Serialize) -> Result<(), String> {
    if index != 0 {
        emit(out, b",")?;
    }
    json(out, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::update::SourceCapture;
    use crate::nets::{Anchor, AnchorKind, ClassBinding, Lineage, Retirement, Scope, ScopeKind};
    use uuid::Uuid;

    /// Large net records with every optional field populated must match `serde_json::to_vec`
    /// byte-for-byte across all three `State` variants.
    #[test]
    fn large_record_encoding_matches_serde_with_all_optional_fields() {
        let project = Uuid::from_u128(1);
        let scope = Scope { kind: ScopeKind::Schematic, id: project };
        let primary = Anchor { kind: AnchorKind::Terminal, project, scope, occurrence: Uuid::nil(), object: 10, terminal: "1".into() };
        let fallback = Anchor { kind: AnchorKind::Label, project, scope, occurrence: Uuid::from_u128(2), object: 11, terminal: String::new() };
        let snapshot = SourceCapture { capture_id: Uuid::from_u128(3), content_digest: "a".repeat(64) };
        let ids: Vec<_> = (10..=137).map(Uuid::from_u128).collect();
        let record_id = Uuid::from_u128(4);
        let mut row = Record {
            id: record_id,
            generated_number: Some(42),
            primary: primary.clone(),
            fallbacks: vec![fallback],
            lineage: ids.iter().map(|&id| Lineage { snapshot: snapshot.clone(), net: Some(id), anchor: primary.clone() }).collect(),
            state: State::Active {},
            class_bindings: vec![ClassBinding { class: Uuid::from_u128(5), origin: record_id }],
        };
        for state in [
            State::Active {},
            State::Ambiguous { snapshot: snapshot.clone(), branches: ids.clone() },
            State::Retired { snapshot, reason: Retirement::Merged, successors: ids.clone() },
        ] {
            row.state = state;
            let mut cursor = RecordEncoding::default();
            let mut out = Vec::new();
            let mut steps = 0;
            while !cursor.step(&mut out, &row).unwrap() {
                steps += 1;
                assert!(steps < 512);
            }
            assert!(steps > ids.len());
            assert_eq!(out, serde_json::to_vec(&row).unwrap());
        }
    }
}
