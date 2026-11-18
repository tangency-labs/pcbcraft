//! Opt-in release diagnostic for track-only island construction and the two DRC sweeps.

use super::*;
use crate::board::track::{MIN_WIDTH_NM, Track};
use std::{io::Write, sync::Arc, time::Instant};
use uuid::Uuid;

const MM: i64 = 1_000_000;
const PITCH: i64 = 20_000;
const BUDGET: usize = 1 << 20;
const RECIPE: &str = "track-checks-v1";

/// Emit progress directly, including when a timed-out run never returns to libtest.
fn log(line: std::fmt::Arguments<'_>) {
    writeln!(std::io::stderr().lock(), "{line}").unwrap();
}

/// Fixed geometry and track IDs, no PRNG. Every diagonal bound contains the origin, but
/// the copper is isolated: adjacent centerlines are 20 um / sqrt(2) apart and 10 um wide.
fn tracks(n: usize, overlap: bool, layer: Uuid) -> Vec<Track> {
    let at = |x_nm, y_nm| Point { x_nm, y_nm };
    (0..n)
        .map(|k| {
            let (start, end) = if overlap {
                let c = (k as i64 - n as i64 / 2) * PITCH;
                if c >= 0 { (at(-MM, c + MM), at(c + MM, -MM)) } else { (at(c - MM, MM), at(MM, c - MM)) }
            } else {
                let x = (k % 256) as i64 * MM;
                let y = (k / 256) as i64 * MM;
                (at(x, y), at(x + 200_000, y))
            };
            assert!([start.x_nm, start.y_nm, end.x_nm, end.y_nm].into_iter().all(|v| v.abs() <= crate::schematic::MAX_CENTER_NM));
            Track { id: Uuid::from_u128((1 << 64) | k as u128), locked: false, layer, width_nm: MIN_WIDTH_NM, start, end }
        })
        .collect()
}

/// Supply known singleton islands so a slow contact pass cannot mask clearance cost.
/// Snapshot setup and geometry projection are outside the sweep timings. This diagnostic
/// measures internal phases, not native storage admission or end-to-end checking.
fn clearance_fixture(n: usize, overlap: bool) -> (Session, BoardGeometry) {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":RECIPE,"create_board":true})).unwrap();
    session.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":session.revision()})).unwrap();
    let layer = crate::board::track::tests::copper_layers(&session)[0];
    let mut project = session.project().unwrap().clone();
    Arc::make_mut(&mut project.board).tracks = tracks(n, overlap, layer);
    let view = project.board_view(project.board_target().unwrap()).unwrap();
    let copper = Copper { tracks: view.tracks().iter().enumerate().map(|(i, track)| (track.id, i)).collect(), ..Copper::default() };
    let members = (0..n).map(|i| vec![i]).collect();
    let geometry = BoardGeometry::from_islands(view, Vec::new(), copper, members);
    session.open_snapshot(Arc::new(project));
    (session, geometry)
}

/// Report a conservative count of work units consumed by the real builder, including a lower
/// bound if killed during a later chunk. A unit is not necessarily a pair test.
fn measure_islands(n: usize, overlap: bool) {
    let tracks = tracks(n, overlap, Uuid::from_u128(9));
    let started = Instant::now();
    let mut builder = IslandBuilder::new(&[], &tracks, &[]);
    let mut consumed = 0_usize;
    while !builder.step(BUDGET) {
        consumed += BUDGET;
        if consumed.is_multiple_of(16 * BUDGET) {
            log(format_args!("progress work_units_at_least={consumed} elapsed_s={:.9}", started.elapsed().as_secs_f64()));
        }
    }
    let (copper, members) = builder.finish(&[], &tracks, &[]);
    let elapsed = started.elapsed().as_secs_f64();
    assert_eq!(copper.tracks.len(), n);
    assert_eq!(members.len(), n);
    assert!(members.iter().enumerate().all(|(i, nodes)| nodes.as_slice() == [i]), "every track stays isolated");
    log(format_args!("finish elapsed_s={elapsed:.9} isolated={} work_units_in=[{consumed},{}]", members.len(), consumed + BUDGET));
}

/// Time the synchronous checker’s actual clearance iterator, excluding finding formatting.
fn measure_sweep(geometry: &BoardGeometry, expected: usize) {
    let mut evaluated = 0_usize;
    let mut violations = 0;
    let started = Instant::now();
    for decision in geometry.clearance_pairs(DEFAULT_CLEARANCE_NM, None) {
        evaluated += 1;
        violations += usize::from(decision.clearance_violation);
        if evaluated.is_multiple_of(16 * BUDGET) {
            log(format_args!("progress decisions={evaluated} violations={violations} elapsed_s={:.9}", started.elapsed().as_secs_f64()));
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    assert_eq!(violations, expected);
    log(format_args!("finish elapsed_s={elapsed:.9} decisions={evaluated} violations={violations}"));
}

/// Time the resumable checker’s own sorting, sweep and capped finding collection, excluding
/// unrelated stages and final report construction.
fn measure_job(session: &Session, geometry: BoardGeometry, expected: usize) {
    let mut job = session.design_check_job(&json!({})).unwrap();
    job.geometry = Some(geometry);
    job.phase = 3;
    let started = Instant::now();
    assert!(!job.step(BUDGET));
    assert_eq!(job.phase, 4);
    job.phase = 5;
    let mut consumed = 0_usize;
    while job.phase == 5 {
        assert!(!job.step(BUDGET));
        if job.phase == 5 {
            consumed += BUDGET;
            if consumed.is_multiple_of(16 * BUDGET) {
                let inspected = job.work;
                let violations = job.found.total;
                log(format_args!("progress inspected_pairs={inspected} violations={violations} elapsed_s={:.9}", started.elapsed().as_secs_f64()));
            }
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let violations = job.found.counts.get(&Kind::Clearance).copied().unwrap_or_default();
    assert_eq!(violations, expected);
    log(format_args!("finish elapsed_s={elapsed:.9} violations={violations} work_units_in=[{consumed},{}]", consumed + BUDGET));
}

/// Run one selected case per process, with an external deadline. See the performance guide.
#[test]
#[ignore = "release timing diagnostic; run one explicitly configured case per process"]
fn release_board_checks() {
    let n = std::env::var("PCB_CHECK_TRACKS").unwrap().parse::<usize>().unwrap();
    assert!([16_384, 32_768, 65_536].contains(&n));
    let layout = std::env::var("PCB_CHECK_LAYOUT").unwrap();
    let overlap = match layout.as_str() {
        "sparse" => false,
        "overlap" => true,
        _ => panic!("expected sparse or overlap"),
    };
    let phase = std::env::var("PCB_CHECK_PHASE").unwrap();
    let xy_overlap_pairs = if overlap { n * (n - 1) / 2 } else { 0 };
    // Default clearance + width = 210 um. At pitch / sqrt(2) this spans 14 neighbor offsets.
    let expected = if overlap { (1..=14).map(|offset| n - offset).sum() } else { 0 };
    log(format_args!(
        "start recipe={RECIPE} layout={layout} phase={phase} tracks={n} xy_overlap_pairs={xy_overlap_pairs} expected_clearance={expected}"
    ));
    match phase.as_str() {
        "islands" => measure_islands(n, overlap),
        "sweep" | "job" => {
            let (session, geometry) = clearance_fixture(n, overlap);
            if phase == "sweep" { measure_sweep(&geometry, expected) } else { measure_job(&session, geometry, expected) }
        }
        _ => panic!("expected islands, sweep or job"),
    }
}
