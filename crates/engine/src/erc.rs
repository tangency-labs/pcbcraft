//! The shared electrical rules check (ERC): a fixed rule registry and its on-demand report.
//!
//! A run evaluates every registered rule, in registry order, on one immutable project snapshot
//! and stamps the result with that project's identity and revision. Running never changes the
//! project, its history or its revision. The session keeps the latest run so the schematic
//! editor and MCP read the same results; once the project changes, that run is reported as
//! stale and only a new run produces current results. The first rule reuses the existing
//! no-connect evaluator ([`Project::check_no_connects`]) unchanged, so its finding keys and the
//! `schematic.check_no_connects` response stay as they were.

use crate::{CommandError, Project, Session, no_connect};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// Run every registered rule on the current revision and keep the result as the session's report.
pub const RUN: &str = "schematic.run_erc";
/// Read the session's latest report and whether it still matches the current revision.
pub const REPORT: &str = "schematic.erc_report";

/// One registered rule: a stable machine identity, a display title and its evaluator.
pub struct Rule {
    /// Stable rule identity; never reused for a different check.
    pub id: &'static str,
    /// Plain label for reports and the editor.
    pub title: &'static str,
    evaluate: fn(&Project, u64) -> RuleResult,
}

impl Rule {
    /// Evaluate only this rule on `project`, stamped with `revision`. Read-only.
    pub fn evaluate(&self, project: &Project, revision: u64) -> RuleResult {
        (self.evaluate)(project, revision)
    }
}

/// The no-connect rule. `schematic.check_no_connects` evaluates it alone through the registry.
pub const NO_CONNECT: Rule = Rule { id: "no_connect", title: "No-connect and unconnected pins", evaluate: no_connect_rule };

/// The registry, in evaluation and report order. Later rules append here.
static RULES: &[Rule] = &[NO_CONNECT];

/// Every registered rule, in the order a run evaluates and reports them.
pub fn rules() -> &'static [Rule] {
    RULES
}

/// Reuse the existing focused evaluator; its report, keys and limits are unchanged.
fn no_connect_rule(project: &Project, revision: u64) -> RuleResult {
    RuleResult::NoConnectCheck(project.check_no_connects(revision))
}

/// The output of one rule, in that rule's existing result shape.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleResult {
    /// The no-connect evaluator's report, exactly as `schematic.check_no_connects` returns it.
    NoConnectCheck(no_connect::Report),
}

impl RuleResult {
    /// Exact finding count for this rule, including findings beyond the detailed list.
    pub fn total_findings(&self) -> usize {
        match self {
            Self::NoConnectCheck(report) => report.total_findings,
        }
    }

    /// Whether the detailed lists omit findings or markers that the totals count.
    pub fn truncated(&self) -> bool {
        match self {
            Self::NoConnectCheck(report) => report.truncated,
        }
    }

    /// This result as the no-connect report, when it is the no-connect rule's result.
    pub fn no_connect(&self) -> Option<&no_connect::Report> {
        match self {
            Self::NoConnectCheck(report) => Some(report),
        }
    }
}

/// One rule's entry in a report.
#[derive(Clone, Debug, Serialize)]
pub struct RuleRun {
    pub rule: &'static str,
    pub title: &'static str,
    pub total_findings: usize,
    pub truncated: bool,
    pub result: RuleResult,
}

/// One on-demand run over a single project snapshot. Two runs on the same revision serialize
/// to identical bytes: rules follow registry order and each rule orders its own findings.
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub project: uuid::Uuid,
    pub revision: u64,
    pub rules: Vec<RuleRun>,
    pub total_findings: usize,
}

/// Whether a report describes the project as it is now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The report ran on the current project and revision.
    Current,
    /// The project changed after the run; its findings must not be used until a rerun.
    Stale,
}

impl Report {
    /// Current only for the same project at the same revision. Session revisions never repeat,
    /// so a project edit, or an Undo or Redo of one, after the run always makes it stale.
    pub fn status(&self, project: uuid::Uuid, revision: u64) -> Status {
        if self.project == project && self.revision == revision { Status::Current } else { Status::Stale }
    }

    /// The no-connect rule's report, which the schematic findings list navigates.
    pub fn no_connect(&self) -> Option<&no_connect::Report> {
        self.rules.iter().find_map(|run| run.result.no_connect())
    }
}

/// Evaluate every registered rule on `project`, stamped with `revision`. Read-only.
pub fn run(project: &Project, revision: u64) -> Report {
    let rules = RULES
        .iter()
        .map(|rule| {
            let result = rule.evaluate(project, revision);
            RuleRun { rule: rule.id, title: rule.title, total_findings: result.total_findings(), truncated: result.truncated(), result }
        })
        .collect::<Vec<_>>();
    let total_findings = rules.iter().map(|run| run.total_findings).sum();
    Report { project: project.id(), revision, rules, total_findings }
}

impl Session {
    /// The latest report of this opening and its status against the current project, if any.
    /// Opening a project discards the previous opening's report; edits keep it, as stale.
    pub fn erc_report(&self) -> Option<(Arc<Report>, Status)> {
        let report = self.erc.clone()?;
        let project = self.project()?;
        let status = report.status(project.id(), self.revision());
        Some((report, status))
    }

    /// Run the registry on the current revision and keep the result. Changes no project state,
    /// history or revision; the caller has already checked scope and parameters.
    pub(crate) fn run_erc(&mut self) -> Result<(), CommandError> {
        let report = run(self.open_project()?, self.revision());
        self.erc = Some(Arc::new(report));
        Ok(())
    }

    /// Answer both ERC commands from the kept report. Only a current report carries findings;
    /// a stale one names the revision it ran on and the rerun command, and nothing else.
    pub(crate) fn erc_reply(&self) -> Result<Value, CommandError> {
        self.open_project()?;
        Ok(match self.erc_report() {
            None => json!({"revision": self.revision(), "status": "not_run", "rerun": RUN}),
            Some((report, Status::Stale)) => {
                json!({"revision": self.revision(), "status": Status::Stale, "report_revision": report.revision, "rerun": RUN})
            }
            Some((report, Status::Current)) => json!({"revision": self.revision(), "status": Status::Current, "erc_report": report}),
        })
    }
}

#[cfg(test)]
mod tests;
