//! Shove planning service costs for #247, separate from editor input and presentation.
use super::*;
use pcbcraft_engine::board::route::{
    Outcome, SHOVE_TRACKS,
    shove::{DEFAULT_BUDGET, Request, Shove, Shover},
};

/// Plan a straight shove along each walkaround case (the fixture's first airwires, the channel
/// corridor and the crowded diagonal) over one captured query per iteration, timing every
/// complete plan. Outside the timed boundary, every iteration must reproduce the first
/// iteration's plans, and on the first iteration each reached plan is committed through
/// `pcb.shove_tracks` on a fresh session; the exhaustive pair reference must then find no
/// clearance violation or short about the changed copper. Any disagreement fails the run.
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
    let mut samples = BTreeMap::new();
    let mut first: Vec<Shove> = Vec::new();
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
        if iteration == 0 {
            for ((id, request), plan) in requests.iter().zip(&plans) {
                verify(&project, artifact.board, request, plan).map_err(|e| format!("{id}: {e}"))?;
            }
            first = plans;
        } else if plans != first {
            return Err("shove plans differ between iterations".into());
        }
    }
    let outcomes: Vec<_> = requests
        .iter()
        .zip(&first)
        .map(|((id, _), plan)| {
            json!({"case":id,"outcome":plan.outcome,"probes":plan.probes,"depth":plan.depth,"moved":plan.moved,"blocker":plan.blocker})
        })
        .collect();
    Ok(json!({"schema":"pcbcraft-shove-performance/1","environment":environment,
        "target":if cfg!(target_arch="wasm32"){"browser-shove-cpu"}else{"native-shove-cpu"},
        "debug_assertions":cfg!(debug_assertions),
        "artifact_sha256":digest(artifact_json.as_bytes()),"project_sha256":artifact.project_sha256,"recipe":artifact.recipe,
        "iterations":iterations,"budget":DEFAULT_BUDGET,"outcomes":outcomes,
        "metrics":samples.into_iter().map(|(id,s)|Metric::new(&id,s)).collect::<Vec<_>>(),
        "correctness":"identical plans on every iteration; first-iteration reached plans committed through pcb.shove_tracks with no exhaustive clearance or short decision about the changed copper",
        "unmeasured":["editor preview and commit latency","physical input-to-visible latency","compositor presentation","browser runtime"]}))
}

/// Commit a reached plan on a fresh session and require the exhaustive pair reference to find
/// no clearance violation or short about the changed copper. Other plans move nothing and pass.
fn verify(project: &std::sync::Arc<pcbcraft_engine::Project>, board: BoardTarget, request: &Request, plan: &Shove) -> Result<()> {
    if matches!(plan.outcome, Outcome::Stale | Outcome::Canceled) {
        return Err("a current, uncanceled plan was not produced".into());
    }
    if plan.probes > DEFAULT_BUDGET {
        return Err("plan exceeded its probe budget".into());
    }
    let Shover::Route { points, width_nm } = &request.shover else { return Err("shove diagnostics plan routes".into()) };
    if plan.outcome != Outcome::Reached {
        return Ok(());
    }
    let mut session = Session::default();
    session.open_snapshot(project.clone());
    let before = project.board_view(board).ok_or("missing board")?;
    let (tracks, vias) = (before.tracks(), before.vias());
    fixture::apply(&mut session, SHOVE_TRACKS, json!({"board":board.board,"layer":request.layer,"width_nm":width_nm,"points":points}))?;
    let shoved = session.project().and_then(|project| project.board_view(board)).ok_or("missing shoved board")?;
    let changed: BTreeSet<String> = shoved
        .tracks()
        .iter()
        .filter(|t| !tracks.contains(t))
        .map(|t| t.id.to_string())
        .chain(shoved.vias().iter().filter(|v| !vias.contains(v)).map(|v| v.id.to_string()))
        .collect();
    let query = session.clearance_query_with_rules(board, &json!({})).map_err(|e| e.to_string())?;
    let objects = query.objects();
    let offending = query.exhaustive_pairs().decisions.into_iter().any(|decision| {
        (decision.clearance_violation || decision.conflicting_nets)
            && (decision.first.is_some_and(|index| changed.contains(&objects[index].name)) || changed.contains(&objects[decision.second].name))
    });
    if offending {
        return Err("committed shove breaks clearance in the exhaustive reference".into());
    }
    Ok(())
}
