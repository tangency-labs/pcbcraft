//! Saved command-generated shove cases, propagation/candidate payload and bounded job costs.
use super::*;
use crate::board_drc::jobs::{Queue, Request as Queued};
use pcbcraft_engine::board::{
    route::{DEFAULT_BUDGET, Mode, Outcome, Plan, PlanningJob, Posture, Request},
    setup::{LayerPurpose, Side},
    track::{ADD_TRACKS, SET_COPPER_LOCKED},
};
use serde_json::Value;

/// Add isolated single/cascade/branch/lock cases to a canonical saved workload through native
/// commands. Retain the resulting bytes once: UUID allocation is intentionally not deterministic.
pub fn fixture(artifact_json: &str, revision: &str) -> Result<String> {
    let mut artifact = load_artifact(artifact_json)?;
    if !matches!(artifact.recipe.as_str(), "reference-v1" | "channels-v1") || revision.trim().is_empty() {
        return Err("shove fixtures require a canonical reference/channel artifact and source revision".into());
    }
    let mut session = Session::default();
    session.open_snapshot(decode_packed(&artifact.project)?.project);
    let layer = front(&session, artifact.board)?;
    for (case, y) in [("single", -20_000_000), ("cascade", -30_000_000), ("branch", -40_000_000), ("lock", -50_000_000)] {
        fixture::apply(
            &mut session,
            ADD_TRACKS,
            json!({"board":artifact.board.board,"layer":layer,"width_nm":if case=="single" {500_000}else{250_000},"points":[point(-60_000_000,y),point(-50_000_000,y)]}),
        )?;
        if case == "cascade" {
            fixture::apply(
                &mut session,
                ADD_TRACKS,
                json!({"board":artifact.board.board,"layer":layer,"width_nm":500_000,"points":[point(-60_000_000,y-700_000),point(-50_000_000,y-700_000)]}),
            )?;
        }
        if case == "branch" {
            fixture::apply(
                &mut session,
                ADD_TRACKS,
                json!({"board":artifact.board.board,"layer":layer,"width_nm":500_000,"points":[point(-50_500_000,y),point(-50_500_000,y-4_000_000)]}),
            )?;
        }
        if case == "lock" {
            let track = session
                .project()
                .and_then(|p| p.board_view(artifact.board))
                .and_then(|v| v.tracks().iter().find(|t| t.start == point(-60_000_000, y)))
                .ok_or("missing lock fixture")?
                .id;
            fixture::apply(&mut session, SET_COPPER_LOCKED, json!({"board":artifact.board.board,"tracks":[track],"locked":true}))?;
        }
    }
    artifact.recipe = format!("shove-v1-{}", artifact.recipe);
    artifact.generator_revision = revision.to_owned();
    artifact.project = pcbcraft_engine::persistence::encode_packed(session.snapshot().ok_or("missing shove fixture")?).map_err(|e| e.to_string())?;
    artifact.project_sha256 = digest(&artifact.project);
    serde_json::to_string(&artifact).map_err(|e| e.to_string())
}

/// Avoid display names and layer order when resolving the selected physical board.
fn front(session: &Session, board: BoardTarget) -> Result<Uuid> {
    session
        .project()
        .and_then(|p| p.board_view(board))
        .ok_or("missing board")?
        .layers()
        .into_iter()
        .find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Front))
        .map(|l| l.id)
        .ok_or_else(|| "missing front copper".into())
}
fn point(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// Drain bounded batches, retaining the largest service call as well as complete elapsed work.
fn drain<T>(samples: &mut BTreeMap<String, Vec<f64>>, id: &str, mut step: T) -> Result<()>
where
    T: FnMut() -> bool,
{
    let started = clock_ms();
    let mut maximum = 0.0_f64;
    for batch in 0..1_000_000 {
        let began = clock_ms();
        let done = step();
        maximum = maximum.max(clock_ms() - began);
        if done {
            samples.entry(format!("{id}.total")).or_default().push(clock_ms() - started);
            samples.entry(format!("{id}.batch_max")).or_default().push(maximum);
            return Ok(());
        }
        if batch == 999_999 {
            return Err("shove diagnostic exceeded work bound".into());
        }
    }
    Err("shove diagnostic did not complete".into())
}

/// Measure synchronous/batched plans, full independent candidate validation, atomic publication,
/// coalesced worker cancellation and contour payload. Oracles and codecs stay outside timings.
/// These CPU/queue distributions do not measure event-loop scheduling or visible presentation.
pub fn run(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    environment.validate()?;
    if !(1..=1000).contains(&iterations) {
        return Err("shove iterations must be between 1 and 1000".into());
    }
    let artifact = load_artifact(artifact_json)?;
    if artifact.recipe.starts_with("shove-joints-v1-") {
        return run_joints(&artifact, environment, iterations, digest(artifact_json.as_bytes()));
    }
    if !artifact.recipe.starts_with("shove-v1-") {
        return Err("use a saved shove-v1 derivative".into());
    }
    let project = decode_packed(&artifact.project)?.project;
    let mut initial = Session::default();
    initial.open_snapshot(project.clone());
    let layer = front(&initial, artifact.board)?;
    let cases: Vec<_> = [("single", -20_000_000), ("cascade", -30_000_000), ("branch", -40_000_000), ("lock", -50_000_000)]
        .into_iter()
        .map(|(name, y)| {
            (
                name,
                Request {
                    mode: Mode::Shove,
                    merge_same_net: false,
                    fixed: Vec::new(),
                    head: point(-58_000_000, y + 2_000_000),
                    target: point(-52_000_000, y + 100_000),
                    layer,
                    origin: None,
                    width_nm: 250_000,
                    posture: Posture::Diagonal,
                    flipped: false,
                    previous: Vec::new(),
                },
            )
        })
        .collect();
    let mut samples = BTreeMap::new();
    let mut first = Vec::<Plan>::new();
    let mut counts = Vec::new();
    let mut queues = Vec::new();
    for iteration in 0..iterations {
        let mut source = Session::default();
        source.open_snapshot(project.clone());
        let context = timed(&mut samples, "shove.context_capture", || source.route_planning(artifact.board)).map_err(|e| e.to_string())?;
        for (case, request) in &cases {
            let mut job =
                timed(&mut samples, &format!("shove.capture.{case}"), || source.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET))
                    .map_err(|e| e.to_string())?;
            drain(&mut samples, &format!("shove.plan.{case}"), || job.step(256))?;
            let plan = job.take_result().map_err(|e| e.to_string())?.ok_or("missing shove plan")?;
            if !job.source().is_current(&source) {
                return Err("shove planning changed its source".into());
            }
            if iteration == 0 {
                let query = source.clearance_query_with_rules(artifact.board, &json!({})).map_err(|e| e.to_string())?;
                if plan != source.plan_route(&query, request, DEFAULT_BUDGET, &|| false).map_err(|e| e.to_string())? {
                    return Err("shove batching differs from synchronous planning".into());
                }
                let expected = match *case {
                    "single" | "branch" => 1,
                    "cascade" => 2,
                    _ => 0,
                };
                if plan.replacements.len() != expected || (*case != "lock" && plan.outcome != Outcome::Reached) {
                    return Err(format!("{case}: unexpected shove behavior: {plan:?}"));
                }
                counts
                    .push(json!({"case":case,"outcome":plan.outcome,"probes":plan.probes,"replacements":plan.replacements.len(),"work":plan.shove}));
                first.push(plan.clone());
            } else {
                let index = cases.iter().position(|(name, _)| name == case).ok_or("missing case")?;
                if plan != first[index] {
                    return Err("shove plans changed between identical iterations".into());
                }
            }
            if plan.outcome != Outcome::Reached {
                continue;
            }
            let mut commit = Session::default();
            commit.open_snapshot(project.clone());
            let params = json!({"board":artifact.board.board,"layer":layer,"width_nm":request.width_nm,"points":plan.points,"replacements":plan.replacements,"expected_revision":commit.revision()});
            let mut edit = commit.prepare_copper_edit(ADD_TRACKS, &params).map_err(|e| e.to_string())?;
            drain(&mut samples, &format!("shove.validation.{case}"), || edit.step(256))?;
            let edit = edit.take_result().map_err(|e| format!("{case}: {e}"))?.ok_or("missing shove edit")?;
            let revision = commit.revision();
            timed(&mut samples, &format!("shove.publish.{case}"), || commit.commit_copper_edit(edit)).map_err(|e| e.to_string())?;
            if commit.revision() != revision + 1 {
                return Err("shove did not publish once".into());
            }
            if iteration == 0 {
                verify(&mut commit, &project, artifact.board, &plan)?;
            }
        }
        let mut queue = Queue::<PlanningJob>::default();
        for id in 0..8 {
            let job = source.prepare_route_plan(&context, cases[1].1.clone(), DEFAULT_BUDGET).map_err(|e| e.to_string())?;
            timed(&mut samples, "shove.queue_replace", || queue.submit(Queued { id, job }));
        }
        let began = clock_ms();
        queue.cancel();
        while queue.busy() {
            let _ = queue.poll();
            if clock_ms() - began > 10_000. {
                return Err("shove cancellation timed out".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::yield_now();
        }
        samples.entry("shove.cancel_acknowledgement".into()).or_default().push(clock_ms() - began);
        queues
            .push(json!({"running_capacity":1,"pending_capacity":1,"replaced":queue.replaced,"canceled":queue.canceled,"completed":queue.completed}));
    }
    Ok(
        json!({"schema":"pcbcraft-shove-performance/1","environment":environment,"artifact_sha256":digest(artifact_json.as_bytes()),"project_sha256":artifact.project_sha256,
        "recipe":artifact.recipe,"iterations":iterations,"budget":DEFAULT_BUDGET,"cases":counts,"queues":queues,
        "metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),
        "correctness":"deterministic synchronous/batched plans; single/cascade/branch displacement and fixed lock; complete compound validation; exhaustive changed-copper decisions; one-step exact Undo/Redo and native reopen; canceled queues retire",
        "unmeasured":["OS input receipt and scheduling","physical input-to-visible preview latency","compositor presentation","isolated allocator/index overhead","endurance"]}),
    )
}

/// Check final changed copper against the exhaustive contour oracle and exact native history.
fn verify(session: &mut Session, source: &std::sync::Arc<pcbcraft_engine::Project>, board: BoardTarget, plan: &Plan) -> Result<()> {
    verify_changed(session, source, board, plan.replacements.iter().map(|r| r.track).chain(plan.via_moves.iter().map(|m| m.via)).collect())
}

/// Verify the changed source IDs and every newly allocated fragment against the exhaustive oracle.
fn verify_changed(
    session: &mut Session,
    source: &std::sync::Arc<pcbcraft_engine::Project>,
    board: BoardTarget,
    replaced: BTreeSet<Uuid>,
) -> Result<()> {
    let before = source.board_view(board).ok_or("missing source")?;
    let old: BTreeSet<_> = before.tracks().iter().map(|t| t.id).chain(before.vias().iter().map(|v| v.id)).collect();
    let query = session.clearance_query_with_rules(board, &json!({})).map_err(|e| e.to_string())?;
    let changed = |i: usize| Uuid::parse_str(&query.objects()[i].name).is_ok_and(|id| !old.contains(&id) || replaced.contains(&id));
    if query
        .exhaustive_pairs()
        .decisions
        .iter()
        .any(|d| (d.first.is_some_and(changed) || changed(d.second)) && (d.clearance_violation || d.conflicting_nets))
    {
        return Err("shove disagrees with the exhaustive changed-copper oracle".into());
    }
    let after = pcbcraft_engine::persistence::encode_documents(session.snapshot().ok_or("missing result")?).map_err(|e| e.to_string())?;
    let reopened = pcbcraft_engine::persistence::decode_documents(&after).map_err(|e| e.to_string())?;
    if pcbcraft_engine::persistence::encode_documents(reopened).map_err(|e| e.to_string())? != after {
        return Err("shove native reopen differs".into());
    }
    session.apply("edit.undo", &Value::Null).map_err(|e| e.to_string())?;
    if pcbcraft_engine::persistence::encode_documents(session.snapshot().ok_or("missing undo")?).map_err(|e| e.to_string())?
        != pcbcraft_engine::persistence::encode_documents(source.clone()).map_err(|e| e.to_string())?
    {
        return Err("shove undo differs".into());
    }
    session.apply("edit.redo", &Value::Null).map_err(|e| e.to_string())?;
    if pcbcraft_engine::persistence::encode_documents(session.snapshot().ok_or("missing redo")?).map_err(|e| e.to_string())? != after {
        return Err("shove redo differs".into());
    }
    Ok(())
}

/// Keep the native project bytes unchanged while selecting the versioned rendered shove trace.
pub fn rendered_fixture(artifact_json: &str) -> Result<String> {
    let mut artifact = load_artifact(artifact_json)?;
    if !artifact.recipe.starts_with("shove-v1-") {
        return Err("rendered shove requires a saved shove fixture".into());
    }
    artifact.trace_id = super::replay::SHOVE_TRACE_ID.into();
    artifact.trace = super::replay::Step::shove();
    serde_json::to_string(&artifact).map_err(|e| e.to_string())
}

/// Validate the real editor commit against the same saved single-neighbor source and oracle.
pub(super) fn verify_replay(session: &mut Session, original: &[u8], board: BoardTarget) -> Result<()> {
    let project = decode_packed(original)?.project;
    let mut source = Session::default();
    source.open_snapshot(project.clone());
    let layer = front(&source, board)?;
    let request = Request {
        mode: Mode::Shove,
        merge_same_net: false,
        fixed: Vec::new(),
        head: point(-58_000_000, -18_000_000),
        target: point(-52_000_000, -19_900_000),
        layer,
        origin: None,
        width_nm: 250_000,
        posture: Posture::Diagonal,
        flipped: false,
        previous: Vec::new(),
    };
    let query = source.clearance_query_with_rules(board, &json!({})).map_err(|e| e.to_string())?;
    let plan = source.plan_route(&query, &request, DEFAULT_BUDGET, &|| false).map_err(|e| e.to_string())?;
    if plan.replacements.is_empty() {
        return Err("rendered shove has no displaced source".into());
    }
    verify(session, &project, board, &plan)
}

/// Extend a saved shove workload with the via/follower and connected D-drag cases from #446.
/// UUIDs are allocated by the commands, so the generated archive is saved once for replay.
pub fn joint_fixture(artifact_json: &str, revision: &str) -> Result<String> {
    let mut artifact = load_artifact(artifact_json)?;
    if !artifact.recipe.starts_with("shove-v1-") || revision.trim().is_empty() {
        return Err("joint fixtures require a saved shove-v1 artifact and generator revision".into());
    }
    let mut session = Session::default();
    session.open_snapshot(decode_packed(&artifact.project)?.project);
    let board = artifact.board;
    let front = front(&session, board)?;
    let back = session
        .project()
        .and_then(|p| p.board_view(board))
        .ok_or("missing board")?
        .layers()
        .into_iter()
        .find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Back))
        .ok_or("missing back copper")?
        .id;
    let via = point(-55_000_000, -60_000_000);
    fixture::apply(&mut session, pcbcraft_engine::ADD_VIA, json!({"board":board.board,"at":via}))?;
    for layer in [front, back] {
        fixture::apply(
            &mut session,
            ADD_TRACKS,
            json!({"board":board.board,"layer":layer,"width_nm":250000,"points":[point(via.x_nm,via.y_nm-3_000_000),via]}),
        )?;
    }
    fixture::apply(
        &mut session,
        ADD_TRACKS,
        json!({"board":board.board,"layer":front,"width_nm":250000,"points":[point(-58_000_000,-16_000_000),point(-58_000_000,-18_000_000),point(-52_000_000,-18_000_000)]}),
    )?;
    artifact.recipe = format!("shove-joints-v1-{}", artifact.recipe);
    artifact.generator_revision = revision.to_owned();
    artifact.project = pcbcraft_engine::persistence::encode_packed(session.snapshot().ok_or("missing project")?).map_err(|e| e.to_string())?;
    artifact.project_sha256 = digest(&artifact.project);
    serde_json::to_string(&artifact).map_err(|e| e.to_string())
}

/// Measure the bounded preparation and atomic publication of one edit. Capture the independent
/// drag preview before publication so that the fixture can compare it to the published geometry.
fn replay_edit(
    session: &mut Session,
    command: &str,
    mut params: Value,
    name: &str,
    samples: &mut BTreeMap<String, Vec<f64>>,
) -> Result<Vec<pcbcraft_engine::board::track::replacements::Replacement>> {
    let revision = session.revision();
    params["expected_revision"] = json!(revision);
    let mut edit = session.prepare_copper_edit(command, &params).map_err(|e| e.to_string())?;
    drain(samples, &format!("{name}.prepare"), || edit.step(256))?;
    let ready = edit.take_result().map_err(|e| format!("{name}: {e}"))?.ok_or("missing compound edit")?;
    let neighbors = ready.drag_replacements().to_vec();
    timed(samples, &format!("{name}.publish"), || session.commit_copper_edit(ready)).map_err(|e| e.to_string())?;
    if session.revision() != revision + 1 {
        return Err(format!("{name} must publish exactly one revision"));
    }
    Ok(neighbors)
}

/// Replay a connected D-drag and check the selected tracks and the complete pushed rail path.
fn replay_drag(
    project: &std::sync::Arc<pcbcraft_engine::Project>,
    board: BoardTarget,
    layer: Uuid,
    samples: &mut BTreeMap<String, Vec<f64>>,
    check_history: bool,
) -> Result<()> {
    let before = project.board_view(board).ok_or("missing board")?;
    let by_start = |p| before.tracks().iter().find(|t| t.layer == layer && t.start == p).ok_or("missing drag fixture segment");
    let fixed = point(-58_000_000, -16_000_000);
    let joint = point(-58_000_000, -19_900_000);
    let tip = point(-52_000_000, joint.y_nm);
    let primary = by_start(point(-58_000_000, -18_000_000))?.id;
    let follower = by_start(fixed)?.id;
    let rail = by_start(point(-60_000_000, -20_000_000))?;
    let mut session = Session::default();
    session.open_snapshot(project.clone());
    let params =
        json!({"board":board.board,"mode":"shove","tracks":[{"track":primary,"start":joint,"end":tip},{"track":follower,"start":fixed,"end":joint}]});
    let paths = replay_edit(&mut session, pcbcraft_engine::RESHAPE_TRACKS, params, "drag", samples)?;
    let [path] = paths.as_slice() else {
        return Err("drag must push one neighboring rail".into());
    };
    let after = session.project().and_then(|p| p.board_view(board)).ok_or("missing drag result")?;
    if path.track != rail.id
        || path.points.len() <= 2
        || path.points.first() != Some(&rail.start)
        || path.points.last() != Some(&rail.end)
        || !after.tracks().iter().any(|t| t.id == primary && t.start == joint && t.end == tip)
        || !after.tracks().iter().any(|t| t.id == follower && t.start == fixed && t.end == joint)
        || !path.points.windows(2).enumerate().all(|(i, segment)| {
            after
                .tracks()
                .iter()
                .any(|t| (i != 0 || t.id == rail.id) && t.layer == rail.layer && t.width_nm == rail.width_nm && [t.start, t.end] == segment)
        })
    {
        return Err("drag changed the joint, lost a rail endpoint or published a different path than its preview".into());
    }
    if check_history {
        verify_changed(&mut session, project, board, BTreeSet::from([primary, follower, rail.id]))?;
    }
    Ok(())
}

/// Measure via planning, follower validation and connected drag using two fresh sessions per iteration.
/// Exhaustive clearance, native reopen and exact history checks run outside the timed boundaries.
fn run_joints(artifact: &Artifact, environment: Environment, iterations: usize, artifact_hash: String) -> Result<Value> {
    let project = decode_packed(&artifact.project)?.project;
    let mut source = Session::default();
    source.open_snapshot(project.clone());
    let board = artifact.board;
    let layer = front(&source, board)?;
    let start = point(-58_000_000, -59_700_000);
    let end = point(-52_000_000, -59_700_000);
    let request = Request {
        head: start,
        target: end,
        layer,
        origin: None,
        width_nm: 250000,
        mode: Mode::ShoveVias,
        posture: Posture::Diagonal,
        flipped: false,
        fixed: Vec::new(),
        previous: Vec::new(),
        merge_same_net: false,
    };
    let view = project.board_view(board).ok_or("missing board")?;
    let at = point(-55_000_000, -60_000_000);
    let via = view.vias().iter().find(|v| v.at == at).ok_or("missing fixture via")?.id;
    let followers: Vec<_> = view.tracks().iter().filter(|t| t.end == at).collect();
    if followers.len() != 2 || followers[0].layer == followers[1].layer {
        return Err("fixture must attach a follower on each side".into());
    }
    let mut samples = BTreeMap::new();
    let mut first = None;
    for iteration in 0..iterations {
        let mut session = Session::default();
        session.open_snapshot(project.clone());
        let context = session.route_planning(board).map_err(|e| e.to_string())?;
        let budget = request.mode.default_probe_budget();
        let mut job = session.prepare_route_plan(&context, request.clone(), budget).map_err(|e| e.to_string())?;
        drain(&mut samples, "via.plan", || job.step(256))?;
        let plan = job.take_result().map_err(|e| e.to_string())?.ok_or("missing via plan")?;
        if plan.outcome != Outcome::Reached
            || plan.points != [start, end]
            || plan.via_moves.len() != 1
            || plan.via_moves[0].via != via
            || plan.via_moves[0].at == at
            || plan.replacements.len() != 2
            || first.as_ref().is_some_and(|old| old != &plan)
        {
            return Err(format!("unexpected via displacement: {plan:?}"));
        }
        let params = json!({"board":board.board,"layer":layer,"points":plan.points,"width_nm":request.width_nm,"via_moves":plan.via_moves,"replacements":plan.replacements});
        replay_edit(&mut session, ADD_TRACKS, params, "via", &mut samples)?;
        let result = session.project().and_then(|p| p.board_view(board)).ok_or("missing via result")?;
        let new_at = plan.via_moves[0].at;
        if result.vias().iter().find(|v| v.id == via).is_none_or(|v| v.at != new_at)
            || followers.iter().any(|source| {
                !result.tracks().iter().any(|t| {
                    t.id == source.id && t.layer == source.layer && t.start == source.start && t.end == new_at && t.width_nm == source.width_nm
                })
            })
            || !result.tracks().iter().any(|t| t.layer == layer && t.start == start && t.end == end && t.width_nm == request.width_nm)
        {
            return Err("route, via and followers disagree with plan".into());
        }
        if iteration == 0 {
            verify_changed(&mut session, &project, board, followers.iter().map(|t| t.id).chain([via]).collect())?;
        }
        replay_drag(&project, board, layer, &mut samples, iteration == 0)?;
        first = Some(plan);
    }
    Ok(
        json!({"schema":"pcbcraft-shove-joints/1","environment":environment,"recipe":artifact.recipe,"iterations":iterations,"artifact_sha256":artifact_hash,"project_sha256":artifact.project_sha256,"via_plan":first,"budget":request.mode.default_probe_budget(),"metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),"correctness":"source-addressed via center and two layer endpoints; connected D drag and previewed rail path; exhaustive changed copper, native reopen, exact Undo/Redo","unmeasured":["visible latency","allocator overhead"]}),
    )
}
