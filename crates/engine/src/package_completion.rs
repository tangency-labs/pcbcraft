//! Explicit package completion for equivalent schematic units.

use crate::{COMPLETE_PACKAGES, CommandError, Project, Session, SymbolPlacement, Terminal, WireEndpoint};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

/// A loose unit or an existing package that can host one. `occupied` is a bitmap of the native unit slots.
struct Owner<'a> {
    first: &'a SymbolPlacement,
    sheet: Uuid,
    across_sheets: bool,
    occupied: u32,
}

/// The move retains the drawing's address and pose, but changes its part instance and unit slot.
struct Move<'a> {
    symbol: &'a SymbolPlacement,
    sheet: Uuid,
    target: u64,
    slot: usize,
    pins: Vec<(Terminal, Terminal)>,
}

/// Prefer packages that already had multiple units, then stable instance order.
type Slots = BTreeMap<usize, BTreeSet<(bool, u64)>>;

/// Runtime authority for a deferred package completion, independent of display names or boards.
#[derive(Clone, Debug)]
pub struct PackageCompletionSource {
    snapshot: Arc<Project>,
    session: Arc<()>,
    opening: u64,
    revision: u64,
    defer_net_reconciliation: bool,
}

impl PackageCompletionSource {
    /// Revision captured when preparation started.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Reject a different session, project reopening, or any edit/Undo/Redo since capture.
    pub fn is_current(&self, session: &Session) -> bool {
        Arc::ptr_eq(&self.session, &session.clearance_identity)
            && self.opening == session.history_root_revision()
            && self.revision == session.revision()
            && self.defer_net_reconciliation == session.defer_net_reconciliation
            && session.snapshot().is_some_and(|p| Arc::ptr_eq(&p, &self.snapshot))
    }
}

/// Engine-validated package completion ready for reply-size checks and atomic publication.
/// For an effectful Apply, `report()` already carries the post-commit revision so live hosts
/// can verify the exact reply frame size before committing.
pub struct PreparedPackageCompletion {
    source: PackageCompletionSource,
    params: Value,
    report: Value,
    candidate: Option<(Project, u64)>,
}

impl PreparedPackageCompletion {
    /// Snapshot authority validated during preparation.
    pub fn source(&self) -> &PackageCompletionSource {
        &self.source
    }

    /// The exact JSON report that will be returned if this preparation is accepted.
    pub fn report(&self) -> &Value {
        &self.report
    }

    /// Whether committing this preparation will publish a new snapshot and Undo step.
    pub fn is_effectful(&self) -> bool {
        self.candidate.is_some()
    }
}

/// Immutable-snapshot package completion work that can run off the editor thread without
/// mutating session state. Dropping or cancelling it leaves the project and history untouched.
pub struct PackageCompletionJob {
    source: PackageCompletionSource,
    params: Value,
    sheet: Option<Uuid>,
    no_op: bool,
    dry_run: bool,
}

impl PackageCompletionJob {
    /// Captured snapshot authority for staleness checks while work is in flight.
    pub fn source(&self) -> &PackageCompletionSource {
        &self.source
    }

    /// Run preparation to completion synchronously.
    pub fn finish(self) -> Result<PreparedPackageCompletion, CommandError> {
        match self.run_inner(None)? {
            Some(prepared) => Ok(prepared),
            None => Err(CommandError::Unavailable("package completion canceled".into())),
        }
    }

    /// Run preparation off the editor thread, stopping cooperatively if `cancelled` is set.
    pub fn run_cancellable(self, cancelled: &AtomicBool) -> Result<Option<PreparedPackageCompletion>, CommandError> {
        self.run_inner(Some(cancelled))
    }

    fn run_inner(self, cancelled: Option<&AtomicBool>) -> Result<Option<PreparedPackageCompletion>, CommandError> {
        let is_cancelled = || cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed));
        if is_cancelled() {
            return Ok(None);
        }
        if self.no_op {
            return Ok(Some(PreparedPackageCompletion {
                report: json!({"revision": self.source.revision, "changes": [], "skipped": []}),
                source: self.source,
                params: self.params,
                candidate: None,
            }));
        }
        let before = &self.source.snapshot;
        let owners = owners(before);
        let Some((moves, skipped)) = plan_moves(before, &owners, self.sheet, cancelled)? else {
            return Ok(None);
        };
        if is_cancelled() {
            return Ok(None);
        }
        let changes: Vec<_> = moves.iter().map(Move::report).collect();
        let mut report = json!({"revision": self.source.revision, "changes": changes, "skipped": skipped});
        let candidate = if moves.is_empty() {
            None
        } else {
            let (next, terminals) = transfer(before, &moves);
            if is_cancelled() {
                return Ok(None);
            }
            let (prepared, highest) =
                Session::prepare_change_from_snapshot(Some(before), self.source.defer_net_reconciliation, next, |candidate, _| {
                    candidate.rekey_nets(before, &terminals)
                })?;
            if self.dry_run {
                None
            } else {
                let next_revision = (self.source.revision + 1).max(highest);
                report["revision"] = json!(next_revision);
                Some((prepared, highest))
            }
        };
        Ok(Some(PreparedPackageCompletion { source: self.source, params: self.params, report, candidate }))
    }
}

impl Session {
    /// Plan and validate against the current revision, then optionally publish as one undo step.
    /// `scope` must explicitly enable completion. Preview and no-ops never publish an edit.
    pub(crate) fn complete_packages(&mut self, params: &Value) -> Result<Value, CommandError> {
        self.complete_packages_with_commit(params, || Ok(()))
    }

    /// Validate scope, revision and parameters synchronously, capturing an immutable snapshot
    /// job whose planning and net reconciliation can run off the editor thread.
    pub fn prepare_package_completion(&self, params: &Value) -> Result<PackageCompletionJob, CommandError> {
        self.authorize_command(COMPLETE_PACKAGES)?;
        self.refuse_stale(COMPLETE_PACKAGES, params)?;
        self.validate_project_command(COMPLETE_PACKAGES, params)?;
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let sheet_arg = params.get("sheet").and_then(Value::as_str);
        let (no_op, sheet) = match params["scope"].as_str().unwrap_or("none") {
            "sheet" => (false, Some(snapshot.resolve_sheet(sheet_arg)?.id())),
            _ if sheet_arg.is_some() => return Err(CommandError::InvalidParams("`sheet` is only allowed with sheet scope.".into())),
            "none" => (true, None),
            _ => (false, None),
        };
        Ok(PackageCompletionJob {
            source: PackageCompletionSource {
                snapshot,
                session: self.clearance_identity.clone(),
                opening: self.history_root_revision(),
                revision: self.revision(),
                defer_net_reconciliation: self.defer_net_reconciliation,
            },
            params: params.clone(),
            sheet,
            no_op,
            dry_run: params["dry_run"].as_bool() == Some(true),
        })
    }

    /// Adopt a prepared package completion atomically after rechecking session scope and
    /// snapshot currency. A stale or unauthorized session returns an error without mutating state.
    pub fn commit_package_completion(&mut self, mut prepared: PreparedPackageCompletion) -> Result<Value, CommandError> {
        self.authorize_command(COMPLETE_PACKAGES)?;
        if !prepared.source.is_current(self) {
            return Err(CommandError::StaleRevision { current: self.revision() });
        }
        if let Some((candidate, highest)) = prepared.candidate {
            self.publish_change(candidate, highest);
            self.report_committed(COMPLETE_PACKAGES, &prepared.params);
            prepared.report["revision"] = json!(self.revision());
        }
        Ok(prepared.report)
    }

    /// Run package completion with host authorization immediately before publication.
    /// Only a prepared, effectful Apply invokes `authorize`; an error discards the candidate
    /// without changing the snapshot, revision, history or the edit observer.
    pub fn complete_packages_with_commit<E: From<CommandError>>(
        &mut self,
        params: &Value,
        authorize: impl FnOnce() -> Result<(), E>,
    ) -> Result<Value, E> {
        let prepared = self.prepare_package_completion(params)?.finish()?;
        if prepared.is_effectful() {
            authorize()?;
        }
        self.commit_package_completion(prepared).map_err(Into::into)
    }
}

impl Move<'_> {
    /// Report all part/unit transfers and terminal-number changes; the source part is retired.
    fn report(&self) -> Value {
        let symbol = self.symbol;
        let terminals: BTreeMap<_, _> = self.pins.iter().filter(|(from, to)| from != to).map(|(from, to)| (from.number(), to.number())).collect();
        json!({"sheet":self.sheet,"symbol":symbol.id(),"from_part":symbol.component_id(),"from_unit":symbol.unit_id(),
            "to_part":crate::part_groups::part_instance_address(self.target),"to_unit":symbol.definition.units()[self.slot].unit_id(),
            "remapped_terminals":terminals})
    }
}

/// Capture each owner's occupied slots and whether all of them belong to one sheet.
fn owners(project: &Project) -> BTreeMap<u64, Owner<'_>> {
    let mut owners = BTreeMap::new();
    for sheet in project.sheets.iter() {
        for symbol in sheet.symbols.iter().filter(|symbol| symbol.definition.units.len() > 1) {
            let owner = owners.entry(symbol.component).or_insert(Owner { first: symbol, sheet: sheet.sheet_id, across_sheets: false, occupied: 0 });
            owner.across_sheets |= owner.sheet != sheet.sheet_id;
            owner.occupied |= 1 << symbol.unit_index;
        }
    }
    owners
}

/// A package key excludes placement and reference but includes pinned part, package, fields and groups.
/// Symbol digests are computed once per shared definition, not once per placement.
fn match_key(project: &Project, id: u64, digests: &mut BTreeMap<*const crate::SymbolDefinition, String>) -> Result<Vec<u8>, CommandError> {
    let part = &project.components[&id];
    let digest = match digests.entry(Arc::as_ptr(&part.definition)) {
        std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(crate::library::sources::symbol_digest(&part.definition).map_err(CommandError::InvalidParams)?)
        }
    };
    let assignment =
        project.package_assignments().get(&id).map(|a| (&a.library, &a.definition, a.definition_revision, &a.mapping, &a.unconnected_pads));
    serde_json::to_vec(&(
        &*digest,
        &part.value,
        &part.metadata,
        part.part.as_ref().map(crate::part_instances::Selection::record),
        assignment,
        project.part_group_memberships().groups_of(id),
    ))
    .map_err(|error| CommandError::InvalidParams(error.to_string()))
}

/// Exclude locked owners and ones whose authored terminal intent cannot be transferred implicitly.
/// A physical footprint without an accepted counterpart cannot be retired through board review.
fn skip_reason(project: &Project, unreviewable: &BTreeSet<u64>, id: u64) -> Option<&'static str> {
    let part = &project.components[&id];
    if part.designator_locked() {
        Some("designator_locked")
    } else if !part.no_connects.is_empty() {
        Some("terminal_intent")
    } else if unreviewable.contains(&id) {
        Some("board_review_required")
    } else {
        None
    }
}

/// Insert free unit slots for a package; the priority flag distinguishes established packages from loose units.
fn offer(slots: &mut Slots, id: u64, owner: &Owner<'_>) {
    let loose = owner.occupied.count_ones() == 1;
    for slot in (0..owner.first.definition.units.len()).filter(|slot| owner.occupied & (1 << slot) == 0) {
        slots.entry(slot).or_default().insert((loose, id));
    }
}

/// Match the selected drawing with another unit only when each pin has a unique exact role and
/// position. Same-token matches take precedence for pins that are shared across unit drawings.
fn pin_map(symbol: &SymbolPlacement, slot: usize) -> Option<Vec<(Terminal, Terminal)>> {
    let from = symbol.unit_definition();
    let to = &symbol.definition.units[slot];
    if slot == symbol.unit_index {
        return Some(Vec::new());
    }
    if from.drawing != to.drawing || from.pins().len() != to.pins().len() {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut shared = BTreeSet::new();
    for pin in symbol.definition.units().iter().flat_map(crate::schematic::SymbolUnit::pins) {
        if !seen.insert(&pin.number) {
            shared.insert(&pin.number);
        }
    }
    let mut used = BTreeSet::new();
    from.pins()
        .iter()
        .map(|pin| {
            let compatible = |other: &&crate::schematic::PinDefinition| {
                pin.offset == other.offset
                    && pin.name == other.name
                    && pin.electrical_type == other.electrical_type
                    && pin.appearance == other.appearance
            };
            let candidate = if let Some(same) = to.pins().iter().filter(compatible).find(|other| other.number == pin.number) {
                same
            } else {
                let mut candidates = to.pins().iter().filter(compatible);
                let candidate = candidates.next()?;
                if candidates.next().is_some() {
                    return None;
                }
                candidate
            };
            // A shared supply/other aliased terminal is a specific physical pin, not a swappable role.
            if (pin.number != candidate.number && (shared.contains(&pin.number) || shared.contains(&candidate.number)))
                || !used.insert(&candidate.number)
            {
                return None;
            }
            Some((Terminal::from_number(symbol.ordinal, from, &pin.number)?, Terminal::from_number(symbol.ordinal, to, &candidate.number)?))
        })
        .collect()
}

/// Use indexed free slots rather than repeatedly searching all peers. A loose part can join an
/// established package or an earlier loose part, but an established package is never dismantled.
fn plan_moves<'a>(
    project: &Project,
    owners: &BTreeMap<u64, Owner<'a>>,
    sheet: Option<Uuid>,
    cancelled: Option<&AtomicBool>,
) -> Result<Option<(Vec<Move<'a>>, Vec<Value>)>, CommandError> {
    let is_cancelled = || cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed));
    let mut pools = BTreeMap::<Vec<u8>, Slots>::new();
    let mut loose = Vec::new();
    let mut skipped = Vec::new();
    let mut digests = BTreeMap::new();
    let unreviewable = project
        .board
        .footprints()
        .iter()
        .map(|footprint| footprint.component)
        .filter(|id| !project.board.baseline.as_ref().is_some_and(|baseline| baseline.components.contains_key(id)))
        .collect();
    for (&id, owner) in owners {
        if is_cancelled() {
            return Ok(None);
        }
        if sheet.is_some_and(|sheet| sheet != owner.sheet || owner.across_sheets) {
            continue;
        }
        if let Some(reason) = skip_reason(project, &unreviewable, id) {
            skipped.push(json!({"part":owner.first.component_id(),"reason":reason}));
            continue;
        }
        let key = match_key(project, id, &mut digests)?;
        if owner.occupied.count_ones() == 1 {
            loose.push((key, id, owner));
        } else {
            offer(pools.entry(key).or_default(), id, owner);
        }
    }
    let mut moves = Vec::new();
    for (key, id, source) in loose {
        if is_cancelled() {
            return Ok(None);
        }
        let slots = pools.entry(key).or_default();
        let mut choices: Vec<_> = slots
            .iter()
            .filter_map(|(&slot, available)| {
                let &(loose, target) = available.first()?;
                Some((loose, target, slot != source.first.unit_index, slot))
            })
            .collect();
        choices.sort_unstable();
        let chosen = choices.into_iter().find_map(|(loose, target, _, slot)| pin_map(source.first, slot).map(|pins| (loose, target, slot, pins)));
        if let Some((loose, target, slot, pins)) = chosen {
            slots.entry(slot).or_default().remove(&(loose, target));
            moves.push(Move { symbol: source.first, sheet: source.sheet, target, slot, pins });
        } else {
            offer(slots, id, source);
        }
    }
    Ok(Some((moves, skipped)))
}

/// Apply only to a detached candidate. Rewrite exact wire and generated-leg endpoints while keeping
/// stable geometry; retire desired package choices without changing accepted board placements.
fn transfer(before: &Project, moves: &[Move<'_>]) -> (Project, BTreeMap<Terminal, Terminal>) {
    let by_symbol: BTreeMap<_, _> = moves.iter().map(|change| (change.symbol.ordinal, change)).collect();
    let retired: BTreeSet<_> = moves.iter().map(|change| change.symbol.component).collect();
    let terminals: BTreeMap<_, _> = moves.iter().flat_map(|change| change.pins.iter().copied()).filter(|(from, to)| from != to).collect();
    let remap = |end| match end {
        WireEndpoint::Terminal(pin) => WireEndpoint::Terminal(terminals.get(&pin).copied().unwrap_or(pin)),
        _ => end,
    };
    let mut next = before.clone();
    let units = Arc::make_mut(&mut next.units);
    for sheet in Arc::make_mut(&mut next.sheets) {
        if !sheet.symbols.iter().any(|symbol| by_symbol.contains_key(&symbol.ordinal)) {
            continue;
        }
        for symbol in Arc::make_mut(&mut sheet.symbols) {
            if let Some(change) = by_symbol.get(&symbol.ordinal) {
                units.remove(&(symbol.component, symbol.unit.clone()));
                symbol.component = change.target;
                symbol.definition = before.components[&change.target].definition.clone();
                symbol.unit_index = change.slot;
                symbol.unit = symbol.definition.units[change.slot].unit_id().to_owned();
                units.insert((symbol.component, symbol.unit.clone()), symbol.ordinal);
            }
        }
        for wire in Arc::make_mut(&mut sheet.wires) {
            wire.endpoints = wire.endpoints.map(remap);
            // Save and reopen use canonical endpoint order only for undirected links.
            if wire.bends.is_none() {
                wire.endpoints.sort_unstable();
            }
        }
        for leg in Arc::make_mut(&mut sheet.generated_legs) {
            [leg.end, leg.other] = [leg.end, leg.other].map(remap);
        }
        sheet.connectivity = Arc::default();
    }
    let drops_assignments = before.package_assignments().keys().any(|id| retired.contains(id));
    if before.board_target().is_some() {
        if drops_assignments || before.board.baseline.is_some() {
            let board = Arc::make_mut(&mut next.board);
            for &id in &retired {
                board.retain_component_display(id, before.components.get(&id));
            }
            if drops_assignments {
                board.assignments.retain(|id, _| !retired.contains(id));
                let used = board.used_package_keys();
                board.definitions.retain(|key, _| used.contains(key));
            }
        }
    } else if drops_assignments {
        let packages = Arc::make_mut(&mut next.schematic_packages);
        packages.assignments.retain(|id, _| !retired.contains(id));
        let used: BTreeSet<_> = packages.assignments.values().map(crate::board::assignment::Assignment::key).collect();
        packages.definitions.retain(|key, _| used.contains(key));
    }
    Arc::make_mut(&mut next.components).retain(|id, _| !retired.contains(id));
    (next, terminals)
}
