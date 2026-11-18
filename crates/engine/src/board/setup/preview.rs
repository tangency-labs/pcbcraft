//! Disposable review of a complete setup candidate through the ordinary setup validator.

use super::{BoardSetup, Layer};
use crate::{CommandError, Project, Session, board::view::CopperLayers};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::Arc,
};
use uuid::Uuid;

/// Review a stack edit without changing authored state or history.
pub const PREVIEW_BOARD_SETUP: &str = "pcb.preview_board_setup";
/// Revoke the named session-local setup review without an authored edit.
pub const CANCEL_BOARD_SETUP_PREVIEW: &str = "pcb.cancel_board_setup_preview";
const MAX_REPORT_BYTES: usize = 8 * 1024 * 1024;
// Bound the complete changes list independently of report metadata and refusal text.
const MAX_CHANGES_BYTES: usize = 2 * 1024 * 1024;

/// Changes to stable layer identities and physical copper positions.
#[derive(Debug, Serialize)]
pub struct SetupChanges {
    pub removed_layers: Vec<Layer>,
    pub added_layers: Vec<Layer>,
    pub renumbered_layers: Vec<LayerPosition>,
    pub affected_objects: Vec<AffectedObject>,
    pub affected_groups: Vec<super::groups::UserLayerGroup>,
}

/// A surviving copper layer whose one-based physical position changes.
#[derive(Debug, Serialize)]
pub struct LayerPosition {
    pub id: Uuid,
    pub before: usize,
    pub after: usize,
}

/// An object affected by a layer removal, role change or copper-span change.
#[derive(Debug, Serialize)]
pub struct AffectedObject {
    pub kind: &'static str,
    pub id: String,
    /// Authored layer or ordered copper span before the proposed change.
    pub layers: Vec<Uuid>,
}

/// The real command's short refusal; machine-readable changes live separately in the report.
#[derive(Debug, Serialize)]
pub struct SetupRefusal {
    pub kind: &'static str,
    pub message: String,
}

/// Complete review shared by native callers and MCP; never persisted in the project.
#[derive(Debug, Serialize)]
pub struct SetupPreview {
    pub plan_token: Uuid,
    pub revision: u64,
    pub project: Uuid,
    pub board: Uuid,
    pub changes: SetupChanges,
    pub requires_populated_board_consent: bool,
    pub can_apply: bool,
    pub refusal: Option<SetupRefusal>,
}

/// One bounded runtime capability, separate from authored snapshots and undo history.
#[derive(Debug)]
pub(crate) struct CachedPreview {
    snapshot: Arc<Project>,
    opening: u64,
    request: Value,
    report: SetupPreview,
}

impl CachedPreview {
    /// Require the exact opening and immutable input, even after undo or reopen.
    fn matches(&self, session: &Session) -> bool {
        self.report.revision == session.revision()
            && self.opening == session.history_root_revision
            && session.snapshot().is_some_and(|snapshot| Arc::ptr_eq(&self.snapshot, &snapshot))
    }
}

impl Session {
    /// Borrow current native review data. A changed or reopened project has no usable review.
    pub fn board_setup_preview(&self) -> Option<&SetupPreview> {
        self.setup_preview.as_ref().filter(|preview| preview.matches(self)).map(|preview| &preview.report)
    }

    /// Validate a candidate using Apply's checks, bound its report, then replace one runtime
    /// capability. A refusal is review data; malformed candidates and stale revisions are errors.
    pub(crate) fn preview_board_setup(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let request = setup_request(self.open_project()?, params)?;
        let setup: BoardSetup = serde_json::from_value(request["setup"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let before = snapshot.board_setup().ok_or_else(|| CommandError::Unavailable("initialize board setup first".into()))?;
        let target = snapshot.resolve_board(params.get("board").and_then(Value::as_str))?;
        let changes = changes(&snapshot, before, &setup)?;
        let refusal = self
            .board_setup_candidate(params)
            .and_then(|candidate| candidate.map(|project| self.prepare_change(project)).transpose())
            .err()
            .map(refusal);
        let report = SetupPreview {
            plan_token: crate::fresh_id()?,
            revision: self.revision(),
            project: target.project,
            board: target.board,
            requires_populated_board_consent: !snapshot.board.footprints().is_empty() && !before.same_topology(&setup),
            can_apply: refusal.is_none(),
            refusal,
            changes,
        };
        serde_json::to_writer(ReportSize::new(MAX_REPORT_BYTES), &report)
            .map_err(|e| CommandError::Unavailable(format!("board setup preview is too large to review: {e}")))?;
        self.setup_preview = Some(CachedPreview { snapshot, opening: self.history_root_revision, request, report });
        Ok(())
    }

    /// Check optional review authority before any adoption. The token freezes the complete
    /// canonical setup, consent and silkscreen request; it cannot authorize a different candidate.
    pub(crate) fn validate_board_setup_token(&self, params: &Value) -> Result<(), CommandError> {
        if params.get("plan_token").is_none() {
            return Ok(());
        }
        let token = token(params)?;
        let preview = self
            .setup_preview
            .as_ref()
            .filter(|p| p.report.plan_token == token)
            .ok_or_else(|| CommandError::Unavailable("unknown, stale or replaced board setup preview; preview again".into()))?;
        if !preview.matches(self) {
            return Err(CommandError::Unavailable("board setup preview is stale; preview again".into()));
        }
        if preview.request != setup_request(self.open_project()?, params)? {
            return Err(CommandError::InvalidParams("board setup preview does not match the candidate, consent or silkscreen settings".into()));
        }
        Ok(())
    }

    /// Cancel only the current named capability, so a late cancellation cannot revoke a newer review.
    pub(crate) fn cancel_board_setup_preview(&mut self, params: &Value) -> Result<(), CommandError> {
        let token = token(params)?;
        if !self.board_setup_preview().is_some_and(|report| report.plan_token == token) {
            return Err(CommandError::Unavailable("unknown or replaced board setup preview".into()));
        }
        self.setup_preview = None;
        Ok(())
    }
}

/// Canonical candidate data excludes transport addressing, revisions and the capability itself.
fn setup_request(project: &Project, params: &Value) -> Result<Value, CommandError> {
    let setup = super::groups::candidate(project, &params["setup"])?;
    let mut request = json!({"setup":setup,"resolve_existing_footprints":params["resolve_existing_footprints"].as_bool() == Some(true)});
    if let Some(silk) = params.get("silkscreen") {
        let silk: crate::board::silkscreen::Settings =
            serde_json::from_value(silk.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        request["silkscreen"] = json!(silk);
    }
    Ok(request)
}

/// Parse only non-nil UUID capabilities; arbitrary text and persistent IDs are not authority.
fn token(params: &Value) -> Result<Uuid, CommandError> {
    params["plan_token"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or_else(|| CommandError::InvalidParams("invalid board setup preview token".into()))
}

/// Preserve command error categories in the review's serialized refusal.
fn refusal(error: CommandError) -> SetupRefusal {
    let kind = match &error {
        CommandError::InvalidParams(_) => "invalid_params",
        CommandError::Unavailable(_) => "unavailable",
        CommandError::Unknown(_) => "unknown",
    };
    SetupRefusal { kind, message: error.to_string() }
}

/// A budget failure in Preview does not revoke existing authority or affect ordinary Apply.
fn report_budget(error: impl std::fmt::Display) -> CommandError {
    CommandError::Unavailable(format!("complete board setup review exceeds its byte budget: {error}"))
}

/// Borrow a span while counting its encoded cost, before retaining another layer-vector copy.
#[derive(Serialize)]
struct ObjectRef<'a> {
    kind: &'static str,
    id: &'a str,
    layers: &'a [Uuid],
}

/// Charge every complete object and comma before allocation; over-budget lists are never truncated.
fn append(objects: &mut Vec<AffectedObject>, budget: &mut ReportSize, kind: &'static str, id: String, layers: &[Uuid]) -> Result<(), CommandError> {
    serde_json::to_writer(&mut *budget, &ObjectRef { kind, id: &id, layers }).map_err(report_budget)?;
    budget.write_all(b",").map_err(report_budget)?;
    objects.push(AffectedObject { kind, id, layers: layers.to_vec() });
    Ok(())
}

/// Enumerate layer-addressed authored objects without computing geometry or electrical connectivity.
/// Through vias and all-copper pads are affected when their resolved physical spans change.
fn changes(project: &Project, before: &BoardSetup, after: &BoardSetup) -> Result<SetupChanges, CommandError> {
    let positions = |setup: &BoardSetup| setup.copper_stack().enumerate().map(|(i, layer)| (layer.layer, i + 1)).collect::<BTreeMap<_, _>>();
    let (old, new) = (positions(before), positions(after));
    let changed: BTreeSet<_> = before
        .layers()
        .iter()
        .filter(|layer| {
            !after.layers().iter().any(|next| next.id == layer.id && next.purpose == layer.purpose && next.side == layer.side)
                || old.get(&layer.id) != new.get(&layer.id)
                || before.copper(layer.id).map(|c| c.role) != after.copper(layer.id).map(|c| c.role)
        })
        .map(|l| l.id)
        .collect();
    let mut report = SetupChanges {
        removed_layers: before.layers().iter().filter(|l| !after.layers().iter().any(|n| n.id == l.id)).cloned().collect(),
        added_layers: after.layers().iter().filter(|l| !before.layers().iter().any(|n| n.id == l.id)).cloned().collect(),
        renumbered_layers: old
            .iter()
            .filter_map(|(id, before)| {
                new.get(id).filter(|after| *after != before).map(|after| LayerPosition { id: *id, before: *before, after: *after })
            })
            .collect(),
        affected_objects: Vec::new(),
        affected_groups: before.stackup.layer_groups.iter().filter(|group| !after.stackup.layer_groups.contains(group)).cloned().collect(),
    };
    let mut budget = ReportSize::new(MAX_CHANGES_BYTES);
    serde_json::to_writer(&mut budget, &report).map_err(report_budget)?;
    let mut objects = Vec::new();
    for track in &project.board.tracks {
        if changed.contains(&track.layer) {
            append(&mut objects, &mut budget, "track", track.id.to_string(), &[track.layer])?;
        }
    }
    if before.copper_stack().map(|l| l.layer).ne(after.copper_stack().map(|l| l.layer)) {
        let layers: Vec<_> = before.copper_stack().map(|l| l.layer).collect();
        for via in &project.board.vias {
            append(&mut objects, &mut budget, "via", via.id.to_string(), &layers)?;
        }
    }
    for zone in &project.board.zones {
        if changed.contains(&zone.layer) {
            append(&mut objects, &mut budget, "zone", zone.id.to_string(), &[zone.layer])?;
        }
    }
    if let Some(drawings) = &project.board.drawings {
        for drawing in &drawings.items {
            if changed.contains(&drawing.layer) {
                append(&mut objects, &mut budget, "drawing", drawing.id.to_string(), &[drawing.layer])?;
            }
        }
    }
    use crate::board::definition::CopperBinding::{AllCopper, ComponentSide};
    let spans: Vec<_> = [(ComponentSide, false), (ComponentSide, true), (AllCopper, false), (AllCopper, true)]
        .into_iter()
        .map(|(binding, bottom)| {
            (binding, bottom, CopperLayers::resolve(Some(before), binding, bottom), CopperLayers::resolve(Some(after), binding, bottom))
        })
        .collect();
    for footprint in project.board.footprints() {
        if let Some(definition) = project.board.definition_for(footprint) {
            for pad in &definition.pads {
                if let Some((_, _, previous, next)) =
                    spans.iter().find(|(binding, bottom, _, _)| *binding == pad.copper && *bottom == footprint.bottom)
                    && previous != next
                    && let CopperLayers::Resolved { layers, .. } = previous
                {
                    append(&mut objects, &mut budget, "pad", format!("footprint-{}/{}", footprint.id, pad.number), layers)?;
                }
            }
        }
    }
    objects.sort_by(|a, b| (a.kind, &a.id).cmp(&(b.kind, &b.id)));
    report.affected_objects = objects;
    Ok(report)
}

/// Count a complete report without allocating an unbounded serialized intermediate.
struct ReportSize {
    bytes: usize,
    limit: usize,
}
impl ReportSize {
    /// Start a fresh bounded counting sink without retaining serialized data.
    fn new(limit: usize) -> Self {
        Self { bytes: 0, limit }
    }
}
impl Write for ReportSize {
    /// Stop before a complete review would exceed the transport budget.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| io::Error::other("complete review exceeds its byte budget"))?;
        Ok(bytes.len())
    }
    /// The counting sink has no output to flush.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
