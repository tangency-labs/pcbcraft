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
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{hint::black_box, time::Instant};
use uuid::Uuid;

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
    let (bounds, masks): (Vec<Bounds>, Vec<u64>) = if artifact.get("bounds").is_some() {
        let bounds: Vec<Bounds> = serde_json::from_value(artifact["bounds"].clone())?;
        let masks = match artifact.get("layer_masks") {
            Some(v) => serde_json::from_value(v.clone())?,
            None => vec![1; bounds.len()],
        };
        (bounds, masks)
    } else {
        let packed: Vec<u8> = serde_json::from_value(artifact["project"].clone())?;
        let board: BoardTarget = serde_json::from_value(artifact["board"].clone())?;
        let mut session = Session::default();
        session.open_snapshot(decode_packed(&packed)?.project);
        let query = session.clearance_query(board, 200_000)?;
        let bounds = query.objects().iter().map(|o| o.contour.bounds()).collect();
        let masks = native_masks(&query.objects().iter().map(|o| o.layers.as_slice()).collect::<Vec<_>>())?;
        (bounds, masks)
    };
    let owners: Vec<usize> = match artifact.get("owners") {
        Some(v) => serde_json::from_value(v.clone())?,
        None => (0..bounds.len()).collect(),
    };
    if masks.len() != bounds.len() || owners.len() != bounds.len() {
        return Err("invalid simulated masks/owners".into());
    }
    let probes: Vec<(String, Bounds, u64)> =
        if let Some(probes) = artifact.get("probes") { serde_json::from_value(probes.clone())? } else { default_probes(&bounds, &masks) };
    let mut samples = BTreeMap::<&str, Samples>::new();
    let mut execution_order = Vec::new();
    for iteration in 0..iterations {
        let order = strategy_order(iteration);
        for &strategy in &order {
            let sample = samples.entry(strategy).or_default();
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
            sample.build_ms.push(ms);
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
                    sample.candidate_counts.push(ids.len());
                }
                if ids != reference {
                    return Err(format!("{strategy} omitted/reordered a candidate").into());
                }
                sample.query_ms.push(ms);
                sample.query_groups_ms.entry(kind.clone()).or_default().push(ms);
            }
            sample.retained_payload_bytes = tree.retained_bytes(&bounds);
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
                sample.replace_query_ms.push(ms);
            }
        }
        execution_order.push(order);
    }
    let mut layer_mask_counts = BTreeMap::<u64, usize>::new();
    for mask in &masks {
        *layer_mask_counts.entry(*mask).or_default() += 1;
    }
    let report = json!({"schema":4,"fixture_sha256":format!("{:x}",Sha256::digest(bytes)),"objects":bounds.len(),"iterations":iterations,"probes":probes.len(),"probe_layer_masks":probes.iter().map(|p|p.2).collect::<Vec<_>>(),"layer_mask_counts":layer_mask_counts,"execution_order":execution_order,"oracle":"exact inclusive exhaustive AABB candidate equality; shapes/layers unchanged","memory_scope":"retained capacity; grid excludes map bucket/node overhead; Rstar payload unknown; no isolated allocation count","samples":samples});
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
    serde_json::to_writer_pretty(file, &report)?;
    Ok(())
}

/// Raw arrays use the iteration index shared with `execution_order`; each strategy runs once
/// per iteration, with parsing and exhaustive correctness checks outside timed intervals.
#[derive(Default, Serialize)]
struct Samples {
    candidate_counts: Vec<usize>,
    build_ms: Vec<f64>,
    query_ms: Vec<f64>,
    query_groups_ms: BTreeMap<String, Vec<f64>>,
    replace_query_ms: Vec<f64>,
    retained_payload_bytes: Option<usize>,
}

/// Preserve native UUID membership in deterministic bits; refuse an unrepresentable stackup.
fn native_masks(layers: &[&[Uuid]]) -> Result<Vec<u64>, Box<dyn std::error::Error>> {
    let mut unique: Vec<_> = layers.iter().flat_map(|ids| ids.iter().copied()).collect();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() > 64 {
        return Err("native fixture has more than 64 copper layers".into());
    }
    let bits: BTreeMap<_, _> = unique.into_iter().enumerate().map(|(i, id)| (id, 1u64 << i)).collect();
    Ok(layers.iter().map(|ids| ids.iter().filter_map(|id| bits.get(id)).fold(0, |mask, bit| mask | bit)).collect())
}

/// Source and local probes inherit actual layer membership, including spanning objects.
fn default_probes(bounds: &[Bounds], masks: &[u64]) -> Vec<(String, Bounds, u64)> {
    let selected: Vec<_> = bounds.iter().zip(masks).step_by((bounds.len() / 64).max(1)).take(64).collect();
    let mut probes: Vec<_> = selected.iter().map(|(b, mask)| ("source_bounds".into(), **b, **mask)).collect();
    probes.extend(selected.iter().map(|(b, mask)| {
        let (x, y) = ((b[0] + b[2]) / 2., (b[1] + b[3]) / 2.);
        ("local_1mm".into(), [x - 500_000., y - 500_000., x + 500_000., y + 500_000.], **mask)
    }));
    let all_layers = masks.iter().fold(0, |mask, next| mask | next);
    probes.extend([
        ("empty".into(), [1e11, 1e11, 1e11 + 1e6, 1e11 + 1e6], all_layers),
        ("empty".into(), [-1e11, -1e11, -1e11 + 1e6, -1e11 + 1e6], all_layers),
    ]);
    probes
}

/// Rotate starting positions and reverse each full cycle to counterbalance reproducibly.
fn strategy_order(iteration: usize) -> Vec<&'static str> {
    let mut order = vec![
        "linear",
        "bvh",
        "grid",
        "hash_grid",
        "packed_hilbert",
        "rstar_bulk",
        "packed_layer",
        "bvh_layer",
        "scan_layer",
        "hash_layer",
        "rstar_layer",
    ];
    let count = order.len();
    order.rotate_left(iteration % count);
    if iteration / count % 2 == 1 {
        order.reverse();
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coincident native obstacles on distinct layers stay separate; spanning copper remains
    /// visible on both. The 64th layer is representable and larger memberships are rejected.
    #[test]
    fn native_membership_drives_default_probes_and_candidate_oracles() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let masks = native_masks(&[&[first], &[second], &[first, second]]).unwrap();
        assert_eq!(masks, [1, 2, 3]);
        let bounds = vec![[0., 0., 1., 1.]; 3];
        let probes = default_probes(&bounds, &masks);
        let tree = Candidate::layered(&bounds, &masks, "packed_hilbert").unwrap();
        for (i, expected) in [vec![0, 2], vec![1, 2], vec![0, 1, 2]].iter().enumerate() {
            assert_eq!(probes[i].2, masks[i]);
            assert_eq!(probes[i + 3].2, masks[i]);
            assert_eq!(&tree.query_layers(&bounds, &masks, probes[i].1, 200_000., probes[i].2), expected);
        }
        let layers: Vec<_> = (1..=65).map(Uuid::from_u128).collect();
        assert_eq!(native_masks(&[&layers[..64]]).unwrap(), [u64::MAX]);
        assert!(native_masks(&[&layers]).is_err());
    }

    /// Each sample executes each strategy once; every strategy occupies each position equally
    /// over a full cycle, and the following cycle reverses direction. Report order is explicit.
    #[test]
    fn strategy_schedule_covers_positions_without_dropping_candidates() {
        let first = strategy_order(0);
        for position in 0..first.len() {
            let mut seen: Vec<_> = (0..first.len()).map(|i| strategy_order(i)[position]).collect();
            seen.sort_unstable();
            let mut expected = first.clone();
            expected.sort_unstable();
            assert_eq!(seen, expected);
        }
        let mut reversed = first.clone();
        reversed.reverse();
        assert_eq!(strategy_order(first.len()), reversed);
    }
}
