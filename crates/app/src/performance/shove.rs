//! Shove planning service costs for #247, separate from editor input and presentation.
use super::*;
use pcbcraft_engine::board::{
    route::{
        Outcome, SHOVE_TRACKS,
        shove::{DEFAULT_BUDGET, Request, Shove, Shover},
    },
    track::{ADD_TRACKS, ADD_VIA, Track, Via, preparation::CopperEditJob},
};

/// The pushing case's identity in reports.
const PUSH: &str = "push";
/// Search units per host batch, as the editor's snapshot workers and live MCP edits step jobs.
const BATCH: usize = 256;

/// Plan a straight shove along each walkaround case (the fixture's first airwires, the channel
/// corridor and the crowded diagonal) and one pushing case over one captured query per
/// iteration, timing every complete plan. The pushing case adds, through the shared track and
/// via commands, a stack of four parallel tracks and a via with a track ending on it in free
/// space below the fixture's copper, then routes along the stack; it must be reached while
/// moving copper at a push depth of at least two, and its samples are kept apart from the
/// straight cases. Outside the timed boundary, every iteration must reproduce the first
/// iteration's plans, and on the first iteration each reached plan is committed through
/// `pcb.shove_tracks` on a fresh session and checked by [`verify`]. Every iteration also drives
/// the pushing case's `pcb.shove_tracks` copper job in host batches, timing each uninterrupted
/// step by stage, requiring its result to be the plan, and measures how long a snapshot
/// worker takes to acknowledge a cancel issued while it plans. Any disagreement fails the run.
pub fn run(artifact_json: &str, environment: Environment, iterations: usize) -> Result<serde_json::Value> {
    environment.validate()?;
    if !(1..=1000).contains(&iterations) {
        return Err("shove iterations must be between 1 and 1000".into());
    }
    let artifact = load_artifact(artifact_json)?;
    let project = decode_packed(&artifact.project)?.project;
    let view = project.board_view(artifact.board).ok_or("missing board")?;
    let requests: Vec<(String, Request)> = super::walkaround::requests(view)?
        .into_iter()
        .filter(|(_, r)| r.head != r.target)
        .map(|(id, r)| (id, Request { layer: r.layer, shover: Shover::Route { points: vec![r.head, r.target], width_nm: r.width_nm } }))
        .collect();
    let (pushing, push_request) = push_case(&project, artifact.board)?;
    let mut samples = BTreeMap::new();
    let mut first: Vec<Shove> = Vec::new();
    let mut first_push = None;
    let mut cancels = Vec::new();
    let mut job_steps = None;
    for iteration in 0..iterations {
        let mut session = Session::default();
        session.open_snapshot(project.clone());
        let query =
            timed(&mut samples, "shove.prepare", || session.clearance_query_with_rules(artifact.board, &json!({}))).map_err(|e| e.to_string())?;
        let began = clock_ms();
        let mut plans = Vec::new();
        for (id, request) in &requests {
            let plan = timed(&mut samples, &format!("shove.plan.{id}"), || session.plan_shove(&query, request, DEFAULT_BUDGET, &|| false))
                .map_err(|e| format!("{id}: {e}"))?;
            plans.push(plan);
        }
        samples.entry("shove.plan.all".into()).or_default().push(clock_ms() - began);
        let mut pushed = Session::default();
        pushed.open_snapshot(pushing.clone());
        let query =
            timed(&mut samples, "shove.push.prepare", || pushed.clearance_query_with_rules(artifact.board, &json!({}))).map_err(|e| e.to_string())?;
        let push = timed(&mut samples, "shove.plan.push", || pushed.plan_shove(&query, &push_request, DEFAULT_BUDGET, &|| false))
            .map_err(|e| format!("{PUSH}: {e}"))?;
        if iteration == 0 {
            for ((id, request), plan) in requests.iter().zip(&plans) {
                verify(&project, artifact.board, request, plan).map_err(|e| format!("{id}: {e}"))?;
            }
            if push.outcome != Outcome::Reached || push.moved == 0 || push.depth < 2 {
                return Err(format!("the pushing case must reach its target while pushing at depth two or more: {push:?}"));
            }
            verify(&pushing, artifact.board, &push_request, &push).map_err(|e| format!("{PUSH}: {e}"))?;
            first = plans;
            first_push = Some(push.clone());
        } else if plans != first || first_push.as_ref() != Some(&push) {
            return Err("shove plans differ between iterations".into());
        }
        let steps = job(&pushing, artifact.board, &push_request, &push, &mut samples)?;
        job_steps = Some(job_steps.map_or((steps, steps), |(low, high): (usize, usize)| (low.min(steps), high.max(steps))));
        cancels.push(cancel(&pushing, artifact.board, &push_request, &mut samples)?);
    }
    let push = first_push.ok_or("missing pushing plan")?;
    let mut outcomes: Vec<_> = requests
        .iter()
        .zip(&first)
        .map(|((id, _), plan)| {
            json!({"case":id,"outcome":plan.outcome,"probes":plan.probes,"depth":plan.depth,"moved":plan.moved,"blocker":plan.blocker})
        })
        .collect();
    outcomes.push(json!({"case":PUSH,"outcome":push.outcome,"probes":push.probes,"depth":push.depth,"moved":push.moved,"blocker":push.blocker,
        "reshaped":push.reshaped.len(),"added":push.added.len(),"removed":push.removed.len(),"vias":push.vias.len()}));
    let mut canceled_while: BTreeMap<String, usize> = BTreeMap::new();
    for stage in cancels {
        *canceled_while.entry(stage).or_default() += 1;
    }
    Ok(json!({"schema":"pcbcraft-shove-performance/2","environment":environment,
        "target":if cfg!(target_arch="wasm32"){"browser-shove-cpu"}else{"native-shove-cpu"},
        "debug_assertions":cfg!(debug_assertions),
        "artifact_sha256":digest(artifact_json.as_bytes()),"project_sha256":artifact.project_sha256,"recipe":artifact.recipe,
        "push_case":{"project_sha256":digest(&pcbcraft_engine::persistence::encode_packed(pushing).map_err(|e| e.to_string())?),
            "request":{"layer":push_request.layer,"shover":match &push_request.shover { Shover::Route { points, width_nm } => json!({"points":points,"width_nm":width_nm}), Shover::Drag { tracks } => json!({"tracks":tracks}) }}},
        "iterations":iterations,"budget":DEFAULT_BUDGET,"batch_units":BATCH,"push_job_steps":job_steps.map(|(min, max)| json!({"min":min,"max":max})),"outcomes":outcomes,
        "cancel_requested_while":canceled_while,
        "metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),
        "correctness":"identical plans on every iteration; first-iteration reached plans committed through pcb.shove_tracks equal the plan (every kept track and via, new tracks up to fresh identities), keep every connection that existed before and have no exhaustive clearance or short decision about the changed copper; the batched pushing job yields the planned board every iteration",
        "unmeasured":["editor preview and commit latency","physical input-to-visible latency","compositor presentation","browser runtime"]}))
}

/// The pushing case: on a copy of the fixture, add through the shared commands four parallel
/// 0.25 mm tracks 0.45 mm apart from (100, 80) mm to (140, 80) mm and below, and a via at
/// (115, 80.9) mm with a track from it to (115, 84) mm, in free space below the fixture's
/// copper. The route runs along the stack at y = 80.4 mm from 105 to 135 mm: it pushes the
/// stack down in turn and the via up with its track following.
fn push_case(project: &std::sync::Arc<pcbcraft_engine::Project>, board: BoardTarget) -> Result<(std::sync::Arc<pcbcraft_engine::Project>, Request)> {
    let mut session = Session::default();
    session.open_snapshot(project.clone());
    let view = project.board_view(board).ok_or("missing board")?;
    let (_, sample) = super::walkaround::requests(view)?.into_iter().next().ok_or("missing route requests")?;
    let (layer, width_nm) = (sample.layer, 250_000);
    let at = |x_nm: i64, y_nm: i64| Point { x_nm, y_nm };
    for k in 0..4 {
        let y = 80_000_000 - k * 450_000;
        fixture::apply(
            &mut session,
            ADD_TRACKS,
            json!({"board":board.board,"layer":layer,"width_nm":width_nm,"points":[at(100_000_000, y), at(140_000_000, y)]}),
        )?;
    }
    fixture::apply(
        &mut session,
        ADD_VIA,
        json!({"board":board.board,"at":at(115_000_000, 80_900_000),"via_diameter_nm":600_000,"via_drill_nm":300_000}),
    )?;
    fixture::apply(
        &mut session,
        ADD_TRACKS,
        json!({"board":board.board,"layer":layer,"width_nm":width_nm,"points":[at(115_000_000, 80_900_000), at(115_000_000, 84_000_000)]}),
    )?;
    let pushing = session.snapshot().ok_or("missing pushing fixture")?;
    let request = Request { layer, shover: Shover::Route { points: vec![at(105_000_000, 80_400_000), at(135_000_000, 80_400_000)], width_nm } };
    Ok((pushing, request))
}

/// The `pcb.shove_tracks` parameters of a route request on `board`.
fn params(board: BoardTarget, request: &Request) -> Result<serde_json::Value> {
    let Shover::Route { points, width_nm } = &request.shover else { return Err("shove diagnostics plan routes".into()) };
    Ok(json!({"board":board.board,"layer":request.layer,"width_nm":width_nm,"points":points}))
}

/// Drive the pushing case's copper job in host batches, recording the longest uninterrupted
/// step of each stage (`shove.job.step_max.<stage>`) and the whole job. The prepared edit must
/// publish exactly the plan. Returns the number of steps.
fn job(
    project: &std::sync::Arc<pcbcraft_engine::Project>,
    board: BoardTarget,
    request: &Request,
    plan: &Shove,
    samples: &mut BTreeMap<String, Vec<f64>>,
) -> Result<usize> {
    let mut session = Session::default();
    session.open_snapshot(project.clone());
    let mut params = params(board, request)?;
    params["expected_revision"] = json!(session.revision());
    let mut job = session.prepare_copper_edit(SHOVE_TRACKS, &params).map_err(|e| e.to_string())?;
    let mut largest: BTreeMap<&'static str, f64> = BTreeMap::new();
    let (began, mut steps) = (clock_ms(), 0);
    loop {
        let stage = job.progress().stage;
        let start = clock_ms();
        let done = job.step(BATCH);
        let elapsed = clock_ms() - start;
        largest.entry(stage).and_modify(|maximum| *maximum = maximum.max(elapsed)).or_insert(elapsed);
        steps += 1;
        if done {
            break;
        }
        if steps > 10_000_000 {
            return Err("shove job exceeded work bound".into());
        }
    }
    samples.entry("shove.job.total".into()).or_default().push(clock_ms() - began);
    let overall = largest.values().copied().fold(0., f64::max);
    samples.entry("shove.job.step_max".into()).or_default().push(overall);
    for (stage, maximum) in largest {
        samples.entry(format!("shove.job.step_max.{stage}")).or_default().push(maximum);
    }
    let prepared = job.take_result().map_err(|e| e.to_string())?.ok_or("missing prepared shove")?;
    let before = session.project().and_then(|p| p.board_view(board)).map(|v| copper(v.tracks(), v.vias())).ok_or("missing board")?;
    session.commit_copper_edit(prepared).map_err(|e| e.to_string())?;
    let after = session.project().and_then(|p| p.board_view(board)).map(|v| copper(v.tracks(), v.vias())).ok_or("missing shoved board")?;
    agree(&before, &after, request, plan).map_err(|e| format!("batched job: {e}"))?;
    Ok(steps)
}

/// Submit the pushing case to a snapshot worker queue and cancel it as soon as its progress
/// reports planning, polling for a terminal result meanwhile on every target. Only a
/// cancellation the queue confirms (a completion without output) is timed, as
/// `shove.job.cancel_acknowledgement`; a job that finished before the cancel took effect is
/// reported as `finished`, and a worker failure or engine refusal fails the run. Returns the
/// stage the job reported when the cancel was issued, or `finished`.
fn cancel(
    project: &std::sync::Arc<pcbcraft_engine::Project>,
    board: BoardTarget,
    request: &Request,
    samples: &mut BTreeMap<String, Vec<f64>>,
) -> Result<String> {
    use crate::board_drc::jobs::{Completion, Queue, Request as Queued};
    use pcbcraft_engine::{CommandError, board::track::preparation::PreparedCopperEdit};
    /// Fail on a worker or engine error; otherwise whether the queue confirmed a cancellation.
    fn canceled(completion: Completion<std::result::Result<PreparedCopperEdit, CommandError>>) -> Result<bool> {
        match completion.report {
            Ok(None) => Ok(true),
            Ok(Some(Ok(_))) => Ok(false),
            Ok(Some(Err(error))) => Err(format!("canceled shove job was refused: {error}")),
            Err(error) => Err(format!("canceled shove job failed: {error}")),
        }
    }
    let mut session = Session::default();
    session.open_snapshot(project.clone());
    let mut params = params(board, request)?;
    params["expected_revision"] = json!(session.revision());
    let job = session.prepare_copper_edit(SHOVE_TRACKS, &params).map_err(|e| e.to_string())?;
    let mut queue = Queue::<CopperEditJob>::default();
    queue.submit(Queued { id: 0, job });
    let waited = clock_ms();
    let stage = loop {
        let stage = queue.progress().map(|p| p.stage);
        if stage == Some("Planning shove") {
            break stage.unwrap_or("unknown");
        }
        if clock_ms() - waited > 10_000. {
            return Err("the shove job never reported planning".into());
        }
        // Incremental queues advance only when polled; a native worker may already be done.
        if let Some(completion) = queue.poll() {
            return if canceled(completion)? { Err("the shove job was canceled before planning".into()) } else { Ok("finished".into()) };
        }
        #[cfg(not(any(target_arch = "wasm32", test)))]
        std::hint::spin_loop();
    };
    let began = clock_ms();
    queue.cancel();
    let completion = loop {
        if let Some(completion) = queue.poll() {
            break completion;
        }
        if !queue.busy() {
            return Err("the canceled shove job left no completion".into());
        }
        if clock_ms() - began > 10_000. {
            return Err("shove cancellation timed out".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        std::hint::spin_loop();
    };
    let elapsed = clock_ms() - began;
    if !canceled(completion)? {
        return Ok("finished".into());
    }
    samples.entry("shove.job.cancel_acknowledgement".into()).or_default().push(elapsed);
    Ok(stage.to_owned())
}

/// Tracks and vias by identity.
type Copper = (BTreeMap<Uuid, Track>, BTreeMap<Uuid, Via>);

/// A board's tracks and vias by identity.
fn copper(tracks: &[Track], vias: &[Via]) -> Copper {
    (tracks.iter().map(|t| (t.id, t.clone())).collect(), vias.iter().map(|v| (v.id, v.clone())).collect())
}

/// Require the committed copper to be exactly the plan applied to the copper before it: every
/// kept track and via equal to its planned state (moved, reshaped or unchanged), removed tracks
/// gone, and the new tracks equal to the route's legs and the added segments up to their fresh
/// identities. The expectation is built here from the plan, independently of the engine.
fn agree(before: &Copper, after: &Copper, request: &Request, plan: &Shove) -> Result<()> {
    let (mut tracks, mut vias) = before.clone();
    for ends in plan.dragged.iter().chain(&plan.reshaped) {
        let track = tracks.get_mut(&ends.track).ok_or("plan reshapes an unknown track")?;
        (track.start, track.end) = (ends.start, ends.end);
    }
    for removed in &plan.removed {
        tracks.remove(removed).ok_or("plan removes an unknown track")?;
    }
    for moved in &plan.vias {
        vias.get_mut(&moved.via).ok_or("plan moves an unknown via")?.at = moved.at;
    }
    let mut expected = Vec::new();
    for added in &plan.added {
        let like = before.0.get(&added.like).ok_or("plan adds like an unknown track")?;
        expected.push(Track { id: Uuid::nil(), locked: false, start: added.start, end: added.end, ..like.clone() });
    }
    if let Shover::Route { width_nm, .. } = &request.shover {
        expected.extend(plan.points.windows(2).map(|leg| Track {
            id: Uuid::nil(),
            locked: false,
            layer: request.layer,
            width_nm: *width_nm,
            start: leg[0],
            end: leg[1],
        }));
    }
    let key = |t: &Track| (t.start.x_nm, t.start.y_nm, t.end.x_nm, t.end.y_nm, t.width_nm);
    expected.sort_by_key(key);
    let kept: BTreeMap<Uuid, Track> = after.0.iter().filter(|(id, _)| before.0.contains_key(*id)).map(|(id, t)| (*id, t.clone())).collect();
    let mut fresh: Vec<Track> = after.0.values().filter(|t| !before.0.contains_key(&t.id)).map(|t| Track { id: Uuid::nil(), ..t.clone() }).collect();
    fresh.sort_by_key(key);
    if kept != tracks {
        return Err("committed tracks differ from the plan".into());
    }
    if after.1 != vias {
        return Err("committed vias differ from the plan".into());
    }
    if fresh != expected {
        return Err("committed new tracks differ from the route and added segments".into());
    }
    Ok(())
}

/// Copper joined by contact, by query name, from the exhaustive pair reference: a union of
/// every touching pair of non-mechanical objects. Independent of the engine's island builder.
fn joined(query: &ClearanceQuery, pairs: &Results) -> BTreeMap<String, usize> {
    let objects = query.objects();
    let mut parent: Vec<usize> = (0..objects.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for decision in pairs.decisions.iter().filter(|d| d.touches) {
        let Some(first) = decision.first else { continue };
        if objects[first].mechanical || objects[decision.second].mechanical {
            continue;
        }
        let (a, b) = (root(&mut parent, first), root(&mut parent, decision.second));
        parent[a] = b;
    }
    (0..objects.len()).filter(|i| !objects[*i].mechanical).map(|i| (objects[i].name.clone(), root(&mut parent, i))).collect()
}

/// Commit a reached plan on a fresh session and require [`agree`], that every connection that
/// existed before (two objects joined by contact) still exists, and that the exhaustive pair
/// reference finds no clearance violation or short the shove is responsible for ([`regressions`]). Other plans move
/// nothing and pass.
fn verify(project: &std::sync::Arc<pcbcraft_engine::Project>, board: BoardTarget, request: &Request, plan: &Shove) -> Result<()> {
    if matches!(plan.outcome, Outcome::Stale | Outcome::Canceled) {
        return Err("a current, uncanceled plan was not produced".into());
    }
    if plan.probes > DEFAULT_BUDGET {
        return Err("plan exceeded its probe budget".into());
    }
    let params = params(board, request)?;
    if plan.outcome != Outcome::Reached {
        return Ok(());
    }
    let mut session = Session::default();
    session.open_snapshot(project.clone());
    let before_view = project.board_view(board).ok_or("missing board")?;
    let before = copper(before_view.tracks(), before_view.vias());
    let query = session.clearance_query_with_rules(board, &json!({})).map_err(|e| e.to_string())?;
    let before_pairs = query.exhaustive_pairs();
    let connected = joined(&query, &before_pairs);
    let before_query = query;
    fixture::apply(&mut session, SHOVE_TRACKS, params)?;
    let shoved = session.project().and_then(|project| project.board_view(board)).ok_or("missing shoved board")?;
    let after = copper(shoved.tracks(), shoved.vias());
    agree(&before, &after, request, plan)?;
    let changed: BTreeSet<String> = after
        .0
        .values()
        .filter(|t| before.0.get(&t.id) != Some(*t))
        .map(|t| t.id.to_string())
        .chain(after.1.values().filter(|v| before.1.get(&v.id) != Some(*v)).map(|v| v.id.to_string()))
        .collect();
    let query = session.clearance_query_with_rules(board, &json!({})).map_err(|e| e.to_string())?;
    let pairs = query.exhaustive_pairs();
    let now = joined(&query, &pairs);
    let mut groups: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (name, group) in &connected {
        if let Some(island) = now.get(name) {
            groups.entry(*group).or_default().insert(*island);
        }
    }
    if groups.values().any(|islands| islands.len() > 1) {
        return Err("committed shove loses a connection".into());
    }
    if let Some((a, b)) = regressions(&violations(&before_query, &before_pairs), &query, &pairs, &changed).into_iter().next() {
        return Err(format!("committed shove breaks clearance in the exhaustive reference: {a} / {b}"));
    }
    Ok(())
}

/// Violating pairs (clearance or short) of a query's exhaustive reference, by object names.
fn violations(query: &ClearanceQuery, pairs: &Results) -> BTreeSet<(String, String)> {
    let objects = query.objects();
    pairs
        .decisions
        .iter()
        .filter(|d| d.clearance_violation || d.conflicting_nets)
        .filter_map(|d| {
            let (a, b) = (objects[d.first?].name.clone(), objects[d.second].name.clone());
            Some(if a <= b { (a, b) } else { (b, a) })
        })
        .collect()
}

/// Violating pairs after a shove that it is responsible for: every pair involving `changed`
/// copper, and every pair that did not violate before (`before`, by object names), even when
/// neither object moved, as when a route joins a track to a net with a wider class clearance.
fn regressions(before: &BTreeSet<(String, String)>, query: &ClearanceQuery, pairs: &Results, changed: &BTreeSet<String>) -> Vec<(String, String)> {
    violations(query, pairs).into_iter().filter(|pair| changed.contains(&pair.0) || changed.contains(&pair.1) || !before.contains(pair)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::{
        INITIALIZE_BOARD_SETUP,
        board::rules::{SET_RULES, Values},
    };

    /// A violation between copper that did not move still counts when it is new: here a rule
    /// change, standing in for a route that gives an untouched track a wider class clearance,
    /// makes two unmoved tracks 0.3 mm apart violate a 0.4 mm rule.
    #[test]
    fn regressions_include_new_violations_between_unmoved_copper() {
        let mut s = Session::default();
        s.apply("project.new", &json!({"name":"Pairs","create_board":true})).unwrap();
        s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
        let project = s.project().unwrap();
        let target = project.board_target().unwrap();
        let layer = project
            .board_view(target)
            .unwrap()
            .layers()
            .into_iter()
            .find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper)
            .unwrap()
            .id;
        for y in [0, 550_000] {
            let points = [Point { x_nm: 0, y_nm: y }, Point { x_nm: 5_000_000, y_nm: y }];
            s.apply(ADD_TRACKS, &json!({"layer":layer,"width_nm":250_000,"points":points,"expected_revision":s.revision()})).unwrap();
        }
        let before = s.clearance_query_with_rules(target, &json!({})).unwrap();
        let before = violations(&before, &before.exhaustive_pairs());
        assert!(before.is_empty());
        let rules = Values { clearance_nm: 400_000, ..Values::default() };
        s.apply(SET_RULES, &json!({"board":target.board,"rules":rules,"expected_revision":s.revision()})).unwrap();
        let after = s.clearance_query_with_rules(target, &json!({})).unwrap();
        let found = regressions(&before, &after, &after.exhaustive_pairs(), &BTreeSet::new());
        assert_eq!(found.len(), 1, "{found:?}");
    }
}
