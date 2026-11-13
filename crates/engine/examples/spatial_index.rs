//! Isolated #244 bounds comparison over existing command-generated fixture bytes.
#[path = "spatial_index/alternatives.rs"]
mod alternatives;
#[path = "spatial_index/bvh.rs"]
mod bvh;
#[path = "spatial_index/grid.rs"]
mod grid;
#[path = "spatial_index/hash_grid.rs"]
mod hash_grid;
use alternatives::Candidate;
use bvh::{Bounds, near};
use pcbcraft_engine::{Session, board::view::BoardTarget, persistence::decode_packed};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{hint::black_box, time::Instant};

/// Measure service time only; callers validate returned candidates outside this interval.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = black_box(f());
    (value, start.elapsed().as_secs_f64() * 1000.)
}

/// Emit raw samples without overwriting a report; fixture parsing and oracles are untimed.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [fixture, iterations, output] = args.as_slice() else { return Err("usage: spatial_index FIXTURE ITERATIONS NEW_REPORT".into()) };
    let iterations: usize = iterations.parse()?;
    if !(1..=1000).contains(&iterations) {
        return Err("iterations must be 1..=1000".into());
    }
    let bytes = std::fs::read(fixture)?;
    let artifact: Value = serde_json::from_slice(&bytes)?;
    let bounds: Vec<Bounds> = if artifact.get("bounds").is_some() {
        serde_json::from_value(artifact["bounds"].clone())?
    } else {
        let packed: Vec<u8> = serde_json::from_value(artifact["project"].clone())?;
        let board: BoardTarget = serde_json::from_value(artifact["board"].clone())?;
        let mut session = Session::default();
        session.open_snapshot(decode_packed(&packed)?.project);
        session.clearance_query(board, 200_000)?.objects().iter().map(|o| o.contour.bounds()).collect()
    };
    let masks: Vec<u64> = match artifact.get("layer_masks") {
        Some(v) => serde_json::from_value(v.clone())?,
        None => vec![1; bounds.len()],
    };
    let owners: Vec<usize> = match artifact.get("owners") {
        Some(v) => serde_json::from_value(v.clone())?,
        None => (0..bounds.len()).collect(),
    };
    if masks.len() != bounds.len() || owners.len() != bounds.len() {
        return Err("invalid simulated masks/owners".into());
    }
    let probes: Vec<(String, Bounds, u64)> = if let Some(probes) = artifact.get("probes") {
        serde_json::from_value(probes.clone())?
    } else {
        let selected: Vec<Bounds> = bounds.iter().step_by((bounds.len() / 64).max(1)).copied().take(64).collect();
        let mut probes: Vec<_> = selected.iter().map(|b| ("source_bounds".into(), *b, 1)).collect();
        probes.extend(selected.iter().map(|b| {
            let (x, y) = ((b[0] + b[2]) / 2., (b[1] + b[3]) / 2.);
            ("local_1mm".into(), [x - 500_000., y - 500_000., x + 500_000., y + 500_000.], 1)
        }));
        probes.extend([("empty".into(), [1e11, 1e11, 1e11 + 1e6, 1e11 + 1e6], 1), ("empty".into(), [-1e11, -1e11, -1e11 + 1e6, -1e11 + 1e6], 1)]);
        probes
    };
    let mut samples = serde_json::Map::new();
    for strategy in
        ["linear", "bvh", "grid", "hash_grid", "packed_hilbert", "rstar_bulk", "packed_layer", "bvh_layer", "scan_layer", "hash_layer", "rstar_layer"]
    {
        let mut build = Vec::new();
        let mut queries = Vec::new();
        let mut update = Vec::new();
        let mut storage = None;
        let mut grouped = std::collections::BTreeMap::<&str, Vec<f64>>::new();
        let mut candidate_counts = Vec::new();
        for iteration in 0..iterations {
            let (tree, ms) = timed(|| {
                if strategy.ends_with("_layer") {
                    let implementation = match strategy {
                        "bvh_layer" => "bvh",
                        "scan_layer" => "linear",
                        "hash_layer" => "hash_grid",
                        "rstar_layer" => "rstar_bulk",
                        _ => "packed_hilbert",
                    };
                    Candidate::layered(black_box(&bounds), &masks, implementation)
                } else {
                    Candidate::new(strategy, black_box(&bounds))
                }
            });
            let tree = tree?;
            build.push(ms);
            let lookup = |probe: Bounds, layers: u64| {
                let mut ids: Vec<_> = tree.query_layers(&bounds, &masks, probe, 200_000., layers).into_iter().map(|i| owners[i]).collect();
                ids.sort_unstable();
                ids.dedup();
                ids
            };
            for (probe_index, (kind, probe, layers)) in probes.iter().enumerate() {
                let (ids, ms) = timed(|| lookup(black_box(*probe), *layers));
                let mut reference: Vec<_> = bounds
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| (masks[i] & layers != 0 && near(*probe, *b, 200_000.)).then_some(owners[i]))
                    .collect();
                reference.sort_unstable();
                reference.dedup();
                if let Some(actual) = artifact.get("exact_near_ids") {
                    let expected: Vec<usize> = serde_json::from_value(actual[probe_index].clone())?;
                    if expected.iter().any(|id| !ids.contains(id)) {
                        return Err(format!("{strategy}: missing analytic-geometry neighbor").into());
                    }
                }
                if iteration == 0 {
                    candidate_counts.push(ids.len());
                }
                if ids != reference {
                    return Err(format!("{strategy} omitted/reordered a candidate").into());
                }
                queries.push(ms);
                grouped.entry(kind.as_str()).or_default().push(ms);
            }
            storage = tree.retained_bytes(&bounds);
            // Repeated local gesture updates: retain an immutable index, replace a source with
            // one moved envelope, then discard it. No native project document is edited.
            for n in 0..32 {
                if bounds.is_empty() {
                    break;
                }
                let source = n % bounds.len();
                let b = bounds[source];
                let offset = (n as f64 - 16.) * 25_400.;
                let moved = [b[0] + offset, b[1], b[2] + offset, b[3]];
                let (ids, ms) = timed(|| {
                    let mut ids = lookup(black_box(moved), masks[source]);
                    ids.retain(|i| *i != owners[source]);
                    ids
                });
                let mut expected: Vec<_> = bounds
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| {
                        (owners[i] != owners[source] && masks[i] & masks[source] != 0 && near(moved, *b, 200_000.)).then_some(owners[i])
                    })
                    .collect();
                expected.sort_unstable();
                expected.dedup();
                if ids != expected {
                    return Err("overlay candidates differ".into());
                }
                update.push(ms);
            }
        }
        samples.insert(
            strategy.into(),
            json!({"candidate_counts":candidate_counts,"build_ms":build,"query_ms":queries,"query_groups_ms":grouped,"replace_query_ms":update,"retained_payload_bytes":storage}),
        );
    }
    let report = json!({"schema":3,"fixture_sha256":format!("{:x}",Sha256::digest(bytes)),"objects":bounds.len(),"iterations":iterations,"probes":probes.len(),"oracle":"exact inclusive exhaustive AABB candidate equality; shapes/layers unchanged","memory_scope":"retained capacity; grid excludes map bucket/node overhead; Rstar payload unknown; no isolated allocation count","samples":samples});
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
    serde_json::to_writer_pretty(file, &report)?;
    Ok(())
}
