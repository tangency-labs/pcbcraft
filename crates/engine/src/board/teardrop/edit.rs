//! Read-only add/remove planning. A rule evaluator is required for every viable addition.

use super::{Error as ShapeError, StraightCopper, ViaTeardrop};
use crate::board::view::{BoardTarget, BoardView};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[cfg(test)]
mod tests;

/// At most two centered via attachments per straight segment, one at each end.
pub const MAX_INTENTS: usize = 2 * crate::board::track::MAX_TRACKS;

/// One proposed edit to a board's straight-via intent list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Add(ViaTeardrop),
    Remove(Uuid),
}

/// A rule that can prevent a copper addition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    CopperClearance,
    BoardEdgeClearance,
    ConflictingContact,
}

/// The first blocking decision in the rule evaluator's stable order.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleBlock {
    pub rule: Rule,
    /// Board-local item reference, not an obstacle index from a prepared query.
    pub other_object: String,
    pub required_nm: i64,
    pub actual_nm: f64,
}

/// Why a proposed site contributes no new copper.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    Shape(ShapeError),
    Rule(RuleBlock),
}

/// A refused request, including the proposed intent ID and both attachment IDs.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    pub intent: ViaTeardrop,
    pub reason: Refusal,
}

/// A checked addition with disposable copper from the same board view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Addition {
    pub intent: ViaTeardrop,
    pub copper: StraightCopper,
}

/// A deterministic diff against the supplied current intents, not permission to publish an edit.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub board: BoardTarget,
    pub added: Vec<Addition>,
    pub removed: Vec<ViaTeardrop>,
    pub rejected: Vec<Rejected>,
}

/// Invalid batch structure or incomplete rules yields no diff, even after some sites passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error<E> {
    TooManyIntents,
    InvalidIdentity,
    WrongBoard(Uuid),
    DuplicateId(Uuid),
    DuplicateSite { via: Uuid, track: Uuid },
    MissingRemoval(Uuid),
    RuleEvaluation { id: Uuid, cause: E },
}

/// Plan changes in ascending intent UUID order, with removals projected before additions.
/// The caller supplies a complete rule evaluator for the same board snapshot. It sees the
/// candidate, retained intents (after removals), and earlier accepted additions, and must
/// return an error if it cannot check the whole site. A site may be skipped for invalid
/// geometry or a known rule block; evaluator errors abort the whole plan. This never mutates
/// the board or records, publishes history, or authorizes applying the resulting diff.
pub fn plan<E>(
    board: BoardView<'_>,
    current: &[ViaTeardrop],
    edits: &[Edit],
    mut check: impl FnMut(&Addition, &[ViaTeardrop], &[Addition]) -> Result<Option<RuleBlock>, E>,
) -> Result<Plan, Error<E>> {
    if current.len() > MAX_INTENTS || edits.len() > 2 * MAX_INTENTS {
        return Err(Error::TooManyIntents);
    }
    let mut retained = BTreeMap::new();
    let mut sites = BTreeSet::new();
    for &record in current {
        check_address(board.target, record)?;
        if retained.insert(record.id, record).is_some() {
            return Err(Error::DuplicateId(record.id));
        }
        remember_site(&mut sites, record)?;
    }
    let mut requests = BTreeMap::new();
    for &edit in edits {
        let id = match edit {
            Edit::Add(record) => {
                check_address(board.target, record)?;
                if retained.contains_key(&record.id) {
                    return Err(Error::DuplicateId(record.id));
                }
                record.id
            }
            Edit::Remove(id) => id,
        };
        if id.is_nil() {
            return Err(Error::InvalidIdentity);
        }
        if requests.insert(id, edit).is_some() {
            return Err(Error::DuplicateId(id));
        }
    }
    let mut result = Plan { board: board.target, added: Vec::new(), removed: Vec::new(), rejected: Vec::new() };
    let mut proposals = Vec::new();
    for (id, edit) in requests {
        match edit {
            Edit::Remove(_) => {
                let record = retained.remove(&id).ok_or(Error::MissingRemoval(id))?;
                sites.remove(&(record.via, record.track));
                result.removed.push(record);
            }
            Edit::Add(record) => proposals.push(record),
        }
    }
    if retained.len() + proposals.len() > MAX_INTENTS {
        return Err(Error::TooManyIntents);
    }
    for &record in &proposals {
        remember_site(&mut sites, record)?;
    }
    let retained: Vec<_> = retained.into_values().collect();
    for intent in proposals {
        match intent.derive(board) {
            Err(reason) => result.rejected.push(Rejected { intent, reason: Refusal::Shape(reason) }),
            Ok(copper) => {
                let candidate = Addition { intent, copper };
                match check(&candidate, &retained, &result.added).map_err(|cause| Error::RuleEvaluation { id: intent.id, cause })? {
                    Some(reason) => result.rejected.push(Rejected { intent, reason: Refusal::Rule(reason) }),
                    None => result.added.push(candidate),
                }
            }
        }
    }
    Ok(result)
}

/// Refuse a foreign board or a nil record ID before indexing the batch.
fn check_address<E>(board: BoardTarget, record: ViaTeardrop) -> Result<(), Error<E>> {
    if record.board != board {
        return Err(Error::WrongBoard(record.id));
    }
    if record.id.is_nil() {
        return Err(Error::InvalidIdentity);
    }
    Ok(())
}

/// One record can own a via/track site after all requested removals.
fn remember_site<E>(sites: &mut BTreeSet<(Uuid, Uuid)>, record: ViaTeardrop) -> Result<(), Error<E>> {
    if !sites.insert((record.via, record.track)) {
        return Err(Error::DuplicateSite { via: record.via, track: record.track });
    }
    Ok(())
}
