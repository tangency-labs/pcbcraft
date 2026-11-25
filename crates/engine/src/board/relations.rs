//! Board-local reference dimensions. This module reads native geometry and owns
//! annotations only; it never writes placements, copper, or outline geometry.

pub(super) mod advisory;
mod catalog;
pub(crate) mod commands;
mod islands;
#[cfg(test)]
pub(crate) mod probes;
mod transaction;

pub(crate) use transaction::{Base, Disclosure, Prepared, RelationInput, RelationStage, ReviewSlot, certify, choices, reason, verify_final};

use crate::schematic::{MAX_CENTER_NM, PinNumber, Point};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

pub(crate) const MAX_RELATIONS: usize = 2048;
// Two rotations, a bounded local center and two half dimensions fit well inside this envelope.
const MAX_WITNESS_NM: i64 = MAX_CENTER_NM + 4 * super::definition::MAX_OFFSET_NM;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Relations {
    pub(crate) relations: Vec<Relation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Relation {
    id: Uuid,
    kind: Kind,
    refs: [Reference; 2],
    datum: u8,
    #[serde(default, deserialize_with = "super::reference::present", skip_serializing_if = "Option::is_none")]
    value: Option<i64>,
    #[serde(default, deserialize_with = "super::reference::present", skip_serializing_if = "Option::is_none")]
    side: Option<i8>,
    driving: bool,
    suppressed: bool,
    state: State,
    #[serde(default, deserialize_with = "super::reference::present", skip_serializing_if = "Option::is_none")]
    label: Option<Point>,
    // An ordered, board-space display record; never passed to resolution or measure.
    last_resolved: [Witness; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum State {
    Ok,
    Lost { since_revision: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Horizontal,
    Vertical,
    Aligned,
    PointEdge,
    Gap,
    AlignH,
    AlignV,
}

/// Native pinned address, with the explicit built-in legacy marker as the only library-free case.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    #[serde(default, deserialize_with = "super::reference::present", skip_serializing_if = "Option::is_none")]
    library: Option<String>,
    id: String,
    revision: u32,
}

/// A closed tag with only the payload belonging to that tag. Footprint tags always bind a definition.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "tag", deny_unknown_fields)]
enum Reference {
    #[serde(rename = "board.origin")]
    BoardOrigin {},
    #[serde(rename = "fp.origin")]
    Origin { fp: u64, def: Definition },
    #[serde(rename = "pad.center")]
    PadCenter { fp: u64, def: Definition, pad: String },
    #[serde(rename = "pad.corner")]
    PadCorner { fp: u64, def: Definition, pad: String, corner: Corner },
    #[serde(rename = "pad.edge")]
    PadEdge { fp: u64, def: Definition, pad: String, side: Side },
    #[serde(rename = "courtyard.edge")]
    CourtyardEdge { fp: u64, def: Definition, rect: usize, side: Side },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Corner {
    Ll,
    Lr,
    Ur,
    Ul,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Witness {
    Point { at: Point },
    Segment { a: Point, b: Point },
}

/// Check the coordinate envelope without signed overflow, even for malformed input.
fn bounded(point: Point, max: i64) -> bool {
    (-max..=max).contains(&point.x_nm) && (-max..=max).contains(&point.y_nm)
}

impl Witness {
    /// Display storage has a fixed arity, a native coordinate envelope and nondegenerate axis edges.
    fn valid_for(self, reference: &Reference) -> bool {
        match self {
            Self::Point { at } => !reference.is_edge() && bounded(at, MAX_WITNESS_NM),
            Self::Segment { a, b } => {
                reference.is_edge() && bounded(a, MAX_WITNESS_NM) && bounded(b, MAX_WITNESS_NM) && ((a.x_nm == b.x_nm) ^ (a.y_nm == b.y_nm))
            }
        }
    }

    /// True when two valid segment witnesses share the same projection axis (both vertical or both horizontal).
    fn parallel_segments(self, other: Self) -> bool {
        match (self, other) {
            (Self::Segment { a: a0, b: b0 }, Self::Segment { a: a1, b: b1 }) => (a0.x_nm == b0.x_nm) == (a1.x_nm == b1.x_nm),
            _ => false,
        }
    }
}

impl Reference {
    /// Native placement body or `None` for the board datum.
    fn footprint(&self) -> Option<(u64, &Definition)> {
        match self {
            Self::BoardOrigin {} => None,
            Self::Origin { fp, def }
            | Self::PadCenter { fp, def, .. }
            | Self::PadCorner { fp, def, .. }
            | Self::PadEdge { fp, def, .. }
            | Self::CourtyardEdge { fp, def, .. } => Some((*fp, def)),
        }
    }

    /// Read the exact pad number; non-pad references have no electrical land identity.
    fn pad(&self) -> Option<&str> {
        match self {
            Self::PadCenter { pad, .. } | Self::PadCorner { pad, .. } | Self::PadEdge { pad, .. } => Some(pad),
            _ => None,
        }
    }

    /// Only straight offered edges are segment witnesses in v1.
    fn is_edge(&self) -> bool {
        matches!(self, Self::PadEdge { .. } | Self::CourtyardEdge { .. })
    }

    /// Check payload invariants independent of whether the native placement still exists.
    fn valid(&self) -> bool {
        self.footprint().is_none_or(|(fp, def)| {
            (1..=crate::MAX_OBJECT_ID).contains(&fp)
                && def.revision != 0
                && super::definition::valid_id(&def.id)
                && def.library.as_ref().map_or_else(
                    || def.id == super::FOOTPRINT_DEFINITION && def.revision == super::FOOTPRINT_REVISION,
                    |library| super::definition::valid_id(library),
                )
        }) && self.pad().is_none_or(|pad| PinNumber::new(pad).is_some())
            && !matches!(self, Self::CourtyardEdge { rect, .. } if *rect >= 64)
    }
}

impl Kind {
    /// V1 requires an ordered pair from distinct bodies, with a board origin only as first pick.
    fn accepts(self, [a, b]: &[Reference; 2]) -> bool {
        let Some((second, _)) = b.footprint() else { return false };
        if a.footprint().is_some_and(|(first, _)| first == second) {
            return false;
        }
        let edges = u8::from(a.is_edge()) + u8::from(b.is_edge());
        edges
            == match self {
                Self::Gap => 2,
                Self::PointEdge => 1,
                _ => 0,
            }
    }
}

impl Relations {
    /// Normalize only references to a matching placed pin whose new namespace is proved by
    /// the native migration plan. Latched display history is not a live binding; leave it intact.
    pub(super) fn migrate_library_addresses(&mut self, board: &super::Board, replacements: &crate::library::compatibility::migration::Replacements) {
        for reference in self.relations.iter_mut().filter(|row| row.state == State::Ok).flat_map(|row| &mut row.refs) {
            let (fp, def) = match reference {
                Reference::BoardOrigin {} => continue,
                Reference::Origin { fp, def }
                | Reference::PadCenter { fp, def, .. }
                | Reference::PadCorner { fp, def, .. }
                | Reference::PadEdge { fp, def, .. }
                | Reference::CourtyardEdge { fp, def, .. } => (*fp, def),
            };
            if let Some(key) = catalog::footprint(board, fp).and_then(super::Footprint::key)
                && def.library.as_ref() == Some(&key.0)
                && def.id == key.1
                && def.revision == key.2
                && let Some(library) = replacements.footprint_library(&key)
            {
                def.library = Some(library.into());
            }
        }
    }

    /// Count each omitted authored row once if the board selection carries a named footprint
    /// or a native Outline drawing that supplies the board-origin pick. Includes lost/suppressed
    /// rows without resolving them; the counter is bounded by the persisted relation cap.
    pub(super) fn omitted_by_copy(&self, footprints: &BTreeSet<String>, carries_outline_datum: bool) -> usize {
        self.relations
            .iter()
            .filter(|row| {
                row.refs
                    .iter()
                    .any(|reference| reference.footprint().map_or(carries_outline_datum, |(fp, _)| footprints.contains(&format!("footprint-{fp}"))))
            })
            .count()
    }

    /// Strict F1 ingestion: validate saved rows structurally without resolving or settling.
    /// In particular a known-kind future driving row is refused, not silently downgraded.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.relations.is_empty() || self.relations.len() > MAX_RELATIONS {
            return Err(format!("expected 1 through {MAX_RELATIONS} board relations"));
        }
        let mut ids = BTreeSet::new();
        for row in &self.relations {
            if row.driving {
                return Err("driving relations are not supported by this reference-only reader".into());
            }
            if row.id.is_nil()
                || !ids.insert(row.id)
                || row.datum != 0
                || !row.kind.accepts(&row.refs)
                || row.refs.iter().any(|r| !r.valid())
                || row.label.is_some_and(|p| !bounded(p, MAX_CENTER_NM))
                || !row.last_resolved.into_iter().zip(&row.refs).all(|(w, r)| w.valid_for(r))
                || (row.kind == Kind::Gap && !row.last_resolved[0].parallel_segments(row.last_resolved[1]))
                || matches!(row.state, State::Lost { since_revision: 0 })
            {
                return Err("invalid board relation identity, picks, datum, label, state or display witness".into());
            }
            let valid_value = match row.kind {
                Kind::AlignH | Kind::AlignV | Kind::Aligned => row.value.is_none() && row.side.is_none(),
                _ => row.value.is_some_and(|v| (1..=2 * MAX_WITNESS_NM).contains(&v)) && matches!(row.side, Some(-1 | 1)),
            };
            if !valid_value {
                return Err("invalid board relation literal or side".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod capacity_tests;
#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod eco_tests;
#[cfg(test)]
mod locality_tests;
#[cfg(test)]
pub(crate) mod tests;
