//! Checker service-cost diagnostics; these do not observe physical presentation latency.
use super::*;
use crate::board_drc::jobs::{Queue, Request};

/// Measure capture, resumable engine batches and bounded queue/cancellation acknowledgement.
/// Exact report equivalence is checked outside timings, using the same saved artifact bytes.
pub fn run(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    environment.validate()?;
    if !(1..=1000).contains(&iterations) {
        return Err("checker iterations must be between 1 and 1000".into());
    }
    let artifact = load_artifact(artifact_json)?;
    let mut samples = BTreeMap::new();
    let mut queue_stats = Vec::new();
    for _ in 0..iterations {
        let mut session = Session::default();
        session.open_snapshot(decode_packed(&artifact.project)?.project);
        let params = json!({"board":artifact.board.board});
        let expected = session.check_design(&params).map_err(|e| e.to_string())?;
        let mut job = timed(&mut samples, "checker.capture", || session.design_check_job(&params)).map_err(|e| e.to_string())?;
        let begin = clock_ms();
        let mut batches = 0;
        loop {
            let stage = job.progress().stage;
            let done = timed(&mut samples, &format!("checker.batch.{stage}"), || job.step(256));
            batches += 1;
            if batches > 10_000_000 {
                return Err("checker did not finish within supported work bound".into());
            }
            if done {
                break;
            }
        }
        samples.entry("checker.engine_total".into()).or_default().push(clock_ms() - begin);
        let actual = job.take_report().ok_or("checker completed without report")?;
        if json!(actual) != expected {
            return Err("resumable report differs from complete reference".into());
        }
        let mut queue = Queue::default();
        queue.submit(Request { id: 1, job: session.design_check_job(&params).map_err(|e| e.to_string())? });
        let began = clock_ms();
        queue.cancel();
        loop {
            let completion = timed(&mut samples, "checker.queue_poll", || queue.poll());
            if let Some(completion) = completion {
                if completion.report.map_err(|e| e.to_string())?.is_some() {
                    // A native worker may finish before cancellation reaches it. Host authority
                    // still rejects its revoked generation; distinguish that race in evidence.
                    queue_stats.push(json!({"completed_before_cancel_observed":true}));
                }
                break;
            }
            if clock_ms() - began > 30_000. {
                return Err("checker cancellation acknowledgement timed out".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        samples.entry("checker.cancel_acknowledgement".into()).or_default().push(clock_ms() - began);
        queue_stats.push(json!({"running":usize::from(queue.busy()),"pending_capacity":1,"running_capacity":1,"replaced":queue.replaced,"canceled":queue.canceled,"completed":queue.completed}));
    }
    Ok(
        json!({"schema":"pcbcraft-checker-performance/1","environment":environment,"target":if cfg!(target_arch="wasm32"){"browser-checker-cpu"}else{"native-checker-cpu"},"artifact_sha256":digest(artifact_json.as_bytes()),"iterations":iterations,"batch_units":256,"metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),"queues":queue_stats,"correctness":"complete-check report equivalence; no authored mutation","unmeasured":["compositor presentation","physical cancellation-to-visible latency","ERC and route preparation"]}),
    )
}
