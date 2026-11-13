//! Opt-in CPU and rendered diagnostics for #240 and the #237 routing/checker work.
//! Rendered GPU/canvas readback is distinct from compositor presentation or physical
//! input-to-visible latency. Ordinary application builds exclude the harness.

#[cfg(target_arch = "wasm32")]
mod browser_capture;
pub mod checker;
mod fixture;
#[cfg(not(target_arch = "wasm32"))]
mod remote;
pub mod rendered;
mod replay;
mod report;
pub mod routing;
#[cfg(not(target_arch = "wasm32"))]
pub mod watchdog;
pub use report::{Environment, Metric, Report};

use pcbcraft_engine::{
    Session,
    board::{
        clearance::{ClearanceQuery, Results},
        drc::{DEFAULT_CLEARANCE_NM, Rules},
        geometry::Contour,
        track::MOVE_TRACKS,
        view::BoardTarget,
    },
    persistence::decode_packed,
    schematic::Point,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    hint::black_box,
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, String>;
type Finding = (Option<usize>, usize, bool, bool, bool);

/// A harness envelope, not a native project format. Embedded bytes are an ordinary `.pc`
/// archive produced by the production encoder; the board and trace are explicitly addressed.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    schema: String,
    recipe: String,
    generator_revision: String,
    seed: u64,
    board: BoardTarget,
    project_sha256: String,
    project: Vec<u8>,
    trace_id: String,
    trace: Vec<replay::Step>,
}

/// Generate an original fixture artifact once and reuse its exact bytes for comparisons.
pub fn generate(recipe: &str, generator_revision: &str) -> Result<String> {
    if generator_revision.trim().is_empty() {
        return Err("generator revision is required".into());
    }
    serde_json::to_string(&fixture::generate(recipe, generator_revision)?).map_err(|e| e.to_string())
}

/// Hash exact bytes rather than recipe names, since command-created identities are random.
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Use a monotonic host clock on native targets and the browser Performance clock on wasm.
#[cfg(not(target_arch = "wasm32"))]
fn clock_ms() -> f64 {
    use std::{sync::OnceLock, time::Instant};
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// Obtain the browser's monotonic clock, failing the run if that API is unavailable.
#[cfg(target_arch = "wasm32")]
fn clock_ms() -> f64 {
    // Both functions are standard browser APIs; the generated module runs in a Window.
    browser_now()
}
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "export function browser_now() { return performance.now(); }")]
extern "C" {
    fn browser_now() -> f64;
}

/// Measure only the supplied operation. Construction and correctness oracles stay outside it.
fn timed<T>(samples: &mut BTreeMap<String, Vec<f64>>, id: &str, operation: impl FnOnce() -> T) -> T {
    let start = clock_ms();
    let value = operation();
    let elapsed = clock_ms() - start;
    samples.entry(id.into()).or_default().push(elapsed);
    value
}

/// Retain only observable contact/violations when comparing pruned candidates to all pairs.
fn findings(results: &Results) -> Vec<Finding> {
    results
        .decisions
        .iter()
        .filter(|d| d.touches || d.clearance_violation)
        .map(|d| (d.first, d.second, d.touches, d.clearance_violation, d.conflicting_nets))
        .collect()
}

/// Independently evaluate an initially unknown probe against every shared-layer object.
/// Contact combines intended memberships before decisions; only one equal resolved net
/// exempts strict clearance, and disposable probes have no physical-island exemption.
/// This oracle deliberately bypasses candidate enumeration and the probe decision path.
fn exhaustive_probe_findings(query: &ClearanceQuery, contour: &Contour, layers: &[Uuid]) -> Vec<Finding> {
    let gaps: Vec<_> = query
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, object)| object.layers.iter().any(|layer| layers.contains(layer)))
        .map(|(index, object)| (index, object, contour.gap(&object.contour)))
        .collect();
    let connected: BTreeSet<_> =
        gaps.iter().filter(|(_, _, gap)| *gap <= 0.0).flat_map(|(_, object, _)| object.intended_nets.iter().copied()).collect();
    gaps.into_iter()
        .filter_map(|(index, object, gap)| {
            let touches = gap <= 0.0;
            let same_net = connected.len() == 1 && object.intended_nets.len() == 1 && connected.contains(&object.intended_nets[0]);
            let clearance_violation = !same_net && gap < query.authority().clearance_nm() as f64;
            (touches || clearance_violation).then_some((None, index, touches, clearance_violation, touches && connected.len() > 1))
        })
        .collect()
}

/// Refuse a timing report when a probe omits or misclassifies exhaustive contact/clearance
/// findings. The caller keeps both this reference and comparison outside the timed boundary.
fn verify_probe(query: &ClearanceQuery, contour: &Contour, layers: &[Uuid], actual: &Results) -> Result<()> {
    if findings(actual) != exhaustive_probe_findings(query, contour, layers) {
        return Err("probe findings differ from exhaustive reference".into());
    }
    Ok(())
}

/// Validate the envelope and versioned trace before any diagnostic can emit evidence.
fn load_artifact(artifact_json: &str) -> Result<Artifact> {
    let artifact: Artifact = serde_json::from_str(artifact_json).map_err(|e| e.to_string())?;
    if artifact.schema != "pcbcraft-performance-fixture/1"
        || artifact.project_sha256 != digest(&artifact.project)
        || artifact.trace.is_empty()
        || artifact.trace.len() > 4096
    {
        return Err("unsupported fixture schema, corrupt project or invalid trace length".into());
    }
    if artifact.trace_id == replay::STANDARD_TRACE_ID
        && serde_json::to_value(&artifact.trace).map_err(|e| e.to_string())?
            != serde_json::to_value(replay::Step::standard()).map_err(|e| e.to_string())?
    {
        return Err("standard trace ID does not match its versioned steps".into());
    }
    Ok(artifact)
}

/// Run cold-snapshot and warm-query workloads plus the production editor's CPU input/paint
/// replay. Invalid fixtures, missed input, oracle disagreement or stale authority fail the run.
/// Samples are bounded to prevent accidental unbounded runs; no wall-clock budgets gate CI.
pub fn run(artifact_json: &str, environment: Environment, iterations: usize) -> Result<Report> {
    environment.validate()?;
    if !(1..=10_000).contains(&iterations) {
        return Err("iterations must be between 1 and 10000".into());
    }
    let artifact = load_artifact(artifact_json)?;
    let source = decode_packed(&artifact.project)?.project;
    let view = source.board_view(artifact.board).ok_or("fixture board target does not exist")?;
    let track = view.tracks().first().ok_or("fixture requires a track for publication")?.id;
    let geometry = json!({"symbols":source.all_symbols().count(),"sheets":source.sheets().len(),"tracks":view.tracks().len(),"vias":view.vias().len(),"footprints":view.footprints().count()});
    let mut samples = BTreeMap::new();
    for iteration in 0..iterations {
        let project = timed(&mut samples, "decode.cold", || decode_packed(&artifact.project))?.project;
        let mut session = Session::default();
        session.open_snapshot(project.clone());
        timed(&mut samples, "connectivity.cold", || black_box(project.connectivity()));
        timed(&mut samples, "connectivity.cached", || black_box(project.connectivity()));
        timed(&mut samples, "electrical_connectivity.first", || black_box(project.electrical_connectivity()));
        timed(&mut samples, "electrical_connectivity.repeat", || black_box(project.electrical_connectivity()));
        let query =
            timed(&mut samples, "clearance.prepare", || session.clearance_query(artifact.board, DEFAULT_CLEARANCE_NM)).map_err(|e| e.to_string())?;
        let pairs = timed(&mut samples, "clearance.pairs", || black_box(query.pairs()));
        if iteration == 0 && findings(&pairs) != findings(&query.exhaustive_pairs()) {
            return Err("clearance candidate findings differ from exhaustive reference".into());
        }
        drop(pairs);
        let object = query.objects().first().ok_or("fixture requires copper")?;
        for (id, start, end) in [
            ("empty", Point { x_nm: -5_080_000, y_nm: -5_080_000 }, Point { x_nm: 5_080_000, y_nm: -5_080_000 }),
            ("crowded", Point { x_nm: 100_000_000, y_nm: 25_000_000 }, Point { x_nm: 110_000_000, y_nm: 35_000_000 }),
            ("channel", Point { x_nm: 100_000_000, y_nm: 60_750_000 }, Point { x_nm: 140_000_000, y_nm: 60_750_000 }),
        ] {
            let probe = Contour::segment(start, end, 250_000).map_err(|e| e.to_string())?;
            let candidates =
                timed(&mut samples, &format!("clearance.candidates.{id}"), || query.candidates(&probe, &object.layers)).map_err(|e| e.to_string())?;
            if iteration == 0
                && query.objects().iter().enumerate().any(|(index, item)| {
                    item.layers.iter().any(|layer| object.layers.contains(layer))
                        && probe.gap(&item.contour) <= DEFAULT_CLEARANCE_NM as f64
                        && !candidates.contains(&index)
                })
            {
                return Err(format!("{id}: probe candidates omit an exhaustive geometric neighbor"));
            }
            let actual = timed(&mut samples, &format!("clearance.probe.{id}"), || black_box(query.probe(&probe, &object.layers, None)))
                .map_err(|e| e.to_string())?;
            verify_probe(&query, &probe, &object.layers, &actual).map_err(|e| format!("{id}: {e}"))?;
        }
        timed(&mut samples, "drc.synchronous", || black_box(project.board_view(artifact.board).map(|b| b.check_design(Rules::default()))));
        timed(&mut samples, "command.move_tracks", || {
            fixture::apply(&mut session, MOVE_TRACKS, json!({"board":artifact.board.board,"tracks":[track],"dx_nm":0,"dy_nm":100_000}))
        })?;
        if query.authority().is_current(&session) {
            return Err("published edit did not expire query authority".into());
        }
        drop(query);
        drop(session);
        drop(project);
        replay::run(&artifact, &mut samples)?;
    }
    Ok(Report {
        schema: "pcbcraft-performance-report/1".into(),
        environment,
        target: if cfg!(target_arch = "wasm32") { "browser-cpu" } else { "native-cpu" }.into(),
        debug_assertions: cfg!(debug_assertions),
        artifact_sha256: digest(artifact_json.as_bytes()),
        project_sha256: artifact.project_sha256,
        recipe: artifact.recipe,
        trace_id: artifact.trace_id,
        iterations,
        warmup_frames: replay::WARMUP,
        geometry,
        metrics: samples.into_iter().map(|(id, values)| Metric::new(&id, values)).collect(),
        unmeasured: [
            "input-to-visible / GPU presentation",
            "native event-loop scheduling and Wayland endurance",
            "per-operation allocator/index memory (whole-process collection is separate)",
            "background queues / cancellation / progress (not implemented)",
            "spatial index updates (not implemented)",
            "ERC / walkaround / shove (not implemented)",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn environment() -> Environment {
        Environment {
            revision: "test".into(),
            hardware: "test runner".into(),
            os: "test host".into(),
            runtime: "correctness only".into(),
            compiler: "test compiler".into(),
            profile: "test".into(),
            power: "unknown".into(),
            notes: String::new(),
        }
    }

    /// A successful report requires real route/layer/cancel input, the exhaustive oracle and
    /// stale-query rejection, rather than merely finishing a synthetic timing loop.
    #[test]
    fn reference_replays_real_input_and_preserves_evidence() {
        let artifact = generate("reference-v1", "test").unwrap();
        let report = run(&artifact, environment(), 2).unwrap();
        assert_eq!(report.geometry["tracks"], 60);
        assert_eq!(report.geometry["vias"], 0);
        assert_eq!(report.artifact_sha256, digest(artifact.as_bytes()));
        assert!(report.metrics.iter().all(|m| m.samples_ms.len() == 2));
        for id in ["replay.start.up.ui", "replay.layer.down.ui", "replay.cancel.up.ui", "replay.shared-edit.command", "command.move_tracks"] {
            assert!(report.metrics.iter().any(|m| m.id == id), "missing boundary {id}");
        }
        assert!(!report.unmeasured.is_empty());
    }

    /// Route diagnostics retain raw lifecycle samples, exact command equivalence and bounded
    /// replacement/cancellation evidence, rather than only timing job construction.
    #[test]
    fn routing_diagnostic_checks_publication_and_bounded_replacement() {
        let artifact = generate("reference-v1", "test").unwrap();
        let report = routing::run(&artifact, environment(), 2).unwrap();
        assert_eq!(report["schema"], "pcbcraft-route-preparation-performance/1");
        assert_eq!(report["iterations"], 2);
        assert_eq!(report["queues"].as_array().unwrap().len(), 2);
        for queue in report["queues"].as_array().unwrap() {
            assert_eq!(queue["running_capacity"], 1);
            assert_eq!(queue["pending_capacity"], 1);
            assert_eq!(queue["replaced"], 30);
            assert_eq!(queue["completed"], 1);
        }
        for id in ["route.context_capture", "route.request_capture", "route.atomic_publish", "route.cancel_acknowledgement", "route.queue_replace"] {
            assert!(report["metrics"].as_array().unwrap().iter().any(|m| m["id"] == id));
        }
    }

    /// A shortened idle-only trace must not produce successful evidence under the standard
    /// route/reversal/layer/cancel/edit identity. Custom trace identities keep their old behavior.
    #[test]
    fn standard_trace_identity_rejects_unrelated_steps() {
        let mut artifact: serde_json::Value = serde_json::from_str(&generate("reference-v1", "test").unwrap()).unwrap();
        artifact["trace"] = json!([artifact["trace"][14].clone()]);
        assert!(run(&artifact.to_string(), environment(), 1).unwrap_err().contains("standard trace ID"));
        artifact["trace_id"] = json!("custom-idle-v1");
        assert!(run(&artifact.to_string(), environment(), 1).is_ok());
    }

    /// The oracle must catch omitted findings and wrong contact, clearance and conflicting-net
    /// flags, independently of the broad-phase candidate-completeness assertion.
    #[test]
    fn exhaustive_probe_guard_rejects_corrupted_decisions() {
        let artifact: Artifact = serde_json::from_str(&generate("reference-v1", "test").unwrap()).unwrap();
        let mut session = Session::default();
        session.open_snapshot(decode_packed(&artifact.project).unwrap().project);
        for rule in [0, DEFAULT_CLEARANCE_NM] {
            let query = session.clearance_query(artifact.board, rule).unwrap();
            let object = &query.objects()[0];
            let actual = query.probe(&object.contour, &object.layers, None).unwrap();
            assert!(verify_probe(&query, &object.contour, &object.layers, &actual).is_ok());
            let index = actual.decisions.iter().position(|d| d.touches || d.clearance_violation).unwrap();
            let mut omitted = actual.clone();
            omitted.decisions.remove(index);
            assert!(verify_probe(&query, &object.contour, &object.layers, &omitted).is_err());
            for flag in 0..3 {
                let mut wrong = actual.clone();
                let decision = &mut wrong.decisions[index];
                match flag {
                    0 => decision.touches = !decision.touches,
                    1 => decision.clearance_violation = !decision.clearance_violation,
                    _ => decision.conflicting_nets = !decision.conflicting_nets,
                }
                assert!(verify_probe(&query, &object.contour, &object.layers, &wrong).is_err());
            }
        }
    }

    /// Multi-row placement must finish before wiring; otherwise automatic wire healing can
    /// make a later explicit connection redundant and prevent dense artifacts from generating.
    #[test]
    fn dense_recipe_encodes_supported_counts() {
        let artifact: Artifact = serde_json::from_str(&generate("dense-v1", "test").unwrap()).unwrap();
        let project = decode_packed(&artifact.project).unwrap().project;
        let board = project.board_view(artifact.board).unwrap();
        assert_eq!(project.symbols().len(), 128);
        assert_eq!(board.tracks().len(), 1008);
        assert_eq!(board.vias().len(), 112);
    }

    /// New recipes must round-trip through production codecs and exercise real supported
    /// sheet ownership and crowded branch copper rather than only changing workload labels.
    #[test]
    fn branch_and_hierarchy_recipes_have_their_declared_geometry() {
        let channels: Artifact = serde_json::from_str(&generate("channels-v1", "test").unwrap()).unwrap();
        let project = decode_packed(&channels.project).unwrap().project;
        let board = project.board_view(channels.board).unwrap();
        assert_eq!(board.tracks().len(), 410);
        let hierarchy: Artifact = serde_json::from_str(&generate("hierarchy-v1", "test").unwrap()).unwrap();
        let project = decode_packed(&hierarchy.project).unwrap().project;
        assert_eq!(project.sheets().len(), 64);
        assert_eq!(project.all_symbols().count(), 64);
        assert_eq!(project.sheets().filter(|sheet| project.parent_sheet(sheet.target).is_some()).count(), 63);
        assert!(!project.electrical_connectivity().nets.is_empty());
    }

    /// Corrupt archives, unsupported recipes and invalid sample/environment metadata must
    /// fail before timings can be mistaken for baseline evidence.
    #[test]
    fn rejects_invalid_evidence() {
        assert!(generate("unknown", "test").is_err());
        let artifact = generate("reference-v1", "test").unwrap();
        let mut corrupt: serde_json::Value = serde_json::from_str(&artifact).unwrap();
        corrupt["project"][0] = json!(0);
        assert!(run(&corrupt.to_string(), environment(), 1).is_err());
        assert!(run(&artifact, environment(), 0).is_err());
        let mut unlabeled = environment();
        unlabeled.hardware.clear();
        assert!(run(&artifact, unlabeled, 1).is_err());
    }
}
