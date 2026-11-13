//! Copper preparation service costs, separate from physical input-to-visible latency.
use super::*;
use crate::board_drc::jobs::{Queue, Request};
use pcbcraft_engine::board::track::preparation::CopperEditJob;

/// Measure captured preparation, bounded batches, atomic publication, pointer replacement and
/// cancellation acknowledgement on exact saved artifacts. Compare committed bytes with the
/// ordinary synchronous shared command outside timings; neither path serializes during capture.
pub fn run(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    environment.validate()?;
    if !(1..=1000).contains(&iterations) {
        return Err("route iterations must be between 1 and 1000".into());
    }
    let artifact = load_artifact(artifact_json)?;
    let project = decode_packed(&artifact.project)?.project;
    let track = project.board_view(artifact.board).ok_or("missing board")?.tracks().first().ok_or("missing track")?.id;
    let mut samples = BTreeMap::new();
    let mut queues = Vec::new();
    for _ in 0..iterations {
        let mut session = Session::default();
        session.open_snapshot(project.clone());
        let params = json!({"board":artifact.board.board,"tracks":[track],"dx_nm":0,"dy_nm":100_000,"expected_revision":session.revision()});
        let revision = session.revision();
        let context = timed(&mut samples, "route.context_capture", || session.copper_preparation(artifact.board)).map_err(|e| e.to_string())?;
        let mut job = timed(&mut samples, "route.request_capture", || session.prepare_copper_edit_with(&context, MOVE_TRACKS, &params))
            .map_err(|e| e.to_string())?;
        let began = clock_ms();
        let mut batches = 0;
        loop {
            let stage = job.progress().stage;
            let done = timed(&mut samples, &format!("route.batch.{stage}"), || job.step(256));
            batches += 1;
            if batches > 10_000_000 {
                return Err("route preparation exceeded work bound".into());
            }
            if done {
                break;
            }
        }
        samples.entry("route.engine_total".into()).or_default().push(clock_ms() - began);
        let edit = job.take_result().map_err(|e| e.to_string())?.ok_or("missing prepared edit")?;
        if session.revision() != revision || !edit.source().is_current(&session) {
            return Err("preparation mutated source".into());
        }
        timed(&mut samples, "route.atomic_publish", || session.commit_copper_edit(edit)).map_err(|e| e.to_string())?;
        if session.revision() != revision + 1 {
            return Err("route did not publish once".into());
        }
        let mut reference = Session::default();
        reference.open_snapshot(project.clone());
        reference.apply(MOVE_TRACKS, &params).map_err(|e| e.to_string())?;
        if pcbcraft_engine::persistence::encode_documents(reference.snapshot().ok_or("missing reference")?).map_err(|e| e.to_string())?
            != pcbcraft_engine::persistence::encode_documents(session.snapshot().ok_or("missing result")?).map_err(|e| e.to_string())?
        {
            return Err("prepared route differs from synchronous shared command".into());
        }
        // Reuse the same immutable pad context for another pointer request, then coalesce it
        // repeatedly. Capacity includes one active request plus the latest pending request.
        let mut queue = Queue::<CopperEditJob>::default();
        let reused = reference.copper_preparation(artifact.board).map_err(|e| e.to_string())?;
        for id in 0..32 {
            let job = timed(&mut samples, "route.reused_request_capture", || {
                reference.prepare_copper_edit_with(
                    &reused,
                    MOVE_TRACKS,
                    &json!({
                        "board":artifact.board.board,"tracks":[track],"dx_nm":0,"dy_nm":id+1,"expected_revision":reference.revision()
                    }),
                )
            })
            .map_err(|e| e.to_string())?;
            timed(&mut samples, "route.queue_replace", || queue.submit(Request { id, job }));
        }
        let began = clock_ms();
        queue.cancel();
        while queue.busy() {
            let _ = timed(&mut samples, "route.queue_poll", || queue.poll());
            if clock_ms() - began > 30_000. {
                return Err("route cancellation acknowledgement timed out".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        samples.entry("route.cancel_acknowledgement".into()).or_default().push(clock_ms() - began);
        queues.push(json!({"running_capacity":1,"pending_capacity":1,"native_global_workers":2,"replaced":queue.replaced,"canceled":queue.canceled,"completed":queue.completed}));
        // The source context can still produce read-only work, but cannot authorize a second
        // transaction after publication. Reusing it through the session is refused immediately.
        if session.prepare_copper_edit_with(&context, MOVE_TRACKS, &params).is_ok() {
            return Err("obsolete context accepted".into());
        }
    }
    Ok(json!({"schema":"pcbcraft-route-preparation-performance/1","environment":environment,
        "target":if cfg!(target_arch="wasm32"){"browser-route-cpu"}else{"native-route-cpu"},
        "artifact_sha256":digest(artifact_json.as_bytes()),"iterations":iterations,"batch_units":256,
        "metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),"queues":queues,
        "correctness":"exact committed bytes equal synchronous shared command; one revision; source invalidated; coalesced work canceled",
        "unmeasured":["compositor presentation","physical input-to-visible latency","overnight endurance"]}))
}
