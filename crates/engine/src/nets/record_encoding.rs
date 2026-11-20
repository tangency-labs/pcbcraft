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
        let successors = match &row.state {
            State::Active {} => 0,
            State::Ambiguous { branches, .. } => branches.len(),
            State::Retired { successors, .. } => successors.len(),
        };
        match self.phase {
            0 if row.lineage.len() + successors <= 8 => {
                json(out, row)?;
                return Ok(true);
            }
            0 => {
                emit(out, b"{\"id\":")?;
                json(out, &row.id)?;
                if let Some(number) = row.generated_number {
                    emit(out, b",\"generated_number\":")?;
                    json(out, &number)?;
                }
                emit(out, b",\"primary\":")?;
                json(out, &row.primary)?;
                emit(out, b",\"fallbacks\":")?;
                json(out, &row.fallbacks)?;
                emit(out, b",\"lineage\":[")?;
                self.phase = 1;
            }
            1 => {
                if let Some(source) = row.lineage.get(self.item) {
                    array_item(out, self.item, source)?;
                    self.item += 1;
                } else {
                    emit(out, b"],\"state\":")?;
                    self.item = 0;
                    match &row.state {
                        State::Active {} => {
                            json(out, &row.state)?;
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
                let ids = match &row.state {
                    State::Ambiguous { branches, .. } => branches,
                    State::Retired { successors, .. } => successors,
                    State::Active {} => unreachable!(),
                };
                if let Some(id) = ids.get(self.item) {
                    array_item(out, self.item, id)?;
                    self.item += 1;
                } else {
                    emit(out, b"]}")?;
                    self.phase = 3;
                }
            }
            3 => {
                if !row.class_bindings.is_empty() {
                    emit(out, b",\"class_bindings\":")?;
                    json(out, &row.class_bindings)?;
                }
                emit(out, b"}")?;
                *self = Self::default();
                return Ok(true);
            }
            _ => unreachable!(),
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

/// Emit a stable compact array separator and the next member.
fn array_item(out: &mut impl Write, index: usize, value: &impl Serialize) -> Result<(), String> {
    if index != 0 {
        emit(out, b",")?;
    }
    json(out, value)
}
