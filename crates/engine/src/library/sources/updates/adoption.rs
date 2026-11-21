//! Explicit, single-instance acceptance of a reviewed native footprint revision.
use super::{FootprintUpdate, footprint_update};
use crate::{
    Command, CommandError, Param, ParamKind, Project, Session,
    board::{
        assignment::Assignment,
        parse_mapping,
        work::{Progress, Source},
    },
    library::{
        parts::mapping,
        sources::{Catalog, matches_pinned},
    },
    part_instances::{Selection, storage},
    persistence::DocumentsEncoder,
};
use serde_json::Value;
use std::sync::Arc;

pub const APPLY_FOOTPRINT_REVISION: &str = "pcb.apply_footprint_revision";
pub(crate) const COMMAND: Command = Command {
    id: APPLY_FOOTPRINT_REVISION,
    title: "Apply Reviewed Footprint Revision",
    summary: "Accept a complete footprint revision preview and an explicit terminal map for one instance. Keep existing tracks and vias in place; undo the replacement in one step.",
    params: &[
        Param { required: true, ..crate::BOARD },
        Param { name: "part_instance", required: true, summary: "Placed part instance ID from the preview", kind: ParamKind::Text },
        crate::EXPECTED_REVISION,
        Param { name: "acceptance", required: true, summary: "Digest from the unfiltered preview", kind: ParamKind::LibraryDigest },
        Param { name: "mapping", required: true, summary: "Complete reviewed terminal-to-pad mapping", kind: ParamKind::FootprintMapping },
        Param { name: "unconnected_pads", required: false, summary: "Explicit non-electrical pad IDs", kind: ParamKind::UnconnectedPads },
    ],
};

/// Work against one immutable project and catalog. The editor captures this job without
/// resolving library inventory, copying the board, or running a save encoder.
pub struct FootprintRevisionJob {
    source: Source,
    catalog: Arc<Catalog>,
    defer_net_reconciliation: bool,
    params: Value,
    pending: Option<(Arc<Project>, u64, DocumentsEncoder)>,
    result: Option<Result<PreparedFootprintRevision, CommandError>>,
    done: bool,
}

/// A candidate that passed the complete shared transaction and persistence checks.
pub struct PreparedFootprintRevision {
    source: Source,
    catalog: Arc<Catalog>,
    candidate: Arc<Project>,
    highest: u64,
    params: Value,
}

impl Session {
    /// Validate shared command parameters, then capture immutable authority for native hosts.
    pub fn prepare_footprint_revision(&self, params: &Value) -> Result<FootprintRevisionJob, CommandError> {
        self.refuse_stale(APPLY_FOOTPRINT_REVISION, params)?;
        self.validate_project_command(APPLY_FOOTPRINT_REVISION, params)?;
        self.footprint_revision_job(params)
    }

    /// Publish one validated edit; a different project, opening, revision or catalog refuses it.
    pub fn commit_footprint_revision(&mut self, edit: PreparedFootprintRevision) -> Result<u64, CommandError> {
        self.install_footprint_revision(&edit)?;
        self.report_committed(APPLY_FOOTPRINT_REVISION, &edit.params);
        Ok(self.revision())
    }

    /// Capture the validated project and catalog by pointer without visiting board records.
    fn footprint_revision_job(&self, params: &Value) -> Result<FootprintRevisionJob, CommandError> {
        self.check_revision(params)?;
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let target = snapshot.resolve_board(params["board"].as_str())?;
        let effective =
            snapshot.board_view(target).ok_or_else(|| CommandError::Unavailable("The resolved board is unavailable.".into()))?.effective_rules();
        let source = Source {
            snapshot,
            session: self.clearance_identity.clone(),
            opening: self.history_root_revision(),
            revision: self.revision(),
            target,
            effective,
        };
        Ok(FootprintRevisionJob {
            source,
            catalog: self.libraries.clone(),
            defer_net_reconciliation: self.defer_net_reconciliation,
            params: params.clone(),
            pending: None,
            result: None,
            done: false,
        })
    }

    /// Check project and catalog authority before changing the shared history.
    fn install_footprint_revision(&mut self, edit: &PreparedFootprintRevision) -> Result<(), CommandError> {
        if !edit.source.is_current(self) {
            return Err(CommandError::StaleRevision { current: self.revision() });
        }
        if !Arc::ptr_eq(&edit.catalog, &self.libraries) {
            return Err(invalid("Library catalog changed during footprint preparation; preview again."));
        }
        self.install_prepared_change(edit.candidate.clone(), edit.highest);
        Ok(())
    }

    /// Headless execution drains the same job. The enclosing command records the edit once.
    pub(crate) fn apply_footprint_revision(&mut self, params: &Value) -> Result<(), CommandError> {
        let mut job = self.footprint_revision_job(params)?;
        while !job.step(256) {}
        let edit = job.take_result()?.ok_or_else(|| CommandError::Unavailable("Footprint preparation did not complete.".into()))?;
        self.install_footprint_revision(&edit)
    }
}

impl FootprintRevisionJob {
    /// Snapshot authority for the host's queue. Catalog authority is checked at commit too.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Report completed phases, not an estimated time to completion.
    pub fn progress(&self) -> Progress {
        Progress {
            stage: if self.done {
                "Prepared footprint revision"
            } else if self.pending.is_some() {
                "Checking project save limits"
            } else {
                "Preparing footprint revision"
            },
            completed: if self.done { 2 } else { usize::from(self.pending.is_some()) },
            total: 2,
        }
    }

    /// Construct the candidate once on the worker, then check the final documents in batches.
    /// No result is published if preparation or the exact encoder rejects it.
    pub fn step(&mut self, budget: usize) -> bool {
        if self.done || budget == 0 {
            return self.done;
        }
        match self.advance(budget) {
            Ok(None) => return false,
            Ok(Some(edit)) => self.result = Some(Ok(edit)),
            Err(error) => self.result = Some(Err(error)),
        }
        self.pending = None;
        self.done = true;
        true
    }

    /// Take the validated result or refusal once. Dropping the job never edits the session.
    pub fn take_result(&mut self) -> Result<Option<PreparedFootprintRevision>, CommandError> {
        self.result.take().transpose()
    }

    /// Yield before the first encode batch and return a candidate only after the final batch.
    fn advance(&mut self, budget: usize) -> Result<Option<PreparedFootprintRevision>, CommandError> {
        if let Some((_, _, encoder)) = &mut self.pending {
            if encoder.step(budget).map_err(CommandError::InvalidParams)?.is_none() {
                return Ok(None);
            }
            let (candidate, highest, _) =
                self.pending.take().ok_or_else(|| CommandError::Unavailable("The encoded footprint candidate is unavailable.".into()))?;
            return Ok(Some(PreparedFootprintRevision {
                source: self.source.clone(),
                catalog: self.catalog.clone(),
                candidate,
                highest,
                params: std::mem::take(&mut self.params),
            }));
        }
        let project = &self.source.snapshot;
        let candidate = self.candidate(project, &self.params)?;
        let (candidate, highest) = Session::prepare_snapshot_change(Some(project), self.defer_net_reconciliation, candidate)?;
        let candidate = Project::with_allocation_watermark(Arc::new(candidate), project.numeric_id_high_water);
        self.pending = Some((candidate.clone(), highest, DocumentsEncoder::new(candidate)));
        Ok(None)
    }

    /// Resolve review, select the package and validate its candidate mapping against the captured project.
    fn candidate(&self, project: &Project, params: &Value) -> Result<Project, CommandError> {
        let update = footprint_update(project, &self.catalog, params)?;
        let delta = super::footprint_delta::compare(&project.board, update.placement, update.pinned, &update.candidate, None)?;
        if params["acceptance"].as_str() != Some(update.acceptance(project, &self.catalog, self.source.revision(), &delta).as_str()) {
            return Err(invalid("Footprint preview is stale or does not match this target; preview again."));
        }
        if update.placement.locked {
            return Err(invalid("Unlock the footprint before applying a revision."));
        }
        if update.placement.mapping.as_ref() != Some(&update.assignment.mapping)
            || update.placement.unconnected_pads != update.assignment.unconnected_pads
        {
            return Err(invalid("Placed footprint and package assignment have different terminal maps."));
        }
        let mapping = parse_mapping(&params["mapping"]).ok_or_else(|| invalid("A complete terminal map is required."))?;
        let unconnected_pads: Vec<String> = params
            .get("unconnected_pads")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(|error| invalid(error.to_string()))?
            .unwrap_or_default();
        mapping::validate(&mapping, &unconnected_pads, &update.instance.definition, &update.candidate).map_err(CommandError::InvalidParams)?;
        let (library, definition, definition_revision) = update.candidate.key();
        let assignment = Assignment { component: update.instance.ordinal, library, definition, definition_revision, mapping, unconnected_pads };
        let selection = native_selection(&update, &assignment)?;
        let mut candidate = project.clone();
        if let Some(selection) = selection {
            Arc::make_mut(&mut candidate.components)
                .get_mut(&assignment.component)
                .ok_or_else(|| CommandError::NotFound("The reviewed part instance is no longer present.".into()))?
                .part = Some(selection);
        }
        Arc::make_mut(&mut candidate.board).replace_reviewed_footprint(assignment, update.candidate).map_err(CommandError::InvalidParams)?;
        if let Some(baseline) = &candidate.board.baseline {
            baseline.validate_assets(&candidate).map_err(CommandError::InvalidParams)?;
        }
        storage::validate_project(&candidate).map_err(CommandError::InvalidParams)?;
        Ok(candidate)
    }
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::InvalidParams(message.into())
}

/// A physical review cannot authorize a symbol, category, value, MPN or unrelated package change.
fn native_selection(update: &FootprintUpdate<'_>, assignment: &Assignment) -> Result<Option<Selection>, CommandError> {
    let Some((part, package_id)) = update.native else { return Ok(None) };
    let old = update.instance.part.as_ref().ok_or_else(|| invalid("The reviewed part instance has no resolved native package."))?;
    let before = &old.resolved.definition;
    let after = &part.definition;
    let symbol = &update.instance.definition;
    let package = after.packages.get(package_id).ok_or_else(|| invalid("The candidate native part has no matching package."))?;
    let unchanged = before.symbol == after.symbol
        && matches_pinned(symbol, part.schematic_symbol().map_err(CommandError::InvalidParams)?)
        && before.category == after.category
        && old.resolved.category == part.category
        && before.fixed_parameters == after.fixed_parameters
        && before.packages.iter().filter(|(id, _)| id.as_str() != package_id).eq(after.packages.iter().filter(|(id, _)| id.as_str() != package_id))
        && old
            .resolved
            .footprints
            .iter()
            .filter(|(id, _)| id.as_str() != package_id)
            .eq(part.footprints.iter().filter(|(id, _)| id.as_str() != package_id));
    if !unchanged || old.manufacturer.is_some() {
        return Err(invalid("This native part revision needs a broader review of its dependencies or manufacturer selection."));
    }
    if !package
        .mapping
        .iter()
        .map(|(terminal, pads)| (terminal, pads.as_slice()))
        .eq(assignment.mapping.iter().map(|(terminal, pads)| (terminal, pads.pads())))
        || package.unconnected_pads != assignment.unconnected_pads
    {
        return Err(invalid("The reviewed map must match the candidate native package's declared map."));
    }
    let mut resolved = part.clone();
    resolved.symbol = Some(symbol.clone());
    let selection = Selection { resolved: Arc::new(resolved), ..old.clone() };
    selection.validate().map_err(CommandError::InvalidParams)?;
    Ok(Some(selection))
}
