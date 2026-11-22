//! Benchmark suite for large boards: each named action's nearest-rank p50/p95 against a
//! committed budget table. Budgeted boundaries are CPU proxies for the proposed #237 gates;
//! a verdict describes one machine and build and never gates CI.
use super::{Environment, Metric, Report, Result, clock_ms, load_artifact, replay::ROUTE_REVERSAL_METRIC, run};
use pcbcraft_engine::persistence::{decode_packed, encode_documents, encode_packed};
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// Report schema for budget assessments. It embeds the CPU report it was computed from.
pub const SCHEMA: &str = "pcbcraft-performance-budgets/1";

/// Default maximum allowed nearest-rank p95 slowdown percentage before the regression gate fails.
pub const DEFAULT_MAX_REGRESSION_PCT: f64 = 20.0;

/// The save boundary measured by the suite in addition to the shared CPU report.
const SAVE_METRIC: &str = "encode.packed";

/// Which recorded boundary measures an action.
#[derive(Clone, Copy, Debug)]
pub enum Boundary {
    /// One exact metric id from the CPU report.
    Metric(&'static str),
    /// All samples of every metric whose id starts and ends with these strings, pooled into
    /// one distribution (for example, every pointer move of an active route).
    Pooled { prefix: &'static str, suffix: &'static str },
}

/// One action and its strict p95 budget: a pass needs p95 below `p95_ms`. `p95_ms` is `None`
/// when no target has been agreed; `basis` says where the target comes from and what the
/// measured boundary can and cannot certify.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub action: &'static str,
    pub boundary: Boundary,
    pub p95_ms: Option<f64>,
    pub basis: &'static str,
}

/// Basis for actions without an agreed target; they report quantiles only.
const NO_TARGET: &str = "no target yet";

/// The proposed #237 gates (`docs/performance-testing.md`, "Agent acceptance instructions"):
/// routing input p95 < 4 ms and commit p95 < 100 ms. Both boundaries here are CPU proxies,
/// so a pass is evidence about service cost, not about the window or a future shove commit.
/// Open, save, DRC and connectivity have no agreed target and report quantiles only.
pub const BUDGETS: &[Budget] = &[
    Budget { action: "open", boundary: Boundary::Metric("decode.cold"), p95_ms: None, basis: NO_TARGET },
    Budget { action: "save", boundary: Boundary::Metric(SAVE_METRIC), p95_ms: None, basis: NO_TARGET },
    Budget { action: "drc", boundary: Boundary::Metric("drc.synchronous"), p95_ms: None, basis: NO_TARGET },
    Budget { action: "connectivity", boundary: Boundary::Metric("electrical_connectivity.first"), p95_ms: None, basis: NO_TARGET },
    Budget {
        action: "move_commit",
        boundary: Boundary::Metric("command.move_tracks"),
        p95_ms: Some(100.0),
        basis: "proposed #237 commit target; validated track-edit proxy, does not qualify walkaround or shove commits",
    },
    Budget {
        action: "routing_input",
        boundary: Boundary::Metric(ROUTE_REVERSAL_METRIC),
        p95_ms: Some(4.0),
        basis: "proposed #237 routing-input target; CPU service-cost proxy, does not certify the window input budget",
    },
];

/// Targets the suite cannot measure yet; they are listed so a report never implies them.
const UNMEASURED: &[&str] = &[
    "memory: process RSS on the envelope board (use the resource collector)",
    "agent bulk change of 10,000 objects",
    "multi-core speedup of DRC, pour fill and connectivity",
    "window routing input p95 < 4 ms (CPU replay is a proxy)",
    "valid visible preview p95 < 16.7 ms / p99 < 33 ms (CPU replay is a proxy)",
];

/// The outcome for one action. A missing boundary is never turned into a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    MeasuredPass,
    MeasuredMiss,
    Unmeasured,
    NoBudget,
}

/// One row of the assessment: the action's quantiles next to its budget.
#[derive(Clone, Debug, Serialize)]
pub struct Assessment {
    pub action: String,
    pub boundary: String,
    pub samples: usize,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub budget_p95_ms: Option<f64>,
    pub budget_basis: &'static str,
    pub verdict: Verdict,
}

/// Collect the samples an action's boundary selects from the given metrics.
fn samples_for(boundary: Boundary, metrics: &[Metric]) -> Vec<f64> {
    metrics
        .iter()
        .filter(|metric| match boundary {
            Boundary::Metric(id) => metric.id == id,
            Boundary::Pooled { prefix, suffix } => metric.id.starts_with(prefix) && metric.id.ends_with(suffix),
        })
        .flat_map(|metric| metric.samples_ms.iter().copied())
        .collect()
}

/// Compare each budgeted action's nearest-rank p95 with its budget; only a p95 strictly below
/// the budget passes. An action whose boundary recorded no samples is `Unmeasured`, whatever
/// its budget.
pub fn assess(budgets: &[Budget], metrics: &[Metric]) -> Vec<Assessment> {
    budgets
        .iter()
        .map(|budget| {
            let samples = samples_for(budget.boundary, metrics);
            let count = samples.len();
            let (p50, p95) = if count == 0 {
                (None, None)
            } else {
                let metric = Metric::new(budget.action, samples);
                (Some(metric.p50_ms), Some(metric.p95_ms))
            };
            let verdict = match (p95, budget.p95_ms) {
                (None, _) => Verdict::Unmeasured,
                (Some(_), None) => Verdict::NoBudget,
                (Some(actual), Some(limit)) if actual < limit => Verdict::MeasuredPass,
                (Some(_), Some(_)) => Verdict::MeasuredMiss,
            };
            let boundary = match budget.boundary {
                Boundary::Metric(id) => id.to_owned(),
                Boundary::Pooled { prefix, suffix } => format!("{prefix}*{suffix}"),
            };
            Assessment {
                action: budget.action.into(),
                boundary,
                samples: count,
                p50_ms: p50,
                p95_ms: p95,
                budget_p95_ms: budget.p95_ms,
                budget_basis: budget.basis,
                verdict,
            }
        })
        .collect()
}

/// Comparison of one action's nearest-rank p95 between a baseline and a candidate run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegressionRow {
    pub action: String,
    pub boundary: String,
    pub baseline_p95_ms: f64,
    pub candidate_p95_ms: f64,
    /// Percentage change from baseline (`None` when baseline is zero and candidate is positive).
    pub change_pct: Option<f64>,
    pub regressed: bool,
}

/// Result of gating a candidate assessment against a matched baseline assessment.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegressionReport {
    pub max_regression_pct: f64,
    pub passed: bool,
    pub regressed_actions: Vec<String>,
    pub actions: Vec<RegressionRow>,
}

/// Gate a candidate assessment against a matched baseline assessment using nearest-rank p95.
/// Fails any action whose p95 slows down by strictly more than `max_regression_pct` percent,
/// or increases from a zero baseline. Rejects invalid thresholds, mismatched action lists,
/// duplicate action names, and unmeasured or nonfinite p95 values.
pub fn check_regression(baseline: &[Assessment], candidate: &[Assessment], max_regression_pct: f64) -> Result<RegressionReport> {
    if !max_regression_pct.is_finite() || max_regression_pct < 0.0 {
        return Err("max_regression_pct must be finite and non-negative".into());
    }
    if baseline.is_empty() {
        return Err("baseline assessment has no actions".into());
    }
    let mut candidate_by_action = BTreeMap::new();
    for row in candidate {
        if candidate_by_action.insert(row.action.as_str(), row).is_some() {
            return Err(format!("duplicate candidate action: {}", row.action));
        }
    }
    let mut seen_baseline = BTreeSet::new();
    let mut rows = Vec::with_capacity(baseline.len());
    let mut regressed_actions = Vec::new();
    for base in baseline {
        if !seen_baseline.insert(base.action.as_str()) {
            return Err(format!("duplicate baseline action: {}", base.action));
        }
        let Some(cand) = candidate_by_action.remove(base.action.as_str()) else {
            return Err(format!("candidate assessment missing action: {}", base.action));
        };
        if base.boundary != cand.boundary {
            return Err(format!("boundary mismatch for action {}: {} vs {}", base.action, base.boundary, cand.boundary));
        }
        let Some(base_p95) = base.p95_ms.filter(|value| value.is_finite() && *value >= 0.0) else {
            return Err(format!("baseline action {} is unmeasured or nonfinite", base.action));
        };
        let Some(cand_p95) = cand.p95_ms.filter(|value| value.is_finite() && *value >= 0.0) else {
            return Err(format!("candidate action {} is unmeasured or nonfinite", cand.action));
        };
        let (change_pct, regressed) = if base_p95 == 0.0 {
            if cand_p95 == 0.0 { (Some(0.0), false) } else { (None, true) }
        } else {
            let pct = ((cand_p95 - base_p95) / base_p95) * 100.0;
            (Some(pct), pct > max_regression_pct)
        };
        if regressed {
            regressed_actions.push(base.action.clone());
        }
        rows.push(RegressionRow {
            action: base.action.clone(),
            boundary: base.boundary.clone(),
            baseline_p95_ms: base_p95,
            candidate_p95_ms: cand_p95,
            change_pct,
            regressed,
        });
    }
    if let Some((extra, _)) = candidate_by_action.into_iter().next() {
        return Err(format!("candidate assessment has unexpected action: {extra}"));
    }
    Ok(RegressionReport { max_regression_pct, passed: regressed_actions.is_empty(), regressed_actions, actions: rows })
}

/// Run the shared CPU workloads plus the verified packed save and return the raw report and
/// combined metrics list.
pub(super) fn collect_metrics(artifact_json: &str, environment: Environment, iterations: usize) -> Result<(Report, Vec<Metric>)> {
    let mut report = run(artifact_json, environment, iterations)?;
    let artifact = load_artifact(artifact_json)?;
    let expected = encode_documents(decode_packed(&artifact.project)?.project)?;
    let mut save_samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let project = decode_packed(&artifact.project)?.project;
        let start = clock_ms();
        let bytes = encode_packed(project)?;
        save_samples.push(clock_ms() - start);
        let reopened = decode_packed(&bytes)?;
        if reopened.needs_manifest || reopened.documents != expected || encode_documents(reopened.project)? != expected {
            return Err("packed save did not reopen with equivalent project contents".into());
        }
    }
    let mut metrics = std::mem::take(&mut report.metrics);
    metrics.push(Metric::new(SAVE_METRIC, save_samples));
    Ok((report, metrics))
}

/// Run the shared CPU workloads on one saved artifact, time a packed save of the same
/// project, and assess every action against `BUDGETS`. The timed save must reopen with
/// equivalent project contents and a valid manifest; that check stays outside the timed
/// boundary, and the report retains the original input archive hash.
pub fn suite(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    let (report, metrics) = collect_metrics(artifact_json, environment, iterations)?;
    let actions = assess(BUDGETS, &metrics);
    Ok(json!({
        "schema": SCHEMA,
        "environment": report.environment,
        "target": report.target,
        "debug_assertions": report.debug_assertions,
        "artifact_sha256": report.artifact_sha256,
        "project_sha256": report.project_sha256,
        "recipe": report.recipe,
        "iterations": report.iterations,
        "geometry": report.geometry,
        "actions": actions,
        "metrics": metrics,
        "unmeasured": report.unmeasured.iter().map(String::as_str).chain(UNMEASURED.iter().copied()).collect::<Vec<_>>(),
    }))
}
