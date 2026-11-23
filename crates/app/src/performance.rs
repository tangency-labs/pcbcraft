//! Opt-in CPU and rendered diagnostics for #240 and the #237 routing/checker work.
//! Rendered GPU/canvas readback is distinct from compositor presentation or physical
//! input-to-visible latency. Ordinary application builds exclude the harness.

mod bga;
#[cfg(target_arch = "wasm32")]
mod browser_capture;
pub mod checker;
mod envelope;
mod fixture;
#[cfg(not(target_arch = "wasm32"))]
pub mod pointer;
#[cfg(not(target_arch = "wasm32"))]
pub mod project;
#[cfg(not(target_arch = "wasm32"))]
mod remote;
pub mod rendered;
mod replay;
mod report;
pub mod routing;
pub mod shove;
pub mod walkaround;
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
/// Above this object count, `ClearanceQuery::exhaustive_pairs` retains O(n^2) decisions
/// (over a billion on `envelope-v1`) and risks OOM. Smaller fixtures keep the full all-pairs
/// oracle; larger boards verify a deterministic strided sample against all other objects.
const EXHAUSTIVE_PAIR_ORACLE_MAX_OBJECTS: usize = 8_192;
const BOUNDED_PAIR_ORACLE_SAMPLES: usize = 128;

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

/// Verify `ClearanceQuery::pairs` against the full exhaustive oracle on smaller fixtures,
/// or against bounded per-object exhaustive findings on large envelope boards.
fn verify_pairs(query: &ClearanceQuery, pairs: &Results) -> Result<()> {
    if query.objects().len() <= EXHAUSTIVE_PAIR_ORACLE_MAX_OBJECTS {
        if findings(pairs) != findings(&query.exhaustive_pairs()) {
            return Err("clearance candidate findings differ from exhaustive reference".into());
        }
        return Ok(());
    }
    verify_pairs_bounded(query, pairs)
}

/// Verify every finding touching a deterministic strided sample of objects against all other
/// objects without materializing O(n^2) non-violating decisions.
fn verify_pairs_bounded(query: &ClearanceQuery, pairs: &Results) -> Result<()> {
    let total = query.objects().len();
    let step = (total / BOUNDED_PAIR_ORACLE_SAMPLES).max(1);
    let sampled: Vec<usize> = (0..total).step_by(step).take(BOUNDED_PAIR_ORACLE_SAMPLES).collect();
    let selected: BTreeSet<usize> = sampled.iter().copied().collect();
    let actual: Vec<Finding> = findings(pairs)
        .into_iter()
        .filter(|(first, second, _, _, _)| first.is_some_and(|i| selected.contains(&i)) || selected.contains(second))
        .collect();
    let expected = findings(&query.exhaustive_findings_for(&sampled));
    if actual != expected {
        return Err("clearance candidate findings differ from bounded exhaustive reference".into());
    }
    Ok(())
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
    let baseline = query.authority().rules().values.clearance_nm;
    let object_clearance = |object: &pcbcraft_engine::board::clearance::Object| {
        query
            .authority()
            .class_rules()
            .and_then(|context| context.rules_for(&object.name))
            .and_then(|policy| policy.clearance_nm().ok())
            .unwrap_or(baseline)
    };
    let probe_clearance = query
        .objects()
        .iter()
        .filter(|object| object.intended_nets.iter().any(|net| connected.contains(net)))
        .map(object_clearance)
        .fold(baseline, i64::max);
    gaps.into_iter()
        .filter_map(|(index, object, gap)| {
            let touches = gap <= 0.0;
            let same_net = connected.len() == 1 && object.intended_nets.len() == 1 && connected.contains(&object.intended_nets[0]);
            let clearance_violation = !same_net && gap < probe_clearance.max(object_clearance(object)) as f64;
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

/// Verify the newly published route by identity, so matching pre-existing geometry cannot
/// hide an incorrect default. This single-segment workload must add exactly one addressed track.
fn verify_added_route(
    before: &BTreeSet<Uuid>,
    tracks: &[pcbcraft_engine::board::track::Track],
    start: Point,
    end: Point,
    layer: Uuid,
    width: i64,
) -> Result<()> {
    let mut added = tracks.iter().filter(|track| !before.contains(&track.id));
    let Some(track) = added.next() else {
        return Err("classified route added no track".into());
    };
    if added.next().is_some()
        || tracks.len() != before.len() + 1
        || track.start != start
        || track.end != end
        || track.layer != layer
        || track.width_nm != width
    {
        return Err("new classified track differs from expected geometry or contact width".into());
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
    let expected = match artifact.trace_id.as_str() {
        replay::STANDARD_TRACE_ID => Some(("standard", replay::Step::standard())),
        replay::ENVELOPE_TRACE_ID => Some(("envelope", replay::Step::envelope())),
        replay::COMMIT_TRACE_ID => Some(("commit", replay::Step::commit())),
        replay::SHOVE_TRACE_ID => Some(("shove", replay::Step::shove())),
        _ => None,
    };
    if let Some((kind, expected)) = expected
        && serde_json::to_value(&artifact.trace).map_err(|e| e.to_string())? != serde_json::to_value(expected).map_err(|e| e.to_string())?
    {
        return Err(format!("{kind} trace ID does not match its versioned steps"));
    }
    Ok(artifact)
}

/// Derive a short commit/history trace without modifying or regenerating native project bytes.
pub fn route_commit_artifact(artifact_json: &str) -> Result<String> {
    let mut artifact = load_artifact(artifact_json)?;
    artifact.trace_id = replay::COMMIT_TRACE_ID.into();
    artifact.trace = replay::Step::commit();
    serde_json::to_string_pretty(&artifact).map_err(|e| e.to_string())
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
        if iteration == 0 {
            verify_pairs(&query, &pairs)?;
        }
        drop(pairs);
        if let Some(context) = query.authority().class_rules()
            && let Some(object) = query.objects().iter().find(|object| context.rules_for(&object.name).is_some())
        {
            let at = object.at;
            let current_view = project.board_view(artifact.board).ok_or("missing current board")?;
            let policy = timed(&mut samples, "class.route_rules.contact", || current_view.route_rules_at(at, object.layers.first().copied()))
                .map_err(|e| e.to_string())?
                .ok_or("classified contact lost its policy")?;
            let width = policy.track_width(None).map_err(|e| e.to_string())?;
            let before: BTreeSet<_> = current_view.tracks().iter().map(|track| track.id).collect();
            let layer = *object.layers.first().ok_or("classified contact has no copper layer")?;
            let end = Point { x_nm: at.x_nm, y_nm: at.y_nm - 3_000_000 };
            let mut route = Session::default();
            route.open_snapshot(project.clone());
            let params = json!({"board":artifact.board.board,"layer":layer,"points":[at,end],"expected_revision":route.revision()});
            let mut job = timed(&mut samples, "class.route.request_capture", || route.prepare_copper_edit(pcbcraft_engine::ADD_TRACKS, &params))
                .map_err(|e| e.to_string())?;
            let began = clock_ms();
            let mut batches = 0;
            let mut maximum_batches = BTreeMap::<String, f64>::new();
            loop {
                let stage = job.progress().stage;
                let started = clock_ms();
                let done = job.step(256);
                let elapsed = clock_ms() - started;
                maximum_batches.entry(format!("class.route.batch_max.{stage}")).and_modify(|value| *value = value.max(elapsed)).or_insert(elapsed);
                batches += 1;
                if batches > 10_000_000 {
                    return Err("classified route exceeded work bound".into());
                }
                if done {
                    break;
                }
            }
            samples.entry("class.route.engine_total".into()).or_default().push(clock_ms() - began);
            for (stage, maximum) in maximum_batches {
                samples.entry(stage).or_default().push(maximum);
            }
            let edit = job.take_result().map_err(|e| e.to_string())?.ok_or("missing classified edit")?;
            if edit.class_rules().is_none() {
                return Err("classified edit omitted rule provenance".into());
            }
            timed(&mut samples, "class.route.atomic_publish", || route.commit_copper_edit(edit)).map_err(|e| e.to_string())?;
            let routed = route.project().and_then(|project| project.board_view(artifact.board)).ok_or("missing routed board")?;
            verify_added_route(&before, routed.tracks(), at, end, layer, width)?;
        }
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
    // Keep checkout-specific diagnostics from perturbing the fixed-artifact samples.
    if artifact.recipe == "bga-v1" {
        for _ in 0..iterations {
            bga::measure(&mut samples)?;
        }
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

    /// Existing-project diagnostics must exercise real zoom/click handling while leaving the
    /// source archive unchanged; the report identifies the exact immutable input it replayed.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn project_navigation_observes_input_without_authoring_edits() {
        let artifact = load_artifact(&generate("reference-v1", "test").unwrap()).unwrap();
        let source = decode_packed(&artifact.project).unwrap().project;
        let original = pcbcraft_engine::persistence::encode_packed(source.clone()).unwrap();
        let report = project::navigation(source.clone(), environment()).unwrap();
        assert_eq!(report["project_sha256"], digest(&original));
        assert_eq!(pcbcraft_engine::persistence::encode_packed(source).unwrap(), original);
        assert_eq!(report["results"].as_array().unwrap().len(), 2);
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

    /// Extend the retained recipe using only shared commands, preserving its trace and copper.
    fn assigned_reference() -> String {
        let mut artifact = load_artifact(&generate("reference-v1", "test").unwrap()).unwrap();
        let mut author = Session::default();
        author.open_snapshot(decode_packed(&artifact.project).unwrap().project);
        fixture::apply(
            &mut author,
            "net_classes.create",
            json!({"name":"Power","rules":{"clearance_nm":700000,"min_track_width_nm":400000,"preferred_track_width_nm":500000}}),
        )
        .unwrap();
        let class = author.project().unwrap().net_classes().definitions[0].id;
        let targets = author.project().unwrap().inspect_net_registry().unwrap();
        let anchor = targets["nets"][0]["anchor"].clone();
        fixture::apply(&mut author, "nets.set_classes", json!({"target":{"kind":"derived","anchor":anchor},"classes":[class]})).unwrap();
        artifact.project = pcbcraft_engine::persistence::encode_packed(author.snapshot().unwrap()).unwrap();
        artifact.project_sha256 = digest(&artifact.project);
        artifact.recipe = "reference-v1-class-assigned".into();
        serde_json::to_string(&artifact).unwrap()
    }

    /// Assigned fixtures must exercise class contact/default/publication costs as well as the
    /// per-object exhaustive spacing oracle. Unclassified artifact boundaries remain unchanged.
    #[test]
    fn assigned_reference_checks_class_defaults_and_pair_specific_oracles() {
        let report = run(&assigned_reference(), environment(), 1).unwrap();
        for id in ["class.route_rules.contact", "class.route.engine_total", "class.route.atomic_publish"] {
            assert!(report.metrics.iter().any(|metric| metric.id == id));
        }
    }

    /// A matching old track must not hide an incorrectly sized, misplaced or absent new route.
    #[test]
    fn route_oracle_requires_the_new_track_identity_geometry_and_width() {
        use pcbcraft_engine::board::track::Track;
        let (start, end) = (Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 0, y_nm: 3000000 });
        let old = Track { id: Uuid::from_u128(1), locked: false, layer: Uuid::from_u128(3), width_nm: 500000, start, end };
        let before = BTreeSet::from([old.id]);
        let mut new = old.clone();
        new.id = Uuid::from_u128(2);
        new.width_nm = 250000;
        assert!(verify_added_route(&before, &[old.clone(), new.clone()], start, end, old.layer, 500000).is_err());
        new.width_nm = 500000;
        assert!(verify_added_route(&before, &[old.clone(), new.clone()], start, end, old.layer, 500000).is_ok());
        new.end.y_nm += 1;
        assert!(verify_added_route(&before, &[old.clone(), new], start, end, old.layer, 500000).is_err());
        assert!(verify_added_route(&before, std::slice::from_ref(&old), start, end, old.layer, 500000).is_err());
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

    /// Walkaround diagnostics time complete plans over real airwires, reproduce them exactly
    /// and commit each through the shared command against the exhaustive pair reference.
    #[test]
    fn walkaround_diagnostic_reproduces_and_commits_its_plans() {
        let artifact = generate("reference-v1", "test").unwrap();
        let report = walkaround::run(&artifact, environment(), 2).unwrap();
        assert_eq!(report["schema"], "pcbcraft-walkaround-performance/1");
        let outcomes = report["outcomes"].as_array().unwrap();
        assert!(outcomes.iter().any(|o| o["case"] == "airwire.1" && o["outcome"]["kind"] == "reached"), "{outcomes:?}");
        // Airwire 0 starts on a legacy marker without conductive pad geometry. It cannot
        // lend a net to an anonymous route; keep that blocked outcome in the measurements.
        assert!(outcomes.iter().any(|o| o["case"] == "airwire.0" && o["outcome"]["kind"] == "blocked"));
        for id in ["walkaround.prepare", "walkaround.plan.all", "walkaround.plan.channel"] {
            let metric = report["metrics"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap();
            assert_eq!(metric["samples_ms"].as_array().unwrap().len(), 2);
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

    /// The BGA recipe places one array at the pad, terminal and artwork limits next to the
    /// reference board, and a run reports every timed BGA operation.
    #[test]
    fn bga_recipe_places_the_full_array_and_times_each_operation() {
        let artifact_json = generate("bga-v1", "test").unwrap();
        let artifact: Artifact = serde_json::from_str(&artifact_json).unwrap();
        let project = decode_packed(&artifact.project).unwrap().project;
        let board = project.board_view(artifact.board).unwrap();
        assert_eq!(board.footprints().count(), 9);
        assert_eq!(board.tracks().len(), 60);
        assert_eq!(project.all_symbols().map(|symbol| symbol.pins().len()).sum::<usize>(), 16 + 1024);
        let report = serde_json::to_value(run(&artifact_json, environment(), 1).unwrap()).unwrap();
        let ids: BTreeSet<_> = report["metrics"].as_array().unwrap().iter().filter_map(|metric| metric["id"].as_str()).collect();
        for id in [
            "bga.library_load",
            "bga.place_part",
            "bga.place_units",
            "bga.place_footprint",
            "bga.baseline",
            "bga.encode_packed",
            "bga.decode_packed",
            "bga.check_design",
        ] {
            assert!(ids.contains(id), "{id} missing from {ids:?}");
        }
    }

    /// The envelope recipe builds a board at the documented track, via and 1,024-pad footprint
    /// limits through shared commands, stays within every native storage bound, and reopens with
    /// all counts and the front/back layer split intact. Seeded geometry and counts are
    /// deterministic while document UUIDs are random, so the SHA-256 checks verify round-trip
    /// byte stability across save, reopen and re-encode.
    #[test]
    fn envelope_recipe_generates_saves_reopens_and_keeps_stable_bytes() {
        use pcbcraft_engine::{
            board::{
                setup::{LayerPurpose, Side},
                track::{MAX_TRACKS, MAX_VIAS},
            },
            persistence::{MAX_ARCHIVE_BYTES, MAX_DOCUMENT_BYTES, MAX_DOCUMENTS_BYTES, decode_documents, encode_documents, encode_packed},
        };

        let artifact_json = generate("envelope-v1", "test").unwrap();
        let artifact = load_artifact(&artifact_json).unwrap();
        assert_eq!(artifact.recipe, "envelope-v1");
        assert_eq!(artifact.trace_id, replay::ENVELOPE_TRACE_ID);
        assert_eq!(artifact.seed, 237_240);
        assert_eq!(artifact.project_sha256, digest(&artifact.project));
        assert!(artifact.project.len() <= MAX_ARCHIVE_BYTES, "{} bytes", artifact.project.len());

        let reopened = decode_packed(&artifact.project).unwrap().project;
        let board = reopened.board_view(artifact.board).unwrap();
        assert_eq!(reopened.sheets().len(), 1);
        assert_eq!(reopened.components().count(), envelope::FOOTPRINTS);
        assert_eq!(reopened.all_symbols().count(), envelope::DISCRETE_PARTS as usize + envelope::BGA_UNITS);
        assert_eq!(board.footprints().count(), envelope::FOOTPRINTS);
        assert_eq!(board.tracks().len(), MAX_TRACKS);
        assert_eq!(board.vias().len(), MAX_VIAS);
        assert_eq!(board.footprints().map(|footprint| footprint.pads.len()).max(), Some(envelope::BGA_PADS));
        assert_eq!(
            board.footprints().map(|footprint| footprint.pads.len()).sum::<usize>(),
            envelope::DISCRETE_PARTS as usize * 2 + envelope::BGA_PADS
        );

        let setup = board.setup().unwrap();
        let front = setup.layers().iter().find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(Side::Front)).unwrap().id;
        let back = setup.layers().iter().find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(Side::Back)).unwrap().id;
        assert_eq!(board.tracks().iter().filter(|track| track.layer == front).count(), MAX_TRACKS / 2);
        assert_eq!(board.tracks().iter().filter(|track| track.layer == back).count(), MAX_TRACKS / 2);

        let reencoded_packed = encode_packed(reopened.clone()).unwrap();
        assert_eq!(digest(&reencoded_packed), artifact.project_sha256);
        assert_eq!(reencoded_packed, artifact.project);

        let documents = encode_documents(reopened).unwrap();
        let total_bytes: usize = documents.values().map(Vec::len).sum();
        assert!(total_bytes <= MAX_DOCUMENTS_BYTES, "{total_bytes} bytes");
        for (path, bytes) in &documents {
            assert!(bytes.len() <= MAX_DOCUMENT_BYTES, "{path}: {} bytes", bytes.len());
        }
        let expected_digests: BTreeMap<_, _> = documents.iter().map(|(path, bytes)| (path.clone(), digest(bytes))).collect();
        let reopened_documents = decode_documents(&documents).unwrap();
        let reencoded_documents = encode_documents(reopened_documents).unwrap();
        let actual_digests: BTreeMap<_, _> = reencoded_documents.iter().map(|(path, bytes)| (path.clone(), digest(bytes))).collect();
        assert_eq!(actual_digests, expected_digests);
    }

    /// Tampering with the envelope trace under its versioned ID must be rejected by `load_artifact`.
    #[test]
    fn envelope_trace_identity_rejects_unrelated_steps() {
        let mut artifact: serde_json::Value = serde_json::from_str(&generate("reference-v1", "test").unwrap()).unwrap();
        artifact["trace_id"] = json!(replay::ENVELOPE_TRACE_ID);
        let err = load_artifact(&artifact.to_string()).err().unwrap();
        assert!(err.contains("envelope trace ID"), "{err}");
    }

    /// The bounded pair oracle must match valid candidate pairs and reject omitted or altered findings.
    #[test]
    fn bounded_pair_oracle_accepts_valid_pairs_and_rejects_corrupted_findings() {
        let artifact = load_artifact(&generate("reference-v1", "test").unwrap()).unwrap();
        let mut session = Session::default();
        session.open_snapshot(decode_packed(&artifact.project).unwrap().project);
        let query = session.clearance_query(artifact.board, DEFAULT_CLEARANCE_NM).unwrap();
        let pairs = query.pairs();
        assert!(verify_pairs_bounded(&query, &pairs).is_ok());
        let index = pairs.decisions.iter().position(|d| d.touches || d.clearance_violation).unwrap();
        let mut omitted = pairs.clone();
        omitted.decisions.remove(index);
        assert!(verify_pairs_bounded(&query, &omitted).is_err());
        let mut flipped = pairs;
        flipped.decisions[index].clearance_violation = !flipped.decisions[index].clearance_violation;
        assert!(verify_pairs_bounded(&query, &flipped).is_err());
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

    /// Deferred live `pcb.merge_tracks` on `envelope-v1` (65,536 tracks, 32,768 selected)
    /// runs under `performance::` in CI, cancels and rejects stale revisions mid-flight,
    /// and completes preview and full apply in one undo step.
    #[test]
    fn deferred_merge_tracks_on_envelope_v1_cancels_rejects_stale_and_applies() {
        crate::live_control::verify_envelope_v1_deferred_merge();
    }

    /// Native desktop worker timing for deferred `pcb.merge_tracks` on `envelope-v1`
    /// (65,536 tracks, 32,768 selected), reporting editor-thread admission and completion
    /// separately for preview and apply.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn merge_timing_native_envelope_v1() {
        crate::live_control::measure_envelope_v1_native_merge();
    }
}
