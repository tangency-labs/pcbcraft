//! Reference-side preparation and immutable review authority. Stage 1 has no
//! placement outputs; stage 3 may update relation records, never native geometry.

use super::{
    Kind, MAX_WITNESS_NM, Reference, State, Witness,
    catalog::{self, Measure},
};
use crate::{CommandError, Project, Session, board::view::BoardTarget};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RelationStage {
    #[default]
    Outer,
    Raw,
}

#[derive(Debug, Default)]
pub(crate) struct RelationInput {
    pub(crate) review: bool,
    pub(crate) choices: Vec<Choice>,
    pub(crate) explicit_poses: BTreeSet<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Choice {
    relation: Uuid,
    pick: u8,
    choice: RemapChoice,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RemapChoice {
    Correspond,
    KeepLost,
    Repick { reference: Reference },
}

#[derive(Debug, Serialize)]
struct Loss {
    id: Uuid,
    refs: [Reference; 2],
    last_resolved: [Witness; 2],
}

#[derive(Debug, Serialize)]
struct Remap {
    id: Uuid,
    relation: Uuid,
    pick: u8,
    old: Reference,
    new: Reference,
    before: Option<Witness>,
    after: Witness,
    #[serde(skip_serializing_if = "Option::is_none")]
    binding_change: Option<catalog::PadBindingChange>,
}

#[derive(Debug, Serialize)]
struct Pose {
    fp: u64,
    x_nm: i64,
    y_nm: i64,
    rotation: u16,
    bottom: bool,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Disclosure {
    losses: Vec<Loss>,
    remaps: Vec<Remap>,
    poses: Vec<Pose>,
    #[serde(skip)]
    pub(crate) affected: bool,
}

impl Disclosure {
    /// Do not add optional relation details to an empty native ECO report.
    pub(crate) fn is_empty(&self) -> bool {
        self.losses.is_empty() && self.remaps.is_empty() && self.poses.is_empty()
    }

    /// Nonempty decisions must use two independently exact, duplicate-free ID sets.
    /// Empty ECO consent may be omitted for legacy callers; generic consent cannot.
    pub(crate) fn consent(&self, params: &Value, legacy: bool) -> Result<(), CommandError> {
        let may_omit = legacy && self.losses.is_empty() && self.remaps.is_empty();
        check_set(params.get("accept_loss"), self.losses.iter().map(|r| r.id), may_omit)?;
        check_set(params.get("confirm_remaps"), self.remaps.iter().map(|r| r.id), may_omit)
    }

    /// Direct callers never get a token and cannot resubmit bare approvals.
    pub(crate) fn require_no_decisions(&self) -> Result<(), CommandError> {
        if !self.losses.is_empty() {
            return Err(reason("would_lose", "This edit would lose relation references. Preview it to review the loss.", json!(self.losses)));
        }
        if !self.remaps.is_empty() {
            return Err(reason(
                "remap_required",
                "This edit changes pinned references. Preview it to review the correspondence.",
                json!(self.remaps),
            ));
        }
        Ok(())
    }
}

/// Check equality before allowing the caller to take the capability from the slot.
fn check_set(input: Option<&Value>, expected: impl Iterator<Item = Uuid>, may_omit: bool) -> Result<(), CommandError> {
    if input.is_none() && may_omit {
        return Ok(());
    }
    let values: Vec<Uuid> = serde_json::from_value(input.cloned().unwrap_or(Value::Null)).map_err(crate::board::relations::commands::invalid)?;
    let given: BTreeSet<_> = values.iter().copied().collect();
    if given.len() != values.len() || given != expected.collect() {
        return Err(CommandError::InvalidParams("decisions must exactly match the disclosed IDs independently, without duplicates".into()));
    }
    Ok(())
}

/// A structured engine refusal; transport can keep its ordinary Unavailable class.
pub(crate) fn reason(reason: &'static str, message: &str, rows: Value) -> CommandError {
    CommandError::RelationRefused { reason, message: message.to_owned(), rows }
}

/// One exact checked native candidate. It can only be produced by `prepare_change`.
#[derive(Debug)]
pub(crate) struct Prepared {
    pub(crate) snapshot: Arc<Project>,
    pub(crate) highest: u64,
    pub(crate) digest: Option<String>,
    pub(crate) disclosure: Disclosure,
}

/// Stable, unambiguous digest of the exact canonical native document set.
pub(crate) fn certify(snapshot: Arc<Project>) -> Result<String, CommandError> {
    #[cfg(test)]
    super::probes::hit(super::probes::Op::CandidateEncode);
    let documents = crate::persistence::encode_documents(snapshot).map_err(crate::board::relations::commands::invalid)?;
    #[cfg(test)]
    super::probes::hit(super::probes::Op::CandidateDigest);
    let mut digest = Sha256::new();
    digest.update(b"pcbcraft-relation-candidate\0");
    for (path, bytes) in documents {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Bind an issued capability to the current session identity, opening, revision and exact base.
#[derive(Debug)]
pub(crate) struct Base {
    pub(crate) token: Uuid,
    pub(crate) revision: u64,
    pub(crate) snapshot: Arc<Project>,
    pub(crate) target: BoardTarget,
    opening: u64,
    session: Arc<()>,
}

impl Base {
    /// Freeze the normalized target and opening of this preparation, not a caller's alias.
    pub(crate) fn new(session: &Session, snapshot: Arc<Project>, target: BoardTarget) -> Result<Self, CommandError> {
        Ok(Self {
            token: crate::fresh_id()?,
            revision: session.revision,
            opening: session.history_root_revision,
            session: session.clearance_identity.clone(),
            snapshot,
            target,
        })
    }

    /// An unchanged numeric revision is insufficient if saved metadata replaced the Arc.
    pub(crate) fn matches(&self, session: &Session) -> bool {
        self.revision == session.revision
            && self.opening == session.history_root_revision
            && Arc::ptr_eq(&self.session, &session.clearance_identity)
            && session.project.as_ref().is_some_and(|p| Arc::ptr_eq(p, &self.snapshot))
    }
}

#[derive(Debug)]
pub(crate) enum ReviewSlot {
    GenericPrepared(super::commands::Generic),
    Eco(crate::board::update::CachedPlan),
}

/// Canonicalize the explicit choices and reject ambiguity before touching a private candidate.
pub(crate) fn choices(input: Option<&Value>) -> Result<Vec<Choice>, CommandError> {
    let Some(input) = input else { return Ok(Vec::new()) };
    let mut values: Vec<Choice> = serde_json::from_value(input.clone()).map_err(crate::board::relations::commands::invalid)?;
    if values.len() > 2 * super::MAX_RELATIONS
        || values.iter().any(|c| c.relation.is_nil() || c.pick > 1 || matches!(&c.choice, RemapChoice::Repick { reference } if !reference.valid()))
    {
        return Err(CommandError::InvalidParams("invalid or excessive remap choices".into()));
    }
    values.sort_by_key(|c| (c.relation, c.pick));
    if values.windows(2).any(|c| (c[0].relation, c[0].pick) == (c[1].relation, c[1].pick)) {
        return Err(CommandError::InvalidParams("duplicate remap choice".into()));
    }
    Ok(values)
}

/// Only new/re-picked relations infer a sided literal; visible reference remeasurement never
/// changes the user-authored ordering or chooses a different side.
pub(super) fn pick_literal(kind: Kind, measure: Measure) -> Result<(Option<i64>, Option<i8>), CommandError> {
    let error = || CommandError::InvalidParams("Use Align for zero; choose a nonzero magnitude for a distance.".into());
    match (kind, measure) {
        (Kind::AlignH | Kind::AlignV, Measure::Projection { signed_nm: 0, .. }) => Ok((None, None)),
        (Kind::AlignH | Kind::AlignV, _) => Err(CommandError::InvalidParams("Align requires a zero pick projection.".into())),
        (Kind::Aligned, Measure::Aligned { distance_nm, .. }) if distance_nm > 0. => Ok((None, None)),
        (_, Measure::Projection { signed_nm, .. }) if signed_nm != 0 && signed_nm.abs() <= 2 * MAX_WITNESS_NM => {
            Ok((Some(signed_nm.abs()), Some(signed_nm.signum() as i8)))
        }
        _ => Err(error()),
    }
}

/// Stage losses and exact remaps after native reconciliation. A same-address re-pick
/// requires changed pad binding and its own disclosed remap. Output geometry stays native;
/// historical witnesses can only flow to lost display.
pub(crate) fn verify_final(
    previous: Option<&Project>,
    candidate: &mut Project,
    revision: u64,
    input: &RelationInput,
) -> Result<Disclosure, CommandError> {
    #[cfg(test)]
    super::probes::hit(super::probes::Op::VerifyReferences);
    let previous = previous.filter(|p| p.same_document(candidate));
    let old = previous.and_then(|p| p.board.relations.as_deref());
    let next = candidate.board.relations.as_deref();
    let mut report = Disclosure::default();
    if old.is_none() && next.is_none() {
        if !input.choices.is_empty() {
            return Err(CommandError::InvalidParams("no relation can use these remap choices".into()));
        }
        if input.review {
            report.poses = resulting_poses(previous, candidate, input, BTreeSet::new());
        }
        return Ok(report);
    }
    let mut by_id: BTreeMap<_, _> = old.into_iter().flat_map(|rows| &rows.relations).map(|row| (row.id, row)).collect();
    let mut choices: BTreeMap<_, _> = input.choices.iter().map(|c| ((c.relation, c.pick), &c.choice)).collect();
    let mut out = next.cloned();
    report.affected = old != next;
    let mut affected = BTreeSet::new();
    let mut bodies = BTreeSet::new();
    let prior_resolver = previous.map(catalog::Resolver::new);
    let final_resolver = catalog::Resolver::new(candidate);
    for row in out.iter_mut().flat_map(|rows| &mut rows.relations) {
        let prior = by_id.remove(&row.id);
        if row.state != State::Ok {
            continue;
        }
        let mut refs = row.refs.clone();
        let mut proposal = Vec::new();
        let mut loses = false;
        let mut explicit_model = false;
        let mut repick = false;
        if let Some((base, prior)) = previous.zip(prior).filter(|(_, prior)| prior.refs == row.refs && prior.state == State::Ok) {
            for (pick, old_ref) in prior.refs.iter().enumerate() {
                let offered = catalog::correspondence(base, &final_resolver, old_ref);
                let choice = choices.remove(&(row.id, pick as u8));
                repick |= matches!(choice, Some(RemapChoice::Repick { .. }));
                explicit_model |= matches!(choice, Some(RemapChoice::Correspond | RemapChoice::Repick { .. }));
                let mut binding_change = None;
                let proposed = match (offered, choice) {
                    (None, Some(RemapChoice::Correspond)) => {
                        return Err(CommandError::InvalidParams("that correspondence is no longer offered".into()));
                    }
                    // Automatic address equality is not pair compatibility. An explicit re-pick
                    // may fix the ordered pair on the candidate even when this old pick resolves.
                    (Some(new), Some(RemapChoice::Correspond | RemapChoice::KeepLost)) if &new == old_ref => {
                        return Err(CommandError::InvalidParams("remap choice does not name a changed reference".into()));
                    }
                    (_, Some(RemapChoice::KeepLost)) => None,
                    (_, Some(RemapChoice::Repick { reference })) => {
                        if reference == old_ref {
                            binding_change = catalog::same_address_pad_binding_change(base, &final_resolver, old_ref);
                            if binding_change.is_none() {
                                return Err(CommandError::InvalidParams("re-picking the same reference requires a changed pad binding".into()));
                            }
                        }
                        Some(reference.clone())
                    }
                    (offered, _) => offered,
                };
                let Some(proposed) = proposed else {
                    loses = true;
                    continue;
                };
                if &proposed != old_ref || binding_change.is_some() {
                    let after = catalog::resolve(&final_resolver, &proposed).ok_or_else(|| {
                        let message = if matches!(choice, Some(RemapChoice::Repick { .. })) {
                            "The replacement pick is not available."
                        } else {
                            "The proposed reference is not available."
                        };
                        reason("lost_reference", message, Value::Null)
                    })?;
                    proposal.push(Remap {
                        id: crate::fresh_id()?,
                        relation: row.id,
                        pick: pick as u8,
                        old: old_ref.clone(),
                        new: proposed.clone(),
                        before: prior_resolver.as_ref().and_then(|resolver| catalog::resolve(resolver, old_ref)),
                        after,
                        binding_change,
                    });
                }
                refs[pick] = proposed;
            }
        }
        let measured = (!loses).then(|| catalog::measure(&final_resolver, row.kind, &refs)).flatten();
        if let Some(measurement) = measured {
            // Only an explicit re-pick authors the final ordered pair anew. Native motion
            // and correspondence may change the live measure without changing its literal.
            if repick {
                (row.value, row.side) = pick_literal(row.kind, measurement)?;
            }
            row.refs = refs;
            row.last_resolved = measurement.witnesses();
            if !proposal.is_empty() {
                report.remaps.extend(proposal);
                affected.extend(row.refs.iter().filter_map(|r| r.footprint().map(|(id, _)| id)));
            }
            let changed_body = previous.is_some_and(|base| {
                row.refs
                    .iter()
                    .filter_map(Reference::footprint)
                    .any(|(id, _)| catalog::footprint(&base.board, id) != catalog::footprint(&candidate.board, id))
            });
            if prior != Some(&*row)
                || changed_body
                || prior_resolver.as_ref().zip(prior).and_then(|(resolver, row)| catalog::live_measure(resolver, row)) != Some(measurement)
            {
                report.affected = true;
                affected.extend(row.refs.iter().filter_map(|r| r.footprint().map(|(id, _)| id)));
            }
        } else if let Some(prior) = prior.filter(|p| p.refs == row.refs && p.state == State::Ok) {
            if explicit_model {
                return Err(reason(
                    "lost_reference",
                    "The selected repick or correspondence does not make a measurable pair. Review both picks.",
                    json!({"id":row.id}),
                ));
            }
            row.state = State::Lost { since_revision: revision };
            if let Some(old_witnesses) = prior_resolver.as_ref().and_then(|resolver| catalog::live_measure(resolver, prior)).map(Measure::witnesses) {
                row.last_resolved = old_witnesses;
            }
            report.affected = true;
            report.losses.push(Loss { id: row.id, refs: row.refs.clone(), last_resolved: row.last_resolved });
            affected.extend(row.refs.iter().filter_map(|r| r.footprint().map(|(id, _)| id)));
        } else {
            return Err(reason("lost_reference", "Pick two compatible features that resolve on the board.", Value::Null));
        }
    }
    if !choices.is_empty() {
        return Err(CommandError::InvalidParams("some remap choices do not belong to this candidate".into()));
    }
    for row in old.into_iter().chain(out.as_ref()).flat_map(|rows| &rows.relations) {
        bodies.extend(row.refs.iter().filter_map(|r| r.footprint().map(|(id, _)| format!("footprint-{id}"))));
    }
    for row in by_id.into_values() {
        affected.extend(row.refs.iter().filter_map(|r| r.footprint().map(|(id, _)| id)));
    }
    if let Some(rows) = &out {
        rows.validate().map_err(crate::board::relations::commands::invalid)?;
    }
    if next != out.as_ref() {
        Arc::make_mut(&mut candidate.board).relations = out.map(Arc::new);
    }
    if let Some(base) = previous {
        super::islands::check(base, candidate, &bodies)?;
    }
    if input.review {
        report.poses = resulting_poses(previous, candidate, input, affected);
    }
    Ok(report)
}

/// Disclose the union of surviving affected reference bodies, native placement deltas
/// (including additions and ECO package changes), and validated native pose selections.
/// Retired bodies have no resulting pose. Native placement order is stable by ID.
fn resulting_poses(previous: Option<&Project>, candidate: &Project, input: &RelationInput, affected: BTreeSet<u64>) -> Vec<Pose> {
    candidate
        .board
        .footprints()
        .iter()
        .filter(|f| {
            affected.contains(&f.id)
                || input.explicit_poses.contains(&f.id)
                || previous.and_then(|base| catalog::footprint(&base.board, f.id)) != Some(*f)
        })
        .map(|f| Pose { fp: f.id, x_nm: f.x_nm, y_nm: f.y_nm, rotation: f.rotation, bottom: f.bottom })
        .collect()
}

/// Validate the complete engine envelope before installing a replacement or publishing a result.
/// The later host may impose a smaller budget including its own framing/inspection.
pub(crate) fn bounded_result(value: Value, limit: usize) -> Result<Value, CommandError> {
    check_result(&value, limit)?;
    Ok(value)
}

/// Size a borrowed typed result, including escaping and metadata, without allocating wire bytes.
pub(super) fn check_result(value: &impl Serialize, limit: usize) -> Result<(), CommandError> {
    let mut remaining = limit;
    serde_json::to_writer(Budget(&mut remaining), value)
        .map_err(|error| CommandError::CapExceeded(format!("complete relation command result is too large: {error}")))
}

struct Budget<'a>(&'a mut usize);
impl std::io::Write for Budget<'_> {
    /// Subtract actual serialized bytes so there is no size estimate or truncated success.
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        *self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| std::io::Error::other("reply byte budget exceeded"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
