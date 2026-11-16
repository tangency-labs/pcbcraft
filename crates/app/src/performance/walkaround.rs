//! Walkaround planning service costs for #246, separate from editor input and presentation.
use super::*;
use pcbcraft_engine::board::{
    route::{DEFAULT_BUDGET, Outcome, Plan, Posture, Request},
    setup::{LayerPurpose, Side},
    track::ADD_TRACKS,
};

/// Airwires planned per iteration, in the ratsnest's deterministic order.
const AIRWIRES: usize = 6;

/// Plan fixed requests over one captured query per iteration: the fixture's first airwires
/// pad to pad, one free-space route along the channel recipe's corridor and one across the
/// crowded diagonal region, all on front copper at the board's preferred or class width. Each
/// complete plan is timed. Outside the timed boundary, every iteration must reproduce the
/// first iteration's plans exactly, and on the first iteration each plan's points are committed
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
    let front = view
        .layers()
        .into_iter()
        .find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(Side::Front))
        .ok_or("fixture requires front copper")?
        .id;
    let mut cases: Vec<(String, Point, Point)> = view
        .ratsnest()
        .map(|ratsnest| ratsnest.lines.into_iter().take(AIRWIRES).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
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
        requests.push((id, Request { head, target, layer: front, width_nm, posture: Posture::Diagonal, flipped: false, previous: Vec::new() }));
    }
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
        "correctness":"identical plans on every iteration; first-iteration plans committed through pcb.add_tracks with no exhaustive clearance or short decision about the new copper",
        "unmeasured":["editor preview and commit latency","physical input-to-visible latency","compositor presentation","browser runtime"]}))
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
