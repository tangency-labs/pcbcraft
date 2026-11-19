//! Walkaround planning service costs for #246, separate from editor input and presentation.
use super::*;
use pcbcraft_engine::board::{
    route::{DEFAULT_BUDGET, Outcome, Plan, Posture, Request},
    setup::{LayerPurpose, Side},
    track::ADD_TRACKS,
};

/// Airwires planned per iteration, in the ratsnest's deterministic order.
const AIRWIRES: usize = 6;

/// Measure captured planning, largest individual batches, exact validated publication and
/// coalesced cancellation. Oracles compare with the synchronous solver and exhaustive copper
/// decisions outside timings. These service intervals do not measure visible preview latency.
pub fn jobs(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    use crate::board_drc::jobs::{Queue, Request as Queued};
    use pcbcraft_engine::board::route::PlanningJob;
    environment.validate()?;
    if !(1..=1000).contains(&iterations) {
        return Err("planning iterations must be between 1 and 1000".into());
    }
    let artifact = load_artifact(artifact_json)?;
    let project = decode_packed(&artifact.project)?.project;
    let requests = requests(project.board_view(artifact.board).ok_or("missing board")?)?;
    let mut samples = BTreeMap::new();
    let mut queues = Vec::new();
    for _ in 0..iterations {
        let mut session = Session::default();
        session.open_snapshot(project.clone());
        let context = timed(&mut samples, "planning.context_capture", || session.route_planning(artifact.board)).map_err(|e| e.to_string())?;
        for (id, request) in &requests {
            let mut job =
                timed(&mut samples, &format!("planning.capture.{id}"), || session.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET))
                    .map_err(|e| e.to_string())?;
            let started = clock_ms();
            let mut batches = 0;
            let mut largest: BTreeMap<&str, f64> = BTreeMap::new();
            loop {
                let stage = job.progress().stage;
                let began = clock_ms();
                let done = job.step(256);
                let elapsed = clock_ms() - began;
                largest.entry(stage).and_modify(|maximum| *maximum = maximum.max(elapsed)).or_insert(elapsed);
                batches += 1;
                if done {
                    break;
                }
                if batches > 10_000_000 {
                    return Err("planning exceeded work bound".into());
                }
            }
            samples.entry(format!("planning.total.{id}")).or_default().push(clock_ms() - started);
            for (stage, maximum) in largest {
                samples.entry(format!("planning.batch_max.{id}.{stage}")).or_default().push(maximum);
            }
            let plan = job.take_result().map_err(|e| e.to_string())?.ok_or("missing plan")?;
            let query = session.clearance_query_with_rules(artifact.board, &json!({})).map_err(|e| e.to_string())?;
            if plan != session.plan_route(&query, request, DEFAULT_BUDGET, &|| false).map_err(|e| e.to_string())? {
                return Err("batched planning differs from the synchronous solver".into());
            }
            if !job.source().is_current(&session) {
                return Err("planning changed its source".into());
            }
            verify(&project, artifact.board, request, &plan)?;
            if plan.outcome == Outcome::Reached && plan.points.len() >= 2 {
                let mut commit = Session::default();
                commit.open_snapshot(project.clone());
                let params = json!({"board":artifact.board.board,"layer":request.layer,"width_nm":request.width_nm,"points":plan.points,"expected_revision":commit.revision()});
                let mut edit = commit.prepare_copper_edit(ADD_TRACKS, &params).map_err(|e| e.to_string())?;
                let began = clock_ms();
                for batch in 0..10_000_000 {
                    if edit.step(256) {
                        break;
                    }
                    if batch == 9_999_999 {
                        return Err("route validation exceeded work bound".into());
                    }
                }
                samples.entry(format!("planning.validation.{id}")).or_default().push(clock_ms() - began);
                let edit = edit.take_result().map_err(|e| e.to_string())?.ok_or("missing validated path")?;
                let before = commit.revision();
                timed(&mut samples, &format!("planning.atomic_publish.{id}"), || commit.commit_copper_edit(edit)).map_err(|e| e.to_string())?;
                if commit.revision() != before + 1 {
                    return Err("route did not publish once".into());
                }
            }
        }
        let mut queue = Queue::<PlanningJob>::default();
        let (_, request) = requests.first().ok_or("missing route requests")?;
        for id in 0..32 {
            let job = session.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).map_err(|e| e.to_string())?;
            timed(&mut samples, "planning.queue_replace", || queue.submit(Queued { id, job }));
        }
        let began = clock_ms();
        queue.cancel();
        while queue.busy() {
            let _ = queue.poll();
            if clock_ms() - began > 10_000. {
                return Err("planning cancellation timed out".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::yield_now();
        }
        samples.entry("planning.cancel_acknowledgement".into()).or_default().push(clock_ms() - began);
        queues
            .push(json!({"running_capacity":1,"pending_capacity":1,"replaced":queue.replaced,"canceled":queue.canceled,"completed":queue.completed}));
    }
    Ok(json!({"schema":"pcbcraft-walkaround-jobs-performance/1","environment":environment,
        "artifact_sha256":digest(artifact_json.as_bytes()),"iterations":iterations,"batch_units":256,"search_units_per_batch":8,
        "metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),"queues":queues,
        "correctness":"batched plans equal synchronous results; unchanged source; exhaustive validation of new copper; one atomic revision per reached route; canceled coalesced work retired",
        "unmeasured":["physical input-to-visible preview latency","compositor presentation","endurance"]}))
}

/// Plan fixed requests over one captured query per iteration: the fixture's first airwires
/// pad to pad, one free-space route along the channel recipe's corridor and one across the
/// crowded diagonal region, all on front copper at the board's preferred or class width. Each
/// complete plan is timed. Outside the timed boundary, every iteration must reproduce the
/// first iteration's plans exactly, and on the first iteration each plan with at least two points is committed
/// through the shared track command and the exhaustive pair reference must find no clearance
/// violation or short about the new copper. Invalid fixtures or any disagreement fail the run.
pub fn run(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    environment.validate()?;
    if !(1..=1000).contains(&iterations) {
        return Err("walkaround iterations must be between 1 and 1000".into());
    }
    let artifact = load_artifact(artifact_json)?;
    let project = decode_packed(&artifact.project)?.project;
    let view = project.board_view(artifact.board).ok_or("missing board")?;
    let requests = requests(view)?;
    let mut samples = BTreeMap::new();
    let mut first: Vec<Plan> = Vec::new();
    for iteration in 0..iterations {
        let mut session = Session::default();
        session.open_snapshot(project.clone());
        let query = timed(&mut samples, "walkaround.prepare", || session.clearance_query_with_rules(artifact.board, &json!({})))
            .map_err(|e| e.to_string())?;
        let began = clock_ms();
        let mut plans = Vec::new();
        for (id, request) in &requests {
            let plan = timed(&mut samples, &format!("walkaround.plan.{id}"), || session.plan_route(&query, request, DEFAULT_BUDGET, &|| false))
                .map_err(|e| format!("{id}: {e}"))?;
            plans.push(plan);
        }
        samples.entry("walkaround.plan.all".into()).or_default().push(clock_ms() - began);
        if iteration == 0 {
            for ((id, request), plan) in requests.iter().zip(&plans) {
                verify(&project, artifact.board, request, plan).map_err(|e| format!("{id}: {e}"))?;
            }
            first = plans;
        } else if plans != first {
            return Err("walkaround plans differ between iterations".into());
        }
    }
    let outcomes: Vec<_> = requests
        .iter()
        .zip(&first)
        .map(|((id, request), plan)| {
            json!({"case":id,"head":request.head,"target":request.target,"width_nm":request.width_nm,
                "outcome":plan.outcome,"probes":plan.probes,"points":plan.points.len()})
        })
        .collect();
    Ok(json!({"schema":"pcbcraft-walkaround-performance/1","environment":environment,
        "target":if cfg!(target_arch="wasm32"){"browser-walkaround-cpu"}else{"native-walkaround-cpu"},
        "debug_assertions":cfg!(debug_assertions),
        "artifact_sha256":digest(artifact_json.as_bytes()),"project_sha256":artifact.project_sha256,"recipe":artifact.recipe,
        "iterations":iterations,"budget":DEFAULT_BUDGET,"outcomes":outcomes,
        "metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),
        "correctness":"identical plans on every iteration; first-iteration plans with at least two points committed through pcb.add_tracks with no exhaustive clearance or short decision about the new copper",
        "unmeasured":["editor preview and commit latency","physical input-to-visible latency","compositor presentation","browser runtime"]}))
}

/// Resolve the same deterministic airwire, channel and crowded targets for both service runners.
pub(super) fn requests(view: pcbcraft_engine::board::view::BoardView<'_>) -> Result<Vec<(String, Request)>> {
    let front = view
        .layers()
        .into_iter()
        .find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(Side::Front))
        .ok_or("fixture requires front copper")?
        .id;
    let mut cases: Vec<(String, Point, Point)> = view
        .ratsnest()?
        .lines
        .into_iter()
        .take(AIRWIRES)
        .enumerate()
        .map(|(index, line)| (format!("airwire.{index}"), line.from.position, line.to.position))
        .collect();
    cases.push(("channel".into(), Point { x_nm: 99_000_000, y_nm: 60_750_000 }, Point { x_nm: 141_000_000, y_nm: 60_750_000 }));
    cases.push(("crowded".into(), Point { x_nm: 98_000_000, y_nm: 30_000_000 }, Point { x_nm: 130_000_000, y_nm: 45_000_000 }));
    let mut requests = Vec::new();
    for (id, head, target) in cases {
        let width_nm = match view.route_rules_at(head, Some(front)).map_err(|e| e.to_string())?.filter(|policy| policy.has_dimensions()) {
            Some(policy) => policy.track_width(None).map_err(|e| e.to_string())?,
            None => view.effective_rules().values.preferred_track_width_nm,
        };
        requests.push((
            id,
            Request { head, target, layer: front, origin: None, width_nm, posture: Posture::Diagonal, flipped: false, previous: Vec::new() },
        ));
    }
    Ok(requests)
}

/// Commit a plan on a fresh session and require the exhaustive pair reference to find no
/// clearance violation or short about the added copper. Plans with nothing to add pass.
fn verify(project: &std::sync::Arc<pcbcraft_engine::Project>, board: BoardTarget, request: &Request, plan: &Plan) -> Result<()> {
    if matches!(plan.outcome, Outcome::Stale | Outcome::Canceled) {
        return Err("a current, uncanceled plan was not produced".into());
    }
    if plan.probes > DEFAULT_BUDGET {
        return Err("plan exceeded its probe budget".into());
    }
    if plan.points.len() < 2 {
        return Ok(());
    }
    let mut session = Session::default();
    session.open_snapshot(project.clone());
    let before: BTreeSet<Uuid> = project.board_view(board).ok_or("missing board")?.tracks().iter().map(|track| track.id).collect();
    fixture::apply(&mut session, ADD_TRACKS, json!({"board":board.board,"layer":request.layer,"width_nm":request.width_nm,"points":plan.points}))?;
    let routed = session.project().and_then(|project| project.board_view(board)).ok_or("missing routed board")?;
    let added: BTreeSet<String> = routed.tracks().iter().filter(|track| !before.contains(&track.id)).map(|track| track.id.to_string()).collect();
    let query = session.clearance_query_with_rules(board, &json!({})).map_err(|e| e.to_string())?;
    let objects = query.objects();
    let offending = query.exhaustive_pairs().decisions.into_iter().any(|decision| {
        (decision.clearance_violation || decision.conflicting_nets)
            && (decision.first.is_some_and(|index| added.contains(&objects[index].name)) || added.contains(&objects[decision.second].name))
    });
    if offending {
        return Err("committed plan breaks clearance in the exhaustive reference".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saved native bytes exercise actual bounded planning, validated commits and queue retirement.
    #[test]
    fn planning_jobs_use_the_shared_oracles_and_cancel_coalesced_work() {
        let environment = Environment {
            revision: "test".into(),
            hardware: "test".into(),
            os: "test".into(),
            runtime: "test".into(),
            compiler: "test".into(),
            profile: "test".into(),
            power: "unknown".into(),
            notes: String::new(),
        };
        let report = jobs(include_str!("../../../../tools/performance/fixtures/reference-v1.json"), environment, 1).unwrap();
        assert_eq!(report["queues"][0]["pending_capacity"], 1);
        assert_eq!(report["queues"][0]["replaced"], 30);
        assert!(report["metrics"].as_array().unwrap().iter().any(|metric| metric["id"] == "planning.atomic_publish.airwire.1"));
    }
}
