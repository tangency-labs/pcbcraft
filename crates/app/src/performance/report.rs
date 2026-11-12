//! Versioned report data with raw samples and nearest-rank quantiles.
use super::Result;
use serde::{Deserialize, Serialize};

/// Operator-supplied environment facts. Keep runs separate when any of these differ.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub revision: String,
    pub hardware: String,
    pub os: String,
    pub runtime: String,
    pub compiler: String,
    pub profile: String,
    pub power: String,
    pub notes: String,
}

impl Environment {
    /// Reject unlabeled timing evidence; callers must identify unknown conditions explicitly.
    pub(super) fn validate(&self) -> Result<()> {
        if [&self.revision, &self.hardware, &self.os, &self.runtime, &self.compiler, &self.profile, &self.power].iter().any(|s| s.trim().is_empty()) {
            return Err("environment fields must be nonempty; state unknown conditions explicitly".into());
        }
        Ok(())
    }
}

/// One named boundary. Times are milliseconds, with raw samples retained for reanalysis.
#[derive(Debug, Serialize, Deserialize)]
pub struct Metric {
    pub id: String,
    pub samples_ms: Vec<f64>,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}
impl Metric {
    /// Nearest-rank percentiles include the slowest sample for small sample sets.
    pub(super) fn new(id: &str, samples_ms: Vec<f64>) -> Self {
        let mut sorted = samples_ms.clone();
        sorted.sort_by(f64::total_cmp);
        let quantile = |percent: usize| sorted.get((sorted.len() * percent).div_ceil(100).saturating_sub(1)).copied().unwrap_or(0.0);
        Self {
            id: id.into(),
            p50_ms: quantile(50),
            p95_ms: quantile(95),
            p99_ms: quantile(99),
            max_ms: sorted.last().copied().unwrap_or(0.0),
            samples_ms,
        }
    }
}

/// Measurement scope is explicit; missing capabilities cannot silently pass a budget.
#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub environment: Environment,
    pub target: String,
    pub debug_assertions: bool,
    pub artifact_sha256: String,
    pub project_sha256: String,
    pub recipe: String,
    pub trace_id: String,
    pub iterations: usize,
    pub warmup_frames: usize,
    pub geometry: serde_json::Value,
    pub metrics: Vec<Metric>,
    pub unmeasured: Vec<String>,
}
