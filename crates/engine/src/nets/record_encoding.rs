//! Emit large native net rows without treating historical membership as one work unit.

use super::{Record, State};
use serde::Serialize;
use std::io::Write;

/// Cursor shared by the compact byte meter and native document writer. Both use
/// the ordinary Serde form for small rows and for each item of a large row.
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
            0 if lineage.len() + successors + class_bindings.len() <= 8 => {
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
                    self.item = 0;
                    self.phase = 3;
                }
            }
            3 => {
                if class_bindings.is_empty() {
                    emit(out, b"}")?;
                    *self = Self::default();
                    return Ok(true);
                }
                emit(out, b",\"class_bindings\":[")?;
                self.item = 0;
                self.phase = 4;
            }
            4 => {
                if let Some(binding) = class_bindings.get(self.item) {
                    array_item(out, self.item, binding)?;
                    self.item += 1;
                } else {
                    emit(out, b"]}")?;
                    *self = Self::default();
                    return Ok(true);
                }
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

    /// A row that is only large because of `class_bindings` must stream one binding per step,
    /// while a large row with no `class_bindings` must still omit the key.
    #[test]
    fn large_class_bindings_stream_one_per_step_and_empty_is_omitted() {
        let project = Uuid::from_u128(1);
        let scope = Scope { kind: ScopeKind::Schematic, id: project };
        let primary = Anchor { kind: AnchorKind::Terminal, project, scope, occurrence: Uuid::nil(), object: 10, terminal: "1".into() };
        let snapshot = SourceCapture { capture_id: Uuid::from_u128(3), content_digest: "a".repeat(64) };
        let record_id = Uuid::from_u128(4);

        let bindings: Vec<_> = (100..112).map(|n| ClassBinding { class: Uuid::from_u128(n), origin: record_id }).collect();
        let many_bindings = Record {
            id: record_id,
            generated_number: None,
            primary: primary.clone(),
            fallbacks: Vec::new(),
            lineage: Vec::new(),
            state: State::Active {},
            class_bindings: bindings,
        };
        let mut cursor = RecordEncoding::default();
        let mut out = Vec::new();
        let mut steps = 0;
        while !cursor.step(&mut out, &many_bindings).unwrap() {
            steps += 1;
            assert!(steps < 64);
        }
        assert!(cursor.at_start());
        assert!(steps > many_bindings.class_bindings.len());
        assert_eq!(out, serde_json::to_vec(&many_bindings).unwrap());

        let empty_bindings = Record {
            id: record_id,
            generated_number: None,
            primary: primary.clone(),
            fallbacks: Vec::new(),
            lineage: (10..22).map(|n| Lineage { snapshot: snapshot.clone(), net: Some(Uuid::from_u128(n)), anchor: primary.clone() }).collect(),
            state: State::Active {},
            class_bindings: Vec::new(),
        };
        let mut cursor = RecordEncoding::default();
        let mut out = Vec::new();
        let mut steps = 0;
        while !cursor.step(&mut out, &empty_bindings).unwrap() {
            steps += 1;
            assert!(steps < 64);
        }
        assert!(cursor.at_start());
        assert!(steps > empty_bindings.lineage.len());
        assert_eq!(out, serde_json::to_vec(&empty_bindings).unwrap());
        assert!(!String::from_utf8(out).unwrap().contains("class_bindings"));
    }
}
