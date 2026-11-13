//! Same-process production-versus-legacy candidate comparison, independent of editor replay.
use pcbcraft_engine::{
    Session,
    board::{geometry::Contour, view::BoardTarget},
    persistence::decode_packed,
    schematic::Point,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, hint::black_box, time::Instant};

/// Measure only the supplied lookup, leaving source preparation and candidate equality untimed.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = black_box(f());
    (value, start.elapsed().as_secs_f64() * 1000.)
}

/// Keep fixture bytes fixed and alternate lookup order to avoid assigning all warm-cache benefit
/// to one method. The old scan omits layer-input validation, making its baseline conservative.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [fixture, output] = args.as_slice() else { return Err("usage: spatial_probe FIXTURE NEW_REPORT".into()) };
    let bytes = std::fs::read(fixture)?;
    let artifact: Value = serde_json::from_slice(&bytes)?;
    let packed: Vec<u8> = serde_json::from_value(artifact["project"].clone())?;
    let board: BoardTarget = serde_json::from_value(artifact["board"].clone())?;
    let mut session = Session::default();
    session.open_snapshot(decode_packed(&packed)?.project);
    let (query, cold_ms) = timed(|| session.clearance_query(board, 200_000));
    let query = query?;
    let layers = &query.objects().first().ok_or("fixture needs copper")?.layers;
    let mut samples = BTreeMap::<String, Vec<f64>>::new();
    let mut counts = BTreeMap::new();
    for (label, (x, y), (xx, yy)) in [
        ("empty", (-5_080_000, -5_080_000), (5_080_000, -5_080_000)),
        ("crowded", (100_000_000, 25_000_000), (110_000_000, 35_000_000)),
        ("channel", (100_000_000, 60_750_000), (140_000_000, 60_750_000)),
    ] {
        let contour = Contour::segment(Point { x_nm: x, y_nm: y }, Point { x_nm: xx, y_nm: yy }, 250_000)?;
        let b = contour.bounds();
        for iteration in 0..1008 {
            let mut results = [Vec::new(), Vec::new()];
            for strategy in if iteration % 2 == 0 { [0, 1] } else { [1, 0] } {
                let (result, ms) = timed(|| {
                    if strategy == 0 {
                        Ok(query
                            .objects()
                            .iter()
                            .enumerate()
                            .filter_map(|(i, item)| {
                                let a = item.contour.bounds();
                                (item.layers.iter().any(|l| layers.contains(l))
                                    && b[0] <= a[2] + 200_000.
                                    && a[0] <= b[2] + 200_000.
                                    && b[1] <= a[3] + 200_000.
                                    && a[1] <= b[3] + 200_000.)
                                    .then_some(i)
                            })
                            .collect::<Vec<_>>())
                    } else {
                        query.candidates(&contour, layers)
                    }
                });
                // Both branches return Result<Vec<usize>, CommandError> through the typed
                // wrapper below so failures cannot become suspiciously fast timing samples.
                let result: Vec<usize> = result?;
                if iteration >= 8 {
                    samples.entry(format!("{label}.{}", if strategy == 0 { "legacy_scan" } else { "production" })).or_default().push(ms);
                }
                results[strategy] = result;
            }
            if results[0] != results[1] {
                return Err("production candidates differ from exhaustive legacy scan".into());
            }
            counts.insert(label, results[0].len());
        }
    }
    for _ in 0..1000 {
        let (again, ms) = timed(|| session.clearance_query(board, 200_000));
        let again = again?;
        if !std::ptr::eq(query.objects(), again.objects()) {
            return Err("unchanged geometry rebuilt".into());
        }
        samples.entry("prepare.reused".into()).or_default().push(ms);
    }
    let report = json!({"schema":"pcbcraft-spatial-probe/1","fixture_sha256":format!("{:x}",Sha256::digest(bytes)),"objects":query.objects().len(),
        "cold_prepare_ms":cold_ms,"candidate_counts":counts,"samples_ms":samples,"scope":"same-process 1000 warm samples, alternating lookup order; legacy scan excludes validation; no presentation claim"});
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
    serde_json::to_writer_pretty(file, &report)?;
    Ok(())
}
