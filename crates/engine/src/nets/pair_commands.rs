//! Authored relationship edits leave net identity and connectivity untouched.

use super::{MAX_PAIRS, MAX_REGISTRY_BYTES, Pair, Registry};
use crate::{
    CommandError, Project, Session,
    persistence::{PairDocumentBudget, PairDocumentProbe, document_row_framing_bytes},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{io, sync::Arc};
use uuid::Uuid;

/// Create a relationship with two explicitly ordered persistent members.
pub const CREATE_PAIR: &str = "nets.create_pair";
/// Replace a relationship's name and members without replacing its identity or review state.
pub const UPDATE_PAIR: &str = "nets.update_pair";
/// Remove just the relationship; neither member net is deleted.
pub const REMOVE_PAIR: &str = "nets.remove_pair";

/// Identify commands that use the shared deferred pair preparation path.
pub fn is_pair_edit(command: &str) -> bool {
    matches!(command, CREATE_PAIR | UPDATE_PAIR | REMOVE_PAIR)
}

/// Allocate a candidate pair UUID for an editor or a replayable command. The engine still
/// checks that the ID is unused in the target registry when applying the command.
pub fn new_pair_id() -> Result<Uuid, CommandError> {
    crate::fresh_id()
}

impl Project {
    /// Current differential relationships, in stable UUID order, including review intent.
    pub fn differential_pairs(&self) -> &[Arc<Pair>] {
        &self.net_registry.pairs
    }
}

/// Read a required UUID after command schema validation without relying on unchecked indexing.
fn id(params: &Value, key: &str) -> Result<Uuid, CommandError> {
    serde_json::from_value(params[key].clone()).map_err(|_| CommandError::InvalidParams(format!("Expected a UUID for {key}.")))
}

/// Immutable authority for one preparation, including same-revision save-metadata changes.
#[derive(Clone)]
pub struct PairEditSource {
    session: Arc<()>,
    project: Arc<Project>,
    revision: u64,
}

impl PairEditSource {
    /// Check origin, revision and exact captured snapshot just before publishing.
    pub fn is_current(&self, session: &Session) -> bool {
        Arc::ptr_eq(&self.session, &session.clearance_identity)
            && self.revision == session.revision()
            && session.project.as_ref().is_some_and(|project| Arc::ptr_eq(project, &self.project))
    }
}

/// Owns an unpublished pair candidate. Only the session can adopt it, after rechecking its source.
pub struct PreparedPairEdit {
    source: PairEditSource,
    command: &'static str,
    params: Value,
    edit: Edit,
    budget: Option<PairDocumentBudget>,
}

/// Disposable draft and preview records shared by both clients.
struct Edit {
    replacement: Option<Arc<Project>>,
    before: Option<Arc<Pair>>,
    after: Option<Arc<Pair>>,
}

impl Edit {
    /// Describe the replacement in the existing dry-run response shape.
    fn preview(&self, revision: u64) -> Value {
        json!({"revision":revision,"pair_change":{
            "changed":self.before != self.after,"before":self.before,"after":self.after
        }})
    }
}

/// Build just the authored pair change; this never publishes any part of the candidate.
fn pair_edit(project: &Project, command: &str, params: &Value) -> Result<Edit, CommandError> {
    let pair_id = id(params, "pair")?;
    let registry = &project.net_registry;
    let index = registry.pairs.binary_search_by_key(&pair_id, |pair| pair.id);
    let before = index.ok().map(|index| registry.pairs[index].clone());
    if command == CREATE_PAIR {
        if index.is_ok() || pair_id.is_nil() || registry.record(pair_id).is_some() {
            return Err(CommandError::InvalidParams("Choose an unused, non-nil pair UUID distinct from net identities.".into()));
        }
        if registry.pairs.len() >= MAX_PAIRS {
            return Err(CommandError::CapExceeded("Differential pair registry is full.".into()));
        }
    } else if index.is_err() {
        return Err(CommandError::NotFound("Unknown differential pair.".into()));
    }
    let after = if command == REMOVE_PAIR {
        None
    } else {
        let name = params["name"].as_str().ok_or_else(|| CommandError::InvalidParams("Expected a pair name.".into()))?;
        Some(Arc::new(Pair {
            id: pair_id,
            name: name.into(),
            positive: id(params, "positive")?,
            negative: id(params, "negative")?,
            state: before.as_ref().map_or_else(Default::default, |pair| pair.state.clone()),
        }))
    };
    let replacement = if before == after {
        None
    } else {
        let mut registry = (**registry).clone();
        let pairs = Arc::make_mut(&mut registry.pairs);
        match (index, &after) {
            (Ok(index), Some(pair)) => pairs[index] = pair.clone(),
            (Err(index), Some(pair)) => pairs.insert(index, pair.clone()),
            (Ok(index), None) => {
                pairs.remove(index);
            }
            (Err(_), None) => return Err(CommandError::NotFound("Unknown differential pair.".into())),
        }
        registry.validate_pairs().map_err(CommandError::InvalidParams)?;
        let mut candidate = project.clone();
        candidate.net_registry = Arc::new(registry);
        Some(Arc::new(candidate))
    };
    Ok(Edit { replacement, before, after })
}

/// Measure a single compact JSON value without allocating its output.
#[derive(Default)]
struct ByteCounter(usize);

impl io::Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Measure the registry's compact capacity and the native size of the pair rows in the same
/// pass. Both envelopes use the same Serde representation for each individual row.
struct RegistrySizeProbe {
    row: usize,
    registry_bytes: usize,
    pair_rows: usize,
}

impl RegistrySizeProbe {
    /// Count the fixed compact registry envelope; the optional pair array is omitted when empty.
    fn new(registry: &Registry) -> Self {
        let registry_bytes = b"{\"records\":[]}".len() + if registry.has_pairs() { b",\"pairs\":[]".len() } else { 0 };
        Self { row: 0, registry_bytes, pair_rows: 0 }
    }

    /// The actual compact payload size of one net or pair in either storage representation.
    fn row_bytes(row: &impl Serialize) -> Result<usize, CommandError> {
        let mut count = ByteCounter::default();
        serde_json::to_writer(&mut count, row).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        Ok(count.0)
    }

    /// Advance by at most `rows` authored records, checking the same independent registry cap.
    /// Return the exact framed native pair-row byte count when every row has been measured.
    fn step(&mut self, registry: &Registry, rows: usize) -> Result<Option<usize>, CommandError> {
        let net_count = registry.records.len();
        let total = net_count + registry.pairs.len();
        for _ in 0..rows.min(total - self.row) {
            if self.row < net_count {
                self.registry_bytes += Self::row_bytes(&registry.records[self.row])? + usize::from(self.row != 0);
            } else {
                let pair_index = self.row - net_count;
                let size = Self::row_bytes(&registry.pairs[pair_index])?;
                self.registry_bytes += size + usize::from(pair_index != 0);
                self.pair_rows += size + document_row_framing_bytes(pair_index == 0);
            }
            if self.registry_bytes > MAX_REGISTRY_BYTES {
                return Err(CommandError::CapExceeded("Net registry exceeds its byte limit.".into()));
            }
            self.row += 1;
        }
        Ok((self.row == total).then_some(self.pair_rows))
    }
}

/// Prepare a pair change on a captured snapshot, including exact cold-path native byte limits.
/// The native host can schedule it on a worker; a single-threaded host can yield between batches.
pub struct PairEditJob {
    source: PairEditSource,
    command: &'static str,
    params: Value,
    cached: Option<PairDocumentBudget>,
    registry_size: Option<(Edit, RegistrySizeProbe)>,
    probe: Option<(Edit, Box<PairDocumentProbe>)>,
    result: Option<Result<PreparedPairEdit, CommandError>>,
    done: bool,
}

impl PairEditJob {
    /// Snapshot authority to check while the job is pending, before dropping obsolete work.
    pub fn source(&self) -> &PairEditSource {
        &self.source
    }

    /// Advance the bounded preparation phase or one batch of the native Save writer.
    /// A zero budget performs no work. No session, history or editor state is accessed.
    pub fn step(&mut self, records: usize) -> bool {
        if self.done || records == 0 {
            return self.done;
        }
        if let Some(result) = self.advance(records).transpose() {
            self.done = true;
            self.result = Some(result);
        }
        self.done
    }

    /// Expose per-poll registry sizing progress to deterministic work-budget tests.
    #[cfg(test)]
    pub(super) fn measured_registry_rows(&self) -> Option<usize> {
        self.registry_size.as_ref().map(|(_, probe)| probe.row)
    }

    /// Compare the incremental counters with persisted envelopes in the regression tests.
    #[cfg(test)]
    pub(super) fn final_registry_sizes(registry: &Registry) -> Result<(usize, usize), CommandError> {
        let mut size = RegistrySizeProbe::new(registry);
        size.step(registry, registry.records.len() + registry.pairs.len())?;
        Ok((size.registry_bytes, size.pair_rows))
    }

    /// Take the terminal candidate or typed refusal once; a pending job produces no output.
    ///
    /// # Errors
    /// Invalid pair data or native document capacity can reject the candidate.
    pub fn take_result(&mut self) -> Result<Option<PreparedPairEdit>, CommandError> {
        self.result.take().transpose()
    }

    /// Meter registry byte checks before reusing exact headroom or advancing the native writer.
    fn advance(&mut self, records: usize) -> Result<Option<PreparedPairEdit>, CommandError> {
        let (edit, budget) = if let Some((edit, mut probe)) = self.probe.take() {
            let Some(budget) = probe.step(records)? else {
                self.probe = Some((edit, probe));
                return Ok(None);
            };
            (edit, Some(budget))
        } else if let Some((edit, mut size)) = self.registry_size.take() {
            let candidate = edit.replacement.as_ref().ok_or_else(|| CommandError::Unavailable("Pair size check lost its candidate.".into()))?;
            let Some(row_bytes) = size.step(&candidate.net_registry, records)? else {
                self.registry_size = Some((edit, size));
                return Ok(None);
            };
            let has_pairs = candidate.net_registry.has_pairs();
            let cached = self.cached.filter(|budget| budget.has_pairs == has_pairs && row_bytes <= budget.pair_rows);
            if cached.is_none() {
                let probe = Box::new(PairDocumentProbe::new(candidate.clone(), row_bytes));
                self.probe = Some((edit, probe));
                return Ok(None);
            }
            (edit, cached)
        } else {
            let edit = pair_edit(&self.source.project, self.command, &self.params)?;
            if let Some(candidate) = &edit.replacement {
                let size = RegistrySizeProbe::new(&candidate.net_registry);
                self.registry_size = Some((edit, size));
                return Ok(None);
            }
            (edit, None)
        };
        Ok(Some(PreparedPairEdit { source: self.source.clone(), command: self.command, params: std::mem::take(&mut self.params), edit, budget }))
    }

    /// A headless command has no editor throughput or request lease to protect.
    fn finish(mut self) -> Result<PreparedPairEdit, CommandError> {
        while !self.step(256) {}
        self.take_result()?.ok_or_else(|| CommandError::Unavailable("Pair preparation ended without a result.".into()))
    }
}

impl Session {
    /// Validate command admission and retain only a shared snapshot and a small request.
    /// In particular, no pair records or project documents are serialized here.
    ///
    /// # Errors
    /// A stale revision, an unavailable project or malformed parameters reject admission.
    pub fn prepare_pair_edit(&self, command: &str, params: &Value) -> Result<PairEditJob, CommandError> {
        self.refuse_stale(command, params)?;
        if !is_pair_edit(command) {
            return Err(CommandError::InvalidParams("Command does not edit a differential pair.".into()));
        }
        self.validate_project_command(command, params)?;
        self.check_revision(params)?;
        let project = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let source = PairEditSource { session: self.clearance_identity.clone(), project, revision: self.revision() };
        let command = crate::command(command).ok_or_else(|| CommandError::Unknown(command.into()))?.id;
        let key = Arc::downgrade(&source.project);
        let cached = self.pair_document_budget.lock().ok().and_then(|entry| {
            let (snapshot, budget) = entry.as_ref()?;
            snapshot.ptr_eq(&key).then_some(*budget)
        });
        Ok(PairEditJob { source, command, params: params.clone(), cached, registry_size: None, probe: None, result: None, done: false })
    }

    /// Recheck one prepared edit and either return its read-only preview or publish one change.
    /// A live host must also recheck its request lease and UI policy before calling this method.
    ///
    /// # Errors
    /// A prepared edit from a different session or obsolete snapshot returns `StaleRevision`.
    pub fn complete_pair_edit(&mut self, prepared: PreparedPairEdit, inspect: bool) -> Result<Value, CommandError> {
        if !prepared.source.is_current(self) {
            return Err(CommandError::StaleRevision { current: self.revision() });
        }
        if prepared.params["dry_run"].as_bool() == Some(true) {
            self.remember_pair_budget(prepared.budget);
            return Ok(prepared.edit.preview(self.revision()));
        }
        let revision = self.revision();
        self.publish_prepared_pair(&prepared);
        if self.revision() != revision {
            self.report_committed(prepared.command, &prepared.params);
        }
        Ok(if inspect { self.inspect() } else { json!({"revision":self.revision()}) })
    }

    /// Hold one disposable proved budget, keyed by the exact open snapshot.
    fn remember_pair_budget(&self, budget: Option<PairDocumentBudget>) {
        if let (Some(budget), Some(project), Ok(mut entry)) = (budget, &self.project, self.pair_document_budget.lock()) {
            *entry = Some((Arc::downgrade(project), budget));
        }
    }

    /// Describe an edit without publishing it or changing the revision, including no-op updates.
    pub(crate) fn preview_pair_edit(&self, command: &str, params: &Value) -> Result<Value, CommandError> {
        let prepared = self.prepare_pair_edit(command, params)?.finish()?;
        self.remember_pair_budget(prepared.budget);
        Ok(prepared.edit.preview(self.revision()))
    }

    /// Publish only the fully measured candidate and leave other document metadata unchanged.
    fn publish_prepared_pair(&mut self, prepared: &PreparedPairEdit) {
        if prepared.params["dry_run"].as_bool() != Some(true)
            && let Some(candidate) = &prepared.edit.replacement
        {
            self.publish_change(Project::clone(candidate), self.revision());
        }
        self.remember_pair_budget(prepared.budget);
    }

    /// Preserve the ordinary synchronous headless API and its observer owned by `Session::apply`.
    pub(crate) fn edit_pair(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        let prepared = self.prepare_pair_edit(command, params)?.finish()?;
        self.publish_prepared_pair(&prepared);
        Ok(())
    }
}
