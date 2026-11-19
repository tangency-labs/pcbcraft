//! Session-owned authority for bounded, nonpersistent board-update previews.

use super::{Baseline, ComponentChange, PartInstanceState, Preview, SourceCapture, apply_candidate, preview};
use crate::{
    CommandError, Session,
    board::relations::{self, Base, Disclosure, Prepared, RelationInput, ReviewSlot},
};
use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::Arc,
};
use uuid::Uuid;

/// Freeze proposed schematic/package changes without authoring project state.
pub const PREVIEW_UPDATE: &str = "pcb.preview_update";
/// Accept exactly one current opaque preview capability in one undo step.
pub const APPLY_UPDATE: &str = "pcb.apply_update";
/// Revoke the current runtime preview without changing project state or history.
pub const CANCEL_UPDATE: &str = "pcb.cancel_update";
/// Keep a complete preview report below the live transport's reply budget.
pub const MAX_PREVIEW_BYTES: usize = 8 * 1024 * 1024;

/// One session-local capability. It is deliberately absent from project codecs.
#[derive(Debug)]
pub(crate) struct CachedPlan {
    base: Base,
    accepted: Arc<Baseline>,
    proposed: Baseline,
    changes: Preview,
    payload: Payload,
}

#[derive(Debug)]
enum Payload {
    Blocked,
    NoOp,
    LegacyLazy,
    Checked { prepared: Prepared, intent_digest: String },
}

/// Complete bounded review information without duplicating the whole project inspection.
#[derive(Debug, Serialize)]
pub struct BoardUpdatePreview<'a> {
    /// Runtime-only authority; it is never a source object identity.
    pub plan_token: Uuid,
    /// Revision of the exact immutable input retained by the session.
    pub revision: u64,
    /// Persistent project owning this preview.
    pub project: Uuid,
    /// Physical board owning the accepted and proposed inputs.
    pub board: Uuid,
    /// The captured pre-Apply source bundle; persisted only on real acceptance.
    pub source: &'a SourceCapture,
    /// Complete validated changes; a view may group these without creating another plan.
    pub changes: &'a Preview,
    /// True when blockers are empty, including a no-op. Views use `changes.is_noop()`
    /// separately when displaying whether there is anything to accept.
    pub can_apply: bool,
    #[serde(rename = "component_details")]
    details: ComponentDetails<'a>,
    /// Optional complete detail for relation-aware updates (omitted on legacy/no-detail reviews).
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_detail: Option<&'a Disclosure>,
    /// Certificate of the checked candidate, present only when relation-aware preparation occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_digest: Option<&'a str>,
    /// Digest of the typed choices bound to this checked candidate.
    #[serde(skip_serializing_if = "Option::is_none")]
    intent_digest: Option<&'a str>,
}

impl<'a> BoardUpdatePreview<'a> {
    /// Resolve stable part metadata from this captured plan, including parts
    /// removed from the current schematic. It is safe to use for review labels.
    pub fn component(&self, id: u64) -> Option<&'a PartInstanceState> {
        self.details.proposed.get(&id).or_else(|| self.details.accepted.get(&id))
    }

    /// Inspect the previous accepted package, mapping and metadata for an old/new
    /// review row. A newly added part has no accepted predecessor.
    pub fn accepted_component(&self, id: u64) -> Option<&'a PartInstanceState> {
        self.details.accepted.get(&id)
    }

    /// Inspect proposed metadata independently of the accepted predecessor. A
    /// removed part has no proposed record even though `component` labels it.
    pub fn proposed_component(&self, id: u64) -> Option<&'a PartInstanceState> {
        self.details.proposed.get(&id)
    }
}

/// Serialize old/new metadata only for changed parts, without copying whole maps
/// into a native view or repeating unchanged parts in every MCP response.
#[derive(Debug)]
struct ComponentDetails<'a> {
    accepted: &'a BTreeMap<u64, PartInstanceState>,
    proposed: &'a BTreeMap<u64, PartInstanceState>,
    changes: &'a [ComponentChange],
}

impl Serialize for ComponentDetails<'_> {
    /// Deduplicate component-level changes and retain their stable namespace in keys.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Detail<'a> {
            accepted: Option<&'a PartInstanceState>,
            proposed: Option<&'a PartInstanceState>,
        }
        let ids: BTreeSet<_> = self
            .changes
            .iter()
            .map(|change| match change {
                ComponentChange::Added { component }
                | ComponentChange::Removed { component }
                | ComponentChange::Definition { component }
                | ComponentChange::Package { component }
                | ComponentChange::Mapping { component } => *component,
            })
            .collect();
        let mut map = serializer.serialize_map(Some(ids.len()))?;
        for id in ids {
            map.serialize_entry(&format!("component-{id}"), &Detail { accepted: self.accepted.get(&id), proposed: self.proposed.get(&id) })?;
        }
        map.end()
    }
}

impl CachedPlan {
    /// Require the exact immutable opening/input, not merely coincident UUIDs or revisions.
    fn matches(&self, session: &Session) -> bool {
        self.base.matches(session)
    }

    /// Borrow the complete report for size checking or explicit preview inspection.
    fn report(&self) -> BoardUpdatePreview<'_> {
        let (relation_detail, candidate_digest, intent_digest) = match &self.payload {
            Payload::Checked { prepared, intent_digest } => {
                ((!prepared.disclosure.is_empty()).then_some(&prepared.disclosure), prepared.digest.as_deref(), Some(intent_digest.as_str()))
            }
            _ => (None, None, None),
        };
        BoardUpdatePreview {
            plan_token: self.base.token,
            revision: self.base.revision,
            project: self.proposed.project,
            board: self.proposed.board,
            source: &self.proposed.source,
            changes: &self.changes,
            can_apply: self.changes.blockers.is_empty(),
            details: ComponentDetails { proposed: &self.proposed.components, accepted: &self.accepted.components, changes: &self.changes.components },
            relation_detail,
            candidate_digest,
            intent_digest,
        }
    }
}

impl Session {
    /// Borrow only the current ECO tag of the common review slot.
    fn eco_plan(&self) -> Option<&CachedPlan> {
        match &self.update_plan {
            Some(ReviewSlot::Eco(plan)) if plan.matches(self) => Some(plan),
            _ => None,
        }
    }

    /// Borrow a current typed review for native UI. The returned view cannot
    /// outlive its session borrow; Apply still validates the opaque token again.
    pub fn cached_board_update(&self) -> Option<BoardUpdatePreview<'_>> {
        self.eco_plan().map(CachedPlan::report)
    }

    /// Inspect the current bounded review report. Cached data is runtime-only;
    /// stale plans are never presented as available approval authority.
    pub fn board_update_report(&self) -> Option<Value> {
        serde_json::to_value(self.cached_board_update()?).ok()
    }

    /// Publish a small preview summary in ordinary inspection, avoiding repeated
    /// full net membership responses after unrelated inspect operations.
    pub fn board_update_summary(&self) -> Option<Value> {
        let plan = self.eco_plan()?;
        Some(json!({"plan_token":plan.base.token,"board":plan.proposed.board,"revision":plan.base.revision,
            "component_changes":plan.changes.components.len(),"removed_nets":plan.changes.removed_nets.len(),
            "added_nets":plan.changes.added_nets.len(),"blockers":plan.changes.blockers,"can_apply":plan.changes.blockers.is_empty()}))
    }

    /// Replace the one runtime preview only after its immutable input, complete
    /// diff, report bound and fresh capability have all been validated.
    pub(crate) fn preview_board_update(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let target = snapshot.resolve_board(params.get("board").and_then(Value::as_str))?;
        let choices = relations::choices(params.get("remap_choices"))?;
        let source = super::capture_source(&snapshot)?;
        let accepted = if let Some(baseline) = &snapshot.board.baseline {
            Arc::clone(baseline)
        } else if snapshot.board.footprints().is_empty() {
            let target = snapshot.board_target().ok_or_else(|| CommandError::Unavailable("no physical board is available".into()))?;
            Arc::new(Baseline::empty(target.project, target.board, source.clone()).map_err(CommandError::InvalidParams)?)
        } else {
            return Err(CommandError::Unavailable(
                "Initialize the existing board's accepted baseline before previewing changes; its earlier accepted history is unavailable.".into(),
            ));
        };
        let proposed = Baseline::proposed(&snapshot, &Baseline::current_choices(&snapshot), source).map_err(CommandError::InvalidParams)?;
        let changes = preview(&accepted, &proposed).map_err(CommandError::InvalidParams)?;
        let payload = if !changes.blockers.is_empty() {
            Payload::Blocked
        } else if changes.is_noop() {
            if !choices.is_empty() {
                return Err(CommandError::InvalidParams("a no-op board update cannot stage remaps".into()));
            }
            Payload::NoOp
        } else if snapshot.board.relations.is_none() && choices.is_empty() {
            Payload::LegacyLazy
        } else {
            use sha2::{Digest, Sha256};
            let intent = serde_json::to_vec(&json!({"project":target.project,"board":target.board,"source":proposed.source,"choices":choices}))
                .map_err(relations::commands::invalid)?;
            let (next, _) = apply_candidate(&snapshot, &accepted, &proposed).map_err(CommandError::InvalidParams)?;
            let prepared = self.prepare_with(next, &RelationInput { review: true, choices })?;
            Payload::Checked { prepared, intent_digest: format!("{:x}", Sha256::digest(intent)) }
        };
        let plan = CachedPlan { base: Base::new(self, snapshot, target)?, accepted, proposed, changes, payload };
        #[derive(Serialize)]
        struct Envelope<'a> {
            revision: u64,
            board_update_preview: BoardUpdatePreview<'a>,
        }
        serde_json::to_writer(ReportSize { bytes: 0 }, &Envelope { revision: self.revision, board_update_preview: plan.report() })
            .map_err(|error| CommandError::Unavailable(format!("board update preview is too large to review: {error}")))?;
        self.update_plan = Some(ReviewSlot::Eco(plan));
        Ok(())
    }

    /// Validate the exact review and both consent sets. Checked Apply publishes only its
    /// retained candidate; only LegacyLazy derives a physical candidate on Apply.
    pub(crate) fn apply_board_update(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let token = plan_token(params)?;
        let target = self.open_project()?.resolve_board(params.get("board").and_then(Value::as_str))?;
        let Some(ReviewSlot::Eco(plan)) = &self.update_plan else {
            return Err(CommandError::Unavailable("unknown or replaced board update preview; preview again".into()));
        };
        if plan.base.token != token {
            return Err(CommandError::Unavailable("unknown or replaced board update preview; preview again".into()));
        }
        if !plan.matches(self) || plan.base.target != target {
            return Err(CommandError::Unavailable("board update preview is stale; preview again".into()));
        }
        match &plan.payload {
            Payload::Blocked => return Err(CommandError::Unavailable("resolve the preview's package blockers and preview again".into())),
            Payload::Checked { prepared, .. } => {
                prepared.disclosure.consent(params, true)?;
                let Some(ReviewSlot::Eco(CachedPlan { payload: Payload::Checked { prepared, .. }, .. })) = self.update_plan.take() else {
                    unreachable!("validated checked ECO slot")
                };
                self.publish(prepared);
            }
            Payload::LegacyLazy => {
                Disclosure::default().consent(params, true)?;
                if plan.base.snapshot.board.relations.is_some() {
                    return Err(relations::commands::stale());
                }
                let (next, _) = apply_candidate(&plan.base.snapshot, &plan.accepted, &plan.proposed).map_err(CommandError::InvalidParams)?;
                crate::persistence::encode_documents(Arc::new(next.clone())).map_err(CommandError::InvalidParams)?;
                self.change(next)?;
            }
            Payload::NoOp => {
                Disclosure::default().consent(params, true)?;
                self.update_plan = None;
            }
        }
        Ok(())
    }

    /// Revoke only the named current capability. Cancel cannot delete another
    /// client's newer preview or make an authored edit appear undone.
    pub(crate) fn cancel_board_update(&mut self, params: &Value) -> Result<(), CommandError> {
        let token = plan_token(params)?;
        if !self.eco_plan().is_some_and(|plan| plan.base.token == token) {
            return Err(CommandError::Unavailable("unknown or replaced board update preview".into()));
        }
        self.update_plan = None;
        Ok(())
    }
}

/// Parse the opaque runtime capability without accepting project/object IDs as authority.
fn plan_token(params: &Value) -> Result<Uuid, CommandError> {
    params["plan_token"]
        .as_str()
        .and_then(|token| Uuid::parse_str(token).ok())
        .filter(|token| !token.is_nil())
        .ok_or_else(|| CommandError::InvalidParams("invalid board update plan token".into()))
}

/// Count serialized output without retaining an oversized intermediate report.
struct ReportSize {
    bytes: usize,
}
impl Write for ReportSize {
    /// Stop before a preview could exceed the bounded complete-review response.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_PREVIEW_BYTES)
            .ok_or_else(|| io::Error::other("complete preview exceeds 8 MiB"))?;
        Ok(bytes.len())
    }
    /// The size-only sink has no buffered output to publish.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
