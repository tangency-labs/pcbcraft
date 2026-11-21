//! Benchmark suite for large boards: each named action's nearest-rank p50/p95 against a
//! committed budget table. Budgets are estimated targets until measured on named machines;
//! a verdict describes one machine and build and never gates CI.
use super::{Environment, Metric, Result, clock_ms, digest, load_artifact, run};
use pcbcraft_engine::persistence::{decode_packed, encode_packed};
use serde::Serialize;
use serde_json::json;

/// Report schema for budget assessments. It embeds the CPU report it was computed from.
pub const SCHEMA: &str = "pcbcraft-performance-budgets/1";

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

/// One action and its p95 budget. `p95_ms` is `None` when no target has been set yet.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub action: &'static str,
    pub boundary: Boundary,
    pub p95_ms: Option<f64>,
}

/// Estimated targets for a board at the envelope size (`envelope-v1`). They are not
/// measured results; `docs/performance-testing.md` lists the same table. Routing input uses
/// the proposed #237 routing-input target on the CPU replay of pointer moves during an
/// active route; the 16.7 ms visible-preview target needs a rendered window and stays unmeasured.
pub const BUDGETS: &[Budget] = &[
    Budget { action: "open", boundary: Boundary::Metric("decode.cold"), p95_ms: Some(3_000.0) },
    Budget { action: "save", boundary: Boundary::Metric(SAVE_METRIC), p95_ms: Some(2_000.0) },
    Budget { action: "drc", boundary: Boundary::Metric("drc.synchronous"), p95_ms: Some(10_000.0) },
    Budget { action: "connectivity", boundary: Boundary::Metric("electrical_connectivity.first"), p95_ms: None },
    Budget { action: "move_commit", boundary: Boundary::Metric("command.move_tracks"), p95_ms: Some(100.0) },
    Budget { action: "routing_input", boundary: Boundary::Pooled { prefix: "replay.reverse.", suffix: ".ui" }, p95_ms: Some(4.0) },
];

/// Targets the suite cannot measure yet; they are listed so a report never implies them.
const UNMEASURED: &[&str] = &[
    "memory: process RSS on the envelope board (use the resource collector)",
    "agent bulk change of 10,000 objects",
    "multi-core speedup of DRC, pour fill and connectivity",
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
#[derive(Debug, Serialize)]
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

/// Compare each budgeted action's nearest-rank p95 with its budget. An action whose boundary
/// recorded no samples is `Unmeasured`, whatever its budget.
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
                (Some(actual), Some(limit)) if actual <= limit => Verdict::MeasuredPass,
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
                budget_basis: if budget.p95_ms.is_some() { "estimated target" } else { "no target set" },
                verdict,
            }
        })
        .collect()
}

/// Run the shared CPU workloads on one saved artifact, time a packed save of the same
/// project, and assess every action against `BUDGETS`. The save output must equal the saved
/// archive bytes; that check stays outside the timed boundary.
pub fn suite(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    let report = run(artifact_json, environment, iterations)?;
    let artifact = load_artifact(artifact_json)?;
    let mut save_samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let project = decode_packed(&artifact.project)?.project;
        let start = clock_ms();
        let bytes = encode_packed(project)?;
        save_samples.push(clock_ms() - start);
        if digest(&bytes) != artifact.project_sha256 {
            return Err("packed save differs from the saved archive bytes".into());
        }
    }
    let mut metrics = report.metrics;
    metrics.push(Metric::new(SAVE_METRIC, save_samples));
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
