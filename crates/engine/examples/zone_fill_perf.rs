//! Bounded fill-only diagnostic using a saved native archive, separate from editor timing.
use pcbcraft_engine::{
    Session,
    persistence::{decode_packed, encode_documents},
};
use serde_json::json;
use std::{fs, time::Instant};

/// Time capture, calculation and first/repeat connectivity reads without changing saved intent.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let path = args.get(1).ok_or("usage: zone_fill_perf ARCHIVE REPORT")?;
    let output = args.get(2).ok_or("missing report path")?;
    let worker = match args.get(3).map(String::as_str) {
        None => false,
        Some("worker") => true,
        _ => return Err("expected worker or no third argument".into()),
    };
    let bytes = fs::read(path)?;
    let mut session = Session::default();
    session.open_snapshot(decode_packed(&bytes)?.project);
    let native = encode_documents(session.snapshot().ok_or("missing project")?)?;
    let revision = session.revision();
    let target = session.project().and_then(|p| p.board_target()).ok_or("missing board")?;
    let mut samples = Vec::new();
    for _ in 0..if worker { 1 } else { 3 } {
        let start = Instant::now();
        let mut job = session.prepare_zone_fill(target, true)?;
        let capture_ms = start.elapsed().as_secs_f64() * 1e3;
        let start = Instant::now();
        let (fill, transport) = if worker {
            let input = job.worker_request()?;
            let response = pcbcraft_engine::board::fill::worker::execute(&input);
            let fill = job.accept_worker_response(&response)?.ok_or("canceled")?;
            (fill, Some(json!({"request_bytes":input.len(),"response_bytes":response.len()})))
        } else {
            job.step(1);
            (job.take_result()?.ok_or("canceled")?, None)
        };
        let compute_ms = start.elapsed().as_secs_f64() * 1e3;
        let view = session.project().and_then(|p| p.board_view(target)).ok_or("missing view")?;
        let start = Instant::now();
        let lines = view.ratsnest()?.lines.len();
        let first_ratsnest_ms = start.elapsed().as_secs_f64() * 1e3;
        let start = Instant::now();
        assert_eq!(view.ratsnest()?.lines.len(), lines);
        let repeat_ratsnest_ms = start.elapsed().as_secs_f64() * 1e3;
        assert!(fill.problems.is_empty(), "{:?}", fill.problems);
        samples.push(json!({"transport":transport,"contacts":fill.zones.iter().flat_map(|z|&z.islands).map(|i|i.contacts.len()).sum::<usize>(),"capture_ms":capture_ms,"compute_ms":compute_ms,"first_ratsnest_ms":first_ratsnest_ms,"repeat_ratsnest_ms":repeat_ratsnest_ms,"zones":fill.zones.len(),"islands":fill.zones.iter().map(|z|z.islands.len()).sum::<usize>(),"triangles":fill.zones.iter().flat_map(|z|&z.islands).map(|i|i.mesh.triangles.len()).sum::<usize>(),"airwires":lines}));
    }
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().ok_or("missing project")?)?, native);
    let view = session.project().and_then(|p| p.board_view(target)).ok_or("missing view")?;
    let start = Instant::now();
    let drc = view.check_design(view.effective_rules().values);
    let drc_ms = start.elapsed().as_secs_f64() * 1e3;
    let report = json!({"debug_assertions":cfg!(debug_assertions),"fixture":path,"samples":samples,
        "footprints":view.footprints().count(),"pads":view.footprints().map(|f|f.pads.len()).sum::<usize>(),
        "tracks":view.tracks().len(),"vias":view.vias().len(),"drc_ms":drc_ms,"drc_counts":drc.counts,
        "unchanged_native_and_revision":true});
    fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}
