//! Resumable preparation of existing copper commands against one immutable board snapshot.
use super::*;
use crate::board::{
    class_rules::{Context, NetRules},
    clearance::{BoardGeometry, ClearanceQuery},
    teardrop,
    view::BoardTarget,
    work::{Progress, Source},
};
use std::sync::{Arc, Mutex, TryLockError};

/// Whether an existing shared command can use copper preparation; no new router policy is implied.
pub fn is_copper_edit(command: &str) -> bool {
    matches!(
        command,
        super::super::via_nets::SET_VIA_NET | ADD_TRACKS | ADD_VIA | MOVE_TRACKS | RESHAPE_TRACKS | SET_TRACK_WIDTH | SET_VIA_SIZE | DELETE_TRACKS
    )
}

#[derive(Default)]
struct BaseProjection {
    next: usize,
    lands: Vec<Land>,
    ready: Option<Arc<Vec<Land>>>,
}

/// Reusable immutable-source preparation. Partial footprint projection survives pointer
/// request replacement, avoiding starvation when browser work is coalesced every frame.
#[derive(Clone)]
pub struct CopperPreparation {
    source: Source,
    base: Arc<Mutex<BaseProjection>>,
    defer_net_reconciliation: bool,
}
impl std::fmt::Debug for CopperPreparation {
    /// Do not format project contents or lock a worker's projection state for diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopperPreparation").field("source", &self.source.target()).finish()
    }
}
impl CopperPreparation {
    /// Captured board, opening, revision and effective rules for all requests in this context.
    pub fn source(&self) -> &Source {
        &self.source
    }
}

/// An opaque engine-produced edit. Geometry and its successful validation cannot be supplied
/// by a client boolean; publication consumes this value and rechecks current source authority.
pub struct PreparedCopperEdit {
    source: Source,
    candidate: Option<Arc<Project>>,
    /// Present when the worker already validated the publication candidate after regeneration.
    prepared_highest: Option<u64>,
    command: &'static str,
    /// The parameters the edit was prepared with, reported with it once committed.
    params: Value,
    class_rules: Option<crate::board::class_rules::Context>,
}
impl PreparedCopperEdit {
    /// Captured policies explain changed-copper validation, with source identities and conflicts.
    /// Unchanged copper is not certified by this edit and may retain existing violations.
    pub fn class_rules(&self) -> Option<&crate::board::class_rules::Context> {
        self.class_rules.as_ref()
    }
    /// The board and revision validated during preparation.
    pub fn source(&self) -> &Source {
        &self.source
    }
    /// The existing shared command represented by this candidate.
    pub fn command(&self) -> &'static str {
        self.command
    }
}

/// Candidate construction, reusable pad projection and bounded contact validation for a copper
/// command. Dropping unfinished work never changes the source project, revision or history.
pub struct CopperEditJob {
    context: CopperPreparation,
    command: &'static str,
    params: Value,
    phase: u8,
    candidate: Option<Arc<Project>>,
    added: BTreeSet<Uuid>,
    lands: Option<Arc<Vec<Land>>>,
    builder: Option<IslandBuilder>,
    work: usize,
    result: Option<Result<PreparedCopperEdit, CommandError>>,
    class_islands: Option<(Copper, Vec<Vec<usize>>)>,
    check_route_vias: bool,
}

/// A cooperative browser job runs on the editor thread. Refuse active intents before
/// allocating work, rather than doing the final teardrop verification there in one step.
fn check_teardrop_preparation(has_teardrops: bool, cooperative: bool) -> Result<(), CommandError> {
    if has_teardrops && cooperative {
        return Err(CommandError::Unavailable("copper edits with active teardrops are not supported in the browser".into()));
    }
    Ok(())
}

impl Session {
    /// Capture a straight board's preparation context without projecting pads. Native jobs
    /// can check teardrops before publication; browser jobs refuse active intents up front.
    pub fn copper_preparation(&self, target: BoardTarget) -> Result<CopperPreparation, CommandError> {
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let view = snapshot.board_view(target).ok_or_else(|| CommandError::NotFound("unknown board target".into()))?;
        view.require_no_arcs().map_err(CommandError::Unavailable)?;
        // The browser cooperatively polls on the frame thread. The WASI runner
        // uses the headless engine and can prepare active intents like native builds.
        check_teardrop_preparation(!view.teardrops().is_empty(), cfg!(all(target_arch = "wasm32", target_os = "unknown")))?;
        let effective = view.effective_rules();
        Ok(CopperPreparation {
            source: Source {
                snapshot,
                session: self.clearance_identity.clone(),
                opening: self.history_root_revision(),
                revision: self.revision(),
                target,
                effective,
            },
            base: Arc::default(),
            defer_net_reconciliation: self.defer_net_reconciliation,
        })
    }
    /// Validate catalog parameters, source and revision before queuing work. Reusing a context
    /// retains derived pads across pointer generations while every candidate is independently checked.
    /// Scope and stale revisions are checked in the same order as by [`Session::execute`].
    pub fn prepare_copper_edit_with(&self, context: &CopperPreparation, command: &str, params: &Value) -> Result<CopperEditJob, CommandError> {
        let (command, target) = self.admit_copper_edit(command, params)?;
        if target != context.source.target() || !context.source.is_current(self) {
            return Err(CommandError::InvalidParams("copper preparation source is stale or belongs to another board".into()));
        }
        Ok(Self::copper_job(context, command, params))
    }

    /// Prepare an interactive route with all its layer transitions checked against mechanical
    /// openings at the sizes and net membership of the candidate that would be published.
    /// Direct authored commands retain their ordinary validation, including for DRC fixtures.
    pub fn prepare_interactive_route_with(&self, context: &CopperPreparation, params: &Value) -> Result<CopperEditJob, CommandError> {
        let mut job = self.prepare_copper_edit_with(context, ADD_TRACKS, params)?;
        job.check_route_vias = params.get("layer_changes").and_then(Value::as_array).is_some_and(|changes| !changes.is_empty())
            && context.source.snapshot.board.definitions.values().any(|definition| !definition.non_plated_holes.is_empty());
        Ok(job)
    }

    /// Check scope before stale revisions or resolving a preparation context.
    fn admit_copper_edit(&self, command: &str, params: &Value) -> Result<(&'static str, BoardTarget), CommandError> {
        self.authorize_command(command)?;
        self.refuse_stale(command, params)?;
        if !is_copper_edit(command) {
            return Err(CommandError::InvalidParams("command does not prepare copper".into()));
        }
        self.validate_project_command(command, params)?;
        self.check_revision(params)?;
        let target = self.open_project()?.resolve_board(params.get("board").and_then(Value::as_str))?;
        let command = crate::command(command).ok_or_else(|| CommandError::Unknown(command.into()))?.id;
        Ok((command, target))
    }

    /// Construct work only for a validated command with a current preparation context.
    fn copper_job(context: &CopperPreparation, command: &'static str, params: &Value) -> CopperEditJob {
        CopperEditJob {
            context: context.clone(),
            command,
            params: params.clone(),
            phase: 0,
            candidate: None,
            added: BTreeSet::new(),
            lands: None,
            builder: None,
            work: 0,
            result: None,
            class_islands: None,
            check_route_vias: false,
        }
    }
    /// Prepare one existing copper command without adopting any authored state. Headless callers
    /// may drain the job; GUI/MCP hosts can schedule the same work and defer publication.
    /// Scope and a stale `expected_revision` are checked before resolving the board.
    pub fn prepare_copper_edit(&self, command: &str, params: &Value) -> Result<CopperEditJob, CommandError> {
        let (command, target) = self.admit_copper_edit(command, params)?;
        Ok(Self::copper_job(&self.copper_preparation(target)?, command, params))
    }
    /// Consume an engine-validated candidate in one shared history transaction. Another session,
    /// reopening, rule/geometry edit or Undo/Redo makes the prepared source stale before publication.
    /// A regenerated candidate is already validated; publication does not rebuild its geometry.
    pub fn commit_copper_edit(&mut self, prepared: PreparedCopperEdit) -> Result<u64, CommandError> {
        self.authorize_command(prepared.command)?;
        if !prepared.source.is_current(self) {
            return Err(CommandError::InvalidParams("prepared copper edit is stale".into()));
        }
        if let Some(candidate) = prepared.candidate {
            if let Some(highest) = prepared.prepared_highest {
                self.adopt_change(candidate, highest);
            } else {
                self.change((*candidate).clone())?;
            }
            self.report_committed(prepared.command, &prepared.params);
        }
        Ok(self.revision())
    }
}
impl CopperEditJob {
    /// Captured source authority; the host also owns and validates its input generation.
    pub fn source(&self) -> &Source {
        &self.context.source
    }
    /// Report work progress without treating partial validation as a valid candidate.
    pub fn progress(&self) -> Progress {
        let total = self.lands.as_ref().map_or(1, |lands| {
            let n = lands.len() + self.candidate.as_ref().map_or(0, |p| p.board.tracks.len() + p.board.vias.len());
            n.saturating_mul(n.saturating_sub(1)) / 2
        });
        Progress {
            stage: match self.phase {
                0 => "Preparing copper edit",
                1 => "Preparing pad contacts",
                2 => "Checking copper contacts",
                3 => "Validating route",
                _ => "Prepared",
            },
            completed: self.work.min(total),
            total,
        }
    }
    /// Advance bounded contact/placement batches, yielding between preparation phases. Errors
    /// terminate the job without a candidate; a zero budget performs no work.
    pub fn step(&mut self, budget: usize) -> bool {
        if self.phase == 4 {
            return true;
        }
        if budget == 0 {
            return false;
        }
        match self.advance(budget) {
            Ok(done) => done,
            Err(error) => {
                self.result = Some(Err(error));
                self.phase = 4;
                true
            }
        }
    }
    /// Consume the complete validated candidate once. Pending work returns None and failed
    /// validation returns its ordinary command error, never a partially accepted route.
    pub fn take_result(&mut self) -> Result<Option<PreparedCopperEdit>, CommandError> {
        self.result.take().transpose()
    }
    /// Capture and enforce class dimensions using the same candidate contacts as regeneration.
    fn candidate_class_rules(&self, view: BoardView<'_>, geometry: &BoardGeometry) -> Result<Option<Context>, CommandError> {
        let context = NetRules::new(view, self.context.source.effective.clone())?.map(|rules| rules.capture(view, geometry)).transpose()?;
        if let Some(context) = &context {
            for item in &geometry.items {
                let Ok(id) = Uuid::parse_str(&item.name) else { continue };
                if !self.added.contains(&id) {
                    continue;
                }
                if item.intended_nets.len() > 1 {
                    return Err(CommandError::InvalidParams("This copper would connect distinct intended nets.".into()));
                }
                if let Some(policy) = context.rules_for(&item.name).filter(|policy| policy.has_dimensions()) {
                    if let Ok(index) = view.tracks().binary_search_by_key(&id, |track| track.id) {
                        policy.track_width(Some(view.tracks()[index].width_nm))?;
                    }
                    if let Ok(index) = view.vias().binary_search_by_key(&id, |via| via.id) {
                        let via = &view.vias()[index];
                        policy.via_sizes(Some(via.diameter_nm), Some(via.drill_nm))?;
                    }
                }
            }
        }
        Ok(context)
    }

    /// For an interactive route with layer changes, refuse an added via land too close to a
    /// non-plated hole at the candidate's final sizes and resolved net clearance.
    fn check_route_via_holes(
        &self,
        land_count: usize,
        view: BoardView<'_>,
        geometry: &BoardGeometry,
        context: Option<&Context>,
    ) -> Result<(), CommandError> {
        if !self.check_route_vias {
            return Ok(());
        }
        let via_start = land_count + view.tracks().len();
        let baseline = self.context.source.effective.values.clearance_nm;
        for (offset, via) in view.vias().iter().enumerate().filter(|(_, via)| self.added.contains(&via.id)) {
            let index = via_start + offset;
            let clearance = context.and_then(|rules| rules.object(index)).map_or(Ok(baseline), |policy| policy.clearance_nm())?;
            if geometry.mechanical_violation(index, clearance) {
                return Err(CommandError::InvalidParams(format!(
                    "Via land at ({}, {}) is too close to a non-plated hole.",
                    via.at.x_nm, via.at.y_nm
                )));
            }
        }
        Ok(())
    }

    /// Shared native/browser phases use exactly the synchronous command's geometry and short policy.
    fn advance(&mut self, budget: usize) -> Result<bool, CommandError> {
        match self.phase {
            0 => {
                let (candidate, added) = edit_candidate(&self.context.source.snapshot, self.command, &self.params)?;
                self.added = added;
                self.candidate = candidate.map(Arc::new);
                if self.candidate.as_ref().is_none_or(|candidate| self.added.is_empty() && candidate.board.teardrops.is_empty()) {
                    self.phase = 4;
                    self.result = Some(Ok(PreparedCopperEdit {
                        source: self.context.source.clone(),
                        candidate: self.candidate.take(),
                        prepared_highest: None,
                        command: self.command,
                        params: self.params.clone(),
                        class_rules: None,
                    }));
                    return Ok(true);
                }
            }
            1 => {
                let source = &self.context.source;
                let view = source.snapshot.board_view(source.target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
                let mut base = match self.context.base.try_lock() {
                    Ok(base) => base,
                    Err(TryLockError::WouldBlock) => return Ok(false),
                    Err(TryLockError::Poisoned(_)) => return Err(CommandError::Unavailable("copper preparation state is unavailable".into())),
                };
                if base.ready.is_none() {
                    let next = base.next;
                    let count = budget.min(32);
                    base.lands.extend(lands_from_footprints(view.footprints_range(next, count)));
                    base.next = base.next.saturating_add(count);
                    if base.next < source.snapshot.board.footprints.len() {
                        return Ok(false);
                    }
                    base.ready = Some(Arc::new(std::mem::take(&mut base.lands)));
                }
                self.lands = base.ready.clone();
                if let (Some(lands), Some(candidate)) = (&self.lands, &self.candidate) {
                    self.builder = Some(IslandBuilder::new(lands, &candidate.board.tracks, &candidate.board.vias));
                }
            }
            2 => {
                if !self.builder.as_mut().is_some_and(|b| b.step(budget)) {
                    self.work = self.work.saturating_add(budget);
                    return Ok(false);
                }
                if let (Some(builder), Some(lands), Some(candidate)) = (self.builder.take(), &self.lands, &self.candidate) {
                    let (copper, members) = builder.finish(lands, &candidate.board.tracks, &candidate.board.vias);
                    if let Some((a, b)) =
                        short_from_members(candidate, lands, &candidate.board.tracks, &candidate.board.vias, &members, &self.added, &BTreeSet::new())
                    {
                        return Err(CommandError::InvalidParams(format!("this track would connect nets {a} and {b}")));
                    }
                    if self.check_route_vias
                        || !candidate.board.teardrops.is_empty()
                        || candidate.has_dimensional_net_classes()
                        || candidate.board.vias.iter().any(|v| v.net_assignment.is_some())
                    {
                        self.builder = Some(IslandBuilder::pad_contacts(lands, &candidate.board.tracks, &candidate.board.vias, &members));
                        self.class_islands = Some((copper, members));
                    }
                }
            }
            3 => {
                if let Some(lands) = &self.lands
                    && self.class_islands.is_some()
                    && !self.builder.as_mut().is_some_and(|builder| builder.step_pads(lands.len(), budget))
                {
                    self.work = self.work.saturating_add(budget);
                    return Ok(false);
                }
                let source = &self.context.source;
                let mut candidate = self.candidate.take();
                let mut class_rules = None;
                let mut prepared_highest = None;
                if let (Some(project), Some(lands), Some(builder), Some((copper, members))) =
                    (&candidate, &self.lands, self.builder.take(), self.class_islands.take())
                {
                    let view = project.board_view(source.target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
                    let (_, merged) = builder.finish(lands, view.tracks(), view.vias());
                    if let Some((a, b)) = short_from_members(project, lands, view.tracks(), view.vias(), &merged, &self.added, &BTreeSet::new()) {
                        return Err(CommandError::InvalidParams(format!("This copper would connect nets {a} and {b}.")));
                    }
                    if view.teardrops().is_empty() {
                        let geometry = BoardGeometry::from_contacts(view, lands.as_ref().clone(), copper, members, merged);
                        class_rules = self.candidate_class_rules(view, &geometry)?;
                        self.check_route_via_holes(lands.len(), view, &geometry, class_rules.as_ref())?;
                    } else {
                        // Reuse the worker's contact batches. Map the candidate memberships
                        // after reconciliation and keep every polygon/rule query off commit.
                        let (next, highest) = Session::prepare_change_from(
                            Some(&source.snapshot),
                            self.context.defer_net_reconciliation,
                            (**project).clone(),
                            Some(|project: &mut Project| {
                                let view = project.board_view(source.target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
                                let geometry = BoardGeometry::from_contacts(view, lands.as_ref().clone(), copper, members, merged);
                                if self.check_route_vias
                                    || project.has_dimensional_net_classes()
                                    || view.vias().iter().any(|via| via.net_assignment.is_some())
                                {
                                    class_rules = self.candidate_class_rules(view, &geometry)?;
                                }
                                self.check_route_via_holes(lands.len(), view, &geometry, class_rules.as_ref())?;
                                teardrop::regenerate(project, source.target, |project| {
                                    let candidate_source = Source { snapshot: Arc::new(project.clone()), ..source.clone() };
                                    ClearanceQuery::from_geometry(&candidate_source, Arc::new(geometry))
                                })
                            }),
                        )?;
                        candidate = Some(Arc::new(next));
                        prepared_highest = Some(highest);
                    }
                }
                self.result = Some(Ok(PreparedCopperEdit {
                    source: source.clone(),
                    candidate,
                    prepared_highest,
                    command: self.command,
                    params: self.params.clone(),
                    class_rules,
                }));
            }
            _ => {}
        }
        self.phase += 1;
        self.work = 0;
        Ok(self.phase == 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::track::tests::{copper_layers, routed_fixture, view};
    use crate::persistence::encode_documents;

    /// Cooperative preparation only refuses boards that would need teardrop regeneration.
    /// Headless WASI coverage also runs the active-intent move through actual admission.
    #[test]
    fn cooperative_preparation_refuses_active_teardrops() {
        for (has_teardrops, cooperative, refused) in [(false, false, false), (true, false, false), (false, true, false), (true, true, true)] {
            let result = check_teardrop_preparation(has_teardrops, cooperative);
            if refused {
                assert!(matches!(result, Err(CommandError::Unavailable(message)) if message ==
                    "copper edits with active teardrops are not supported in the browser"));
            } else {
                assert!(result.is_ok());
            }
        }
    }

    /// A small off-board segment isolates publication and cancellation from pad geometry.
    fn params(s: &Session) -> Value {
        json!({"board":view(s).target.board,"layer":copper_layers(s)[0],"width_nm":250_000,
            "points":[{"x_nm":-20_000_000,"y_nm":-20_000_000},{"x_nm":-10_000_000,"y_nm":-20_000_000}],"expected_revision":s.revision()})
    }
    /// Drain with an intentionally small budget to exercise every yield boundary.
    fn finish(mut job: CopperEditJob) -> PreparedCopperEdit {
        let mut n = 0;
        assert!(!job.step(0));
        assert!(job.take_result().unwrap().is_none());
        while !job.step(1) {
            n += 1;
            assert!(n < 100_000);
        }
        let prepared = job.take_result().unwrap().unwrap();
        assert!(job.take_result().unwrap().is_none());
        prepared
    }
    /// Preparation and abandoned work preserve the exact snapshot, bytes and history; publication
    /// adopts the validated candidate once and ordinary Undo restores the original document.
    #[test]
    fn preparation_is_read_only_and_commit_is_one_transaction() {
        let mut s = routed_fixture();
        let before = s.snapshot().unwrap();
        let bytes = encode_documents(before.clone()).unwrap();
        let revision = s.revision();
        let mut abandoned = s.prepare_copper_edit(ADD_TRACKS, &params(&s)).unwrap();
        abandoned.step(1);
        drop(abandoned);
        let prepared = finish(s.prepare_copper_edit(ADD_TRACKS, &params(&s)).unwrap());
        assert_eq!(prepared.command(), ADD_TRACKS);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert_eq!(bytes, encode_documents(s.snapshot().unwrap()).unwrap());
        assert_eq!(s.revision(), revision);
        s.commit_copper_edit(prepared).unwrap();
        assert_eq!(s.revision(), revision + 1);
        assert_eq!(view(&s).tracks().len(), 1);
        s.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(bytes, encode_documents(s.snapshot().unwrap()).unwrap());
    }
    /// A completion cannot authorize another session, a reopened project, changed rules, or an
    /// out-of-order mutation. Restoring content through Undo does not revive old authority.
    #[test]
    fn prepared_authority_rejects_stale_reopened_rules_and_out_of_order_results() {
        for mode in 0..5 {
            let mut s = routed_fixture();
            let snapshot = s.snapshot().unwrap();
            let older = finish(s.prepare_copper_edit(ADD_TRACKS, &params(&s)).unwrap());
            match mode {
                0 => {
                    let newer = finish(s.prepare_copper_edit(ADD_TRACKS, &params(&s)).unwrap());
                    s.commit_copper_edit(newer).unwrap();
                }
                1 => s.open_snapshot(snapshot),
                2 => {
                    s.apply(crate::SET_RULES, &json!({"board":view(&s).target.board,"rules":crate::board::rules::Values {min_track_width_nm:200_000,..Default::default()},"expected_revision":s.revision()})).unwrap();
                }
                3 => {
                    s.apply("project.rename", &json!({"name":"Changed"})).unwrap();
                    s.apply("edit.undo", &Value::Null).unwrap();
                }
                _ => {
                    let mut other = Session::default();
                    other.open_snapshot(snapshot);
                    s = other;
                }
            }
            let before = s.snapshot().unwrap();
            let revision = s.revision();
            assert!(s.commit_copper_edit(older).is_err());
            assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
            assert_eq!(s.revision(), revision);
        }
    }
    /// Derived pad batches survive obsolete pointer jobs; simultaneous access yields rather than
    /// waiting. A no-op preserves the live snapshot and adds no history entry.
    #[test]
    fn context_reuses_partial_projection_and_noop_preserves_snapshot() {
        let mut s = routed_fixture();
        let context = s.copper_preparation(view(&s).target).unwrap();
        let mut first = s.prepare_copper_edit_with(&context, ADD_TRACKS, &params(&s)).unwrap();
        assert!(!first.step(1));
        assert!(!first.step(1));
        assert_eq!(context.base.lock().unwrap().next, 1);
        drop(first);
        let mut next = s.prepare_copper_edit_with(&context, ADD_TRACKS, &params(&s)).unwrap();
        assert!(!next.step(1));
        {
            let _guard = context.base.lock().unwrap();
            assert!(!next.step(1));
        }
        assert!(!next.step(1));
        assert_eq!(context.base.lock().unwrap().next, 2);
        s.commit_copper_edit(finish(next)).unwrap();
        let id = view(&s).tracks()[0].id;
        let before = s.snapshot().unwrap();
        let revision = s.revision();
        let params = json!({"tracks":[id],"dx_nm":0,"dy_nm":0,"expected_revision":revision});
        let prepared = finish(s.prepare_copper_edit(MOVE_TRACKS, &params).unwrap());
        s.commit_copper_edit(prepared).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert_eq!(s.revision(), revision);
        let mut forged = params;
        forged["valid"] = json!(true);
        assert!(s.prepare_copper_edit(MOVE_TRACKS, &forged).is_err());
    }
    /// Deletions finish without projecting pads or traversing contacts, while still rejecting
    /// stale publication and retaining exact Undo behavior through the ordinary transaction.
    #[test]
    fn deletion_skips_contacts_but_retains_candidate_and_source_authority() {
        let mut s = routed_fixture();
        s.apply(ADD_TRACKS, &params(&s)).unwrap();
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let revision = s.revision();
        let context = s.copper_preparation(view(&s).target).unwrap();
        let p = json!({"tracks":[view(&s).tracks()[0].id],"expected_revision":revision});
        let mut job = s.prepare_copper_edit_with(&context, DELETE_TRACKS, &p).unwrap();
        assert!(job.step(1), "deletion needs no contact-validation batches");
        assert_eq!(context.base.lock().unwrap().next, 0);
        let prepared = job.take_result().unwrap().unwrap();
        let stale = finish(s.prepare_copper_edit(DELETE_TRACKS, &p).unwrap());
        s.commit_copper_edit(prepared).unwrap();
        assert_eq!(s.revision(), revision + 1);
        assert!(view(&s).tracks().is_empty());
        assert!(s.commit_copper_edit(stale).is_err());
        s.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    }

    /// Indexed reshape lookup handles shuffled selections, preserves exact command results and
    /// rejects unknown IDs without publishing any of the other endpoint changes.
    #[test]
    fn indexed_reshape_preserves_track_identity_and_unknown_id_atomicity() {
        let mut s = routed_fixture();
        let mut p = params(&s);
        p["points"] = json!((0..128).map(|x| Point { x_nm: x * 1_000_000, y_nm: -20_000_000 }).collect::<Vec<_>>());
        s.apply(ADD_TRACKS, &p).unwrap();
        let before = s.snapshot().unwrap();
        let ends: Vec<_> = view(&s)
            .tracks()
            .iter()
            .rev()
            .map(|track| {
                json!({"track":track.id,
            "start":Point {y_nm:track.start.y_nm - 1_000_000,..track.start},
            "end":Point {y_nm:track.end.y_nm - 1_000_000,..track.end}})
            })
            .collect();
        let p = json!({"tracks":ends,"expected_revision":s.revision()});
        let mut invalid = p.clone();
        invalid["tracks"][1]["track"] = json!(Uuid::from_u128(u128::MAX));
        let mut job = s.prepare_copper_edit(RESHAPE_TRACKS, &invalid).unwrap();
        while !job.step(256) {}
        assert!(job.take_result().err().unwrap().to_string().contains("unknown track"));
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        let mut reference = Session::default();
        reference.open_snapshot(before);
        let mut reference_params = p.clone();
        reference_params["expected_revision"] = json!(reference.revision());
        reference.apply(RESHAPE_TRACKS, &reference_params).unwrap();
        let prepared = finish(s.prepare_copper_edit(RESHAPE_TRACKS, &p).unwrap());
        s.commit_copper_edit(prepared).unwrap();
        assert_eq!(encode_documents(reference.snapshot().unwrap()).unwrap(), encode_documents(s.snapshot().unwrap()).unwrap());
    }

    /// Contact preparation uses the candidate's moved copper, excluding its old location.
    /// Both short failures and lock failures remain atomic under the prepared path.
    #[test]
    fn prepared_short_policy_agrees_with_independent_complete_contacts() {
        let mut s = routed_fixture();
        let pads = view(&s).footprints().flat_map(|f| f.pads).collect::<Vec<_>>();
        for (a, b) in pads.iter().flat_map(|a| pads.iter().map(move |b| (a, b))).filter(|(a, b)| a.center != b.center) {
            let p = json!({"layer":copper_layers(&s)[0],"width_nm":250_000,"points":[a.center,b.center],"expected_revision":s.revision()});
            let (candidate, added) = edit_candidate(s.project().unwrap(), ADD_TRACKS, &p).unwrap();
            let expected_short = short(candidate.as_ref().unwrap(), &added).is_some();
            let mut job = s.prepare_copper_edit(ADD_TRACKS, &p).unwrap();
            while !job.step(7) {}
            assert_eq!(job.take_result().is_err(), expected_short);
        }
        s.commit_copper_edit(finish(s.prepare_copper_edit(ADD_TRACKS, &params(&s)).unwrap())).unwrap();
        let id = view(&s).tracks()[0].id;
        let p = json!({"tracks":[id],"dx_nm":3_000_000,"dy_nm":3_000_000,"expected_revision":s.revision()});
        let prepared = finish(s.prepare_copper_edit(MOVE_TRACKS, &p).unwrap());
        assert_eq!(prepared.candidate.as_ref().unwrap().board.tracks.len(), 1);
        s.commit_copper_edit(prepared).unwrap();
        assert_eq!(view(&s).tracks().len(), 1);
        assert_eq!(view(&s).tracks()[0].start.x_nm, -17_000_000);
    }
}
