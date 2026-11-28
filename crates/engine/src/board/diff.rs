//! Read-only comparison of UUID-addressed objects in two revisions of one physical board.

use super::{
    drawing::Drawing,
    track::{Track, Via},
    view::{BoardTarget, BoardView},
    zones::Zone,
};
use crate::Project;
use serde::Serialize;
use serde_json::{Map, Value};
use std::{cmp::Ordering, collections::BTreeMap, fmt};
use uuid::Uuid;

/// Maximum number of detailed changes returned by one comparison. Totals are always complete.
pub const MAX_REPORTED_CHANGES: usize = 256;

/// Supported authored object kinds. Variant order breaks UUID ties between different kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Track,
    Via,
    Drawing,
    Zone,
}

/// Counts for the entire comparison, including changes that did not fit in the report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ChangeCounts {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

/// An authored object's differing top-level property. An omitted side means the property
/// was absent from that record, not a null value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PropertyChange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

/// A difference between two authored records. Property maps do not repeat the object's ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum Change {
    Added { after: Map<String, Value> },
    Removed { before: Map<String, Value> },
    Changed { properties: BTreeMap<String, PropertyChange> },
}

/// One change identified by the authored UUID and its object kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ObjectChange {
    pub id: Uuid,
    pub kind: ObjectKind,
    #[serde(flatten)]
    pub change: Change,
}

/// A deterministic prefix of the object changes, ordered by UUID bytes and then kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardDiff {
    pub board: BoardTarget,
    pub counts: ChangeCounts,
    pub truncated: bool,
    pub changes: Vec<ObjectChange>,
}

/// A comparison can fail before giving the caller any report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardDiffError {
    LimitExceeded,
    MissingBeforeBoard,
    MissingAfterBoard,
    PropertyEncoding,
    /// Either revision stores circular copper arcs, which this comparison does not cover.
    UnsupportedArcs,
    /// Either revision stores via teardrop records, which this comparison does not cover.
    UnsupportedTeardrops,
}

impl fmt::Display for BoardDiffError {
    /// Explain why no report was returned.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded => write!(f, "at most {MAX_REPORTED_CHANGES} detailed changes may be requested"),
            Self::MissingBeforeBoard => f.write_str("the requested board is absent from the before revision"),
            Self::MissingAfterBoard => f.write_str("the requested board is absent from the after revision"),
            Self::PropertyEncoding => f.write_str("could not encode authored board object properties"),
            Self::UnsupportedArcs => f.write_str(super::arc::UNSUPPORTED),
            Self::UnsupportedTeardrops => f.write_str(super::teardrop::UNSUPPORTED),
        }
    }
}

impl std::error::Error for BoardDiffError {}

/// Compare immutable revisions of the same explicitly addressed board. The limit caps detailed
/// rows, not counts; zero requests counts only. Neither revision is changed. Only authored
/// tracks, vias, drawings and zone outlines are included. The retained legacy rectangle uses its
/// drawing identity; there is no geometry or net matching.
/// Refuses limits above [`MAX_REPORTED_CHANGES`], a target missing from either revision, and
/// boards with circular arc tracks or via teardrops, rather than reporting such an edit as no change.
pub fn compare(before: &Project, after: &Project, board: BoardTarget, limit: usize) -> Result<BoardDiff, BoardDiffError> {
    if limit > MAX_REPORTED_CHANGES {
        return Err(BoardDiffError::LimitExceeded);
    }
    let before = before.board_view(board).ok_or(BoardDiffError::MissingBeforeBoard)?;
    let after = after.board_view(board).ok_or(BoardDiffError::MissingAfterBoard)?;
    require_linear(before, after)?;
    let legacy_before = before.legacy_outline_drawing();
    let legacy_after = after.legacy_outline_drawing();
    let mut before = objects(before, &legacy_before).peekable();
    let mut after = objects(after, &legacy_after).peekable();
    let mut counts = ChangeCounts::default();
    let mut changes = Vec::new();

    loop {
        let (old, new) = match (before.peek(), after.peek()) {
            (None, None) => break,
            (Some(a), Some(b)) => match a.key().cmp(&b.key()) {
                Ordering::Less => (before.next(), None),
                Ordering::Greater => (None, after.next()),
                Ordering::Equal => (before.next(), after.next()),
            },
            (Some(_), None) => (before.next(), None),
            (None, Some(_)) => (None, after.next()),
        };
        let keep = changes.len() < limit;
        let (object, change) = match (old, new) {
            (None, Some(new)) => {
                counts.added += 1;
                (new, keep.then(|| new.properties().map(|after| Change::Added { after })).transpose()?)
            }
            (Some(old), None) => {
                counts.removed += 1;
                (old, keep.then(|| old.properties().map(|before| Change::Removed { before })).transpose()?)
            }
            (Some(old), Some(new)) if old != new => {
                counts.changed += 1;
                let change = if keep { Some(Change::Changed { properties: changed_properties(old.properties()?, new.properties()?) }) } else { None };
                (new, change)
            }
            _ => continue,
        };
        if let Some(change) = change {
            let (id, kind) = object.key();
            changes.push(ObjectChange { id, kind, change });
        }
    }
    let truncated = counts.added + counts.removed + counts.changed > changes.len();
    Ok(BoardDiff { board, counts, truncated, changes })
}

/// Refuse a pair in which either revision stores arcs or teardrops, so no such edit is omitted.
/// Each record kind keeps its own error rather than reporting a teardrop as an arc.
pub(crate) fn require_linear(before: BoardView, after: BoardView) -> Result<(), BoardDiffError> {
    if !before.arc_tracks().is_empty() || !after.arc_tracks().is_empty() {
        return Err(BoardDiffError::UnsupportedArcs);
    }
    if !before.teardrops().is_empty() || !after.teardrops().is_empty() {
        return Err(BoardDiffError::UnsupportedTeardrops);
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Object<'a> {
    Track(&'a Track),
    Via(&'a Via),
    Drawing(&'a Drawing),
    Zone(&'a Zone),
}

impl Object<'_> {
    /// Match by authored UUID within each kind; legacy typed namespaces need not share one ID space.
    fn key(self) -> (Uuid, ObjectKind) {
        match self {
            Self::Track(track) => (track.id, ObjectKind::Track),
            Self::Via(via) => (via.id, ObjectKind::Via),
            Self::Drawing(drawing) => (drawing.id, ObjectKind::Drawing),
            Self::Zone(zone) => (zone.id, ObjectKind::Zone),
        }
    }

    /// Project only the retained authored record, without identity or resolved display properties.
    fn properties(self) -> Result<Map<String, Value>, BoardDiffError> {
        let value = match self {
            Self::Track(track) => serde_json::to_value(track),
            Self::Via(via) => serde_json::to_value(via),
            Self::Drawing(drawing) => serde_json::to_value(drawing),
            Self::Zone(zone) => serde_json::to_value(zone),
        };
        let Ok(Value::Object(mut properties)) = value else { return Err(BoardDiffError::PropertyEncoding) };
        properties.remove("id");
        // Native storage encodes these marks separately from the via record. Include them
        // here because Via equality includes them; an unmarked via has no such property.
        if let Self::Via(via) = self
            && !via.test_points.is_empty()
        {
            let flags = serde_json::to_value(via.test_points).map_err(|_| BoardDiffError::PropertyEncoding)?;
            properties.insert("test_point".into(), flags);
        }
        Ok(properties)
    }
}

/// Merge the model's ID-sorted authored lists without allocating a board-wide index. Borrow the
/// stored drawing list, or the single projected rectangle for an older board.
fn objects<'a>(view: BoardView<'a>, legacy: &'a Option<Drawing>) -> impl Iterator<Item = Object<'a>> {
    let mut tracks = view.tracks().iter().map(Object::Track).peekable();
    let mut vias = view.vias().iter().map(Object::Via).peekable();
    let drawings = view.project().board.drawings.as_ref().map_or(legacy.as_slice(), |drawings| drawings.items.as_slice());
    let mut drawings = drawings.iter().map(Object::Drawing).peekable();
    let mut zones = view.zones().iter().map(Object::Zone).peekable();
    std::iter::from_fn(move || {
        let (_, kind) = [tracks.peek(), vias.peek(), drawings.peek(), zones.peek()].into_iter().flatten().map(|object| object.key()).min()?;
        match kind {
            ObjectKind::Track => tracks.next(),
            ObjectKind::Via => vias.next(),
            ObjectKind::Drawing => drawings.next(),
            ObjectKind::Zone => zones.next(),
        }
    })
}

/// Report only changed properties; keep missing and present values distinguishable in JSON.
fn changed_properties(before: Map<String, Value>, mut after: Map<String, Value>) -> BTreeMap<String, PropertyChange> {
    let mut properties = BTreeMap::new();
    for (name, value) in before {
        let next = after.remove(&name);
        if next.as_ref() != Some(&value) {
            properties.insert(name, PropertyChange { before: Some(value), after: next });
        }
    }
    properties.extend(after.into_iter().map(|(name, value)| (name, PropertyChange { before: None, after: Some(value) })));
    properties
}

#[cfg(test)]
mod tests;
