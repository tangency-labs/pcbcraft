//! Authored, board-side probe intent on physical pads and vias, independent of net assignment.
use super::{Board, Pad};
use crate::{CommandError, Session, schematic::PinNumber};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// Author or clear one purpose's test-point marking in a single undo step.
pub const SET_TEST_POINT: &str = "pcb.set_test_point";

/// Board faces selected for one test-point purpose, independent of component orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sides {
    /// Top face only.
    Top,
    /// Bottom face only.
    Bottom,
    /// Both faces.
    Both,
}

/// Require a JSON object rather than the positional sequence serde's derived struct decoder accepts.
fn deserialize_object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<T, D::Error> {
    struct ObjectVisitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ObjectVisitor<T> {
        type Value = T;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a JSON object")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
}

/// Separate markings for the assembly fixture and bare-board electrical test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Flags {
    /// Faces marked for assembly testing, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<Sides>,
    /// Faces marked for fabrication testing, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fabrication: Option<Sides>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFlags {
    #[serde(default, deserialize_with = "marked")]
    assembly: Option<Sides>,
    #[serde(default, deserialize_with = "marked")]
    fabrication: Option<Sides>,
}

impl<'de> Deserialize<'de> for Flags {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw: RawFlags = deserialize_object(deserializer)?;
        Ok(Self { assembly: raw.assembly, fabrication: raw.fabrication })
    }
}

/// An omitted purpose is unmarked; an explicit null is malformed and is refused by name.
fn marked<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Sides>, D::Error> {
    match Option::<Sides>::deserialize(deserializer)? {
        Some(sides) => Ok(Some(sides)),
        None => Err(serde::de::Error::custom("a test-point purpose cannot be null; omit it to leave that purpose unmarked")),
    }
}

impl Flags {
    /// Neither test process has a face marked.
    pub(crate) fn is_empty(self) -> bool {
        self.assembly.is_none() && self.fabrication.is_none()
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Purpose {
    Assembly,
    Fabrication,
}

/// External object address in both command parameters and native board records.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Address {
    Pad { footprint: String, pad: String },
    Via { via: Uuid },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawAddress {
    Pad { footprint: String, pad: String },
    Via { via: Uuid },
}

impl<'de> Deserialize<'de> for Address {
    /// Serde's internally tagged decoder also reads `[kind, ...]` sequences; accept only objects.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match deserialize_object(deserializer)? {
            RawAddress::Pad { footprint, pad } => Self::Pad { footprint, pad },
            RawAddress::Via { via } => Self::Via { via },
        })
    }
}

/// Validated object identity scoped to the board containing the record.
#[derive(Clone, Copy)]
enum Target {
    Pad(Pad),
    Via(Uuid),
}

#[derive(Serialize)]
pub(crate) struct Record {
    target: Address,
    flags: Flags,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecord {
    target: Address,
    flags: Flags,
}

impl<'de> Deserialize<'de> for Record {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw: RawRecord = deserialize_object(deserializer)?;
        Ok(Self { target: raw.target, flags: raw.flags })
    }
}

impl Address {
    /// Accept only canonical pad addresses and a non-nil via identity.
    fn target(&self) -> Option<Target> {
        match self {
            Self::Pad { footprint, pad } => Pad::parse(footprint, pad).map(Target::Pad),
            Self::Via { via } => (!via.is_nil()).then_some(Target::Via(*via)),
        }
    }
}

/// Parse the same strict, discriminated address that the native record uses.
pub(crate) fn valid_target(value: &Value) -> bool {
    serde_json::from_value::<Address>(value.clone()).is_ok_and(|address| address.target().is_some())
}

/// Describe one placed pad or one via ID from inspection.
pub(crate) fn target_schema() -> Value {
    json!({"oneOf":[
        {"type":"object","properties":{"kind":{"const":"pad"},"footprint":{"type":"string","pattern":"^footprint-[1-9][0-9]*$"},"pad":{"type":"string","pattern":"^[A-Za-z0-9+_-]{1,8}$"}},"required":["kind","footprint","pad"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"const":"via"},"via":{"type":"string","format":"uuid"}},"required":["kind","via"],"additionalProperties":false}
    ]})
}

/// Indices resolved against the current board only, before any clone or edit.
enum Location {
    Pad(usize, PinNumber),
    Via(usize),
}

impl Location {
    /// Resolve a pad from the pinned definition (including unmapped pads) or a via from this board.
    fn resolve(board: &Board, target: Target) -> Option<Self> {
        match target {
            Target::Pad(pad) => {
                let index = board.footprints.binary_search_by_key(&pad.footprint, |placement| placement.id).ok()?;
                let definition = board.definition_for(board.footprints.get(index)?)?;
                definition.pads.iter().any(|land| land.number == pad.number.as_str()).then_some(Self::Pad(index, pad.number))
            }
            Target::Via(id) => Some(Self::Via(board.vias.binary_search_by_key(&id, |via| via.id).ok()?)),
        }
    }

    /// Read the two independent marks together; omission means neither is marked.
    /// The location was resolved against this board, so a missing index reads as unmarked.
    fn flags(&self, board: &Board) -> Flags {
        match *self {
            Self::Pad(index, pad) => {
                board.footprints.get(index).and_then(|placement| placement.test_points.get(pad.as_str()).copied()).unwrap_or_default()
            }
            Self::Via(index) => board.vias.get(index).map(|via| via.test_points).unwrap_or_default(),
        }
    }

    /// Attach only nonempty pad entries. A true deletion of the placement also deletes its marks.
    /// The location was resolved against this board's unchanged object lists, so lookups succeed.
    fn set(self, board: &mut Board, flags: Flags) {
        match self {
            Self::Pad(index, pad) => {
                if let Some(placement) = board.footprints.get_mut(index) {
                    if flags.is_empty() {
                        placement.test_points.remove(pad.as_str());
                    } else {
                        placement.test_points.insert(pad.as_str().into(), flags);
                    }
                }
            }
            Self::Via(index) => {
                if let Some(via) = board.vias.get_mut(index) {
                    via.test_points = flags;
                }
            }
        }
    }
}

impl Record {
    /// Build a native pad record from its placed identity and nonempty marks.
    pub(crate) fn pad(footprint: u64, pad: String, flags: Flags) -> Self {
        Self { target: Address::Pad { footprint: format!("footprint-{footprint}"), pad }, flags }
    }

    /// Build a native via record from its board-local UUID and nonempty marks.
    pub(crate) fn via(via: Uuid, flags: Flags) -> Self {
        Self { target: Address::Via { via }, flags }
    }
}

impl Board {
    /// The native board alone owns the records; omit the array when no object is marked.
    pub(crate) fn has_test_points(&self) -> bool {
        self.footprints.iter().any(|footprint| !footprint.test_points.is_empty()) || self.vias.iter().any(|via| !via.test_points.is_empty())
    }

    /// Attach records only after decoding the referenced placements, pinned definitions and vias.
    /// The caller discards the unpublished board if a target is absent, duplicated or unmarked.
    pub(crate) fn install_test_points(&mut self, records: Vec<Record>) -> Result<(), String> {
        for record in records {
            let location = record.target.target().and_then(|target| Location::resolve(self, target)).ok_or("unresolved test-point target")?;
            if record.flags.is_empty() || !location.flags(self).is_empty() {
                return Err("empty or duplicate test-point record".into());
            }
            location.set(self, record.flags);
        }
        Ok(())
    }
}

impl Session {
    /// Replace one purpose's faces, preserving the other purpose. Repeated requests are no-ops;
    /// stale revisions and unresolved objects are rejected before any state is published.
    pub(crate) fn set_test_point(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let invalid = |message: &str| CommandError::InvalidParams(message.into());
        let param = |name: &str| params.get(name).cloned().unwrap_or(Value::Null);
        let address: Address = serde_json::from_value(param("target")).map_err(|e| invalid(&e.to_string()))?;
        let location = address
            .target()
            .and_then(|target| Location::resolve(&project.board, target))
            .ok_or_else(|| invalid("No such pad or via on this board."))?;
        let purpose: Purpose = serde_json::from_value(param("purpose")).map_err(|e| invalid(&e.to_string()))?;
        let sides: Option<Sides> = serde_json::from_value(param("sides")).map_err(|e| invalid(&e.to_string()))?;
        let previous = location.flags(&project.board);
        let mut flags = previous;
        match purpose {
            Purpose::Assembly => flags.assembly = sides,
            Purpose::Fabrication => flags.fabrication = sides,
        }
        if flags == previous {
            return Ok(());
        }
        let mut next = project.clone();
        location.set(Arc::make_mut(&mut next.board), flags);
        self.change(next)
    }
}

#[cfg(test)]
mod tests;
