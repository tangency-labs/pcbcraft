//! Original, versioned recipes; all native bytes come from production commands and encoders.

use super::{Artifact, Result, digest, replay::Step};
use pcbcraft_engine::{
    CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, Session,
    board::{
        setup::{LayerPurpose, Side},
        track::ADD_TRACKS,
        update::INITIALIZE_BASELINE,
    },
    persistence::encode_packed,
    schematic::Point,
};
use serde_json::{Value, json};

/// Generate supported geometry, retaining random document identities in the saved artifact.
/// A recipe seed fixes geometry, not UUIDs; comparisons must reuse the resulting artifact.
pub(super) fn generate(recipe: &str, generator_revision: &str) -> Result<Artifact> {
    let seed = 237_240;
    let (components, paths, vertices, via_every) = match recipe {
        "reference-v1" => (8, 4, 16, 0),
        "dense-v1" => (128, 16, 64, 8),
        "capacity-v1" => (512, 16, 256, 2),
        "board-limits-v1" => (512, 17, 256, 2),
        "channels-v1" => (64, 24, 16, 4),
        "hierarchy-v1" => (64, 4, 16, 0),
        "schematic-limits-v1" => (4096, 4, 16, 0),
        _ => {
            return Err(
                "recipe must be reference-v1, dense-v1, capacity-v1, board-limits-v1, channels-v1, hierarchy-v1 or schematic-limits-v1".into()
            );
        }
    };
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true,"name":recipe})).map_err(|e| e.to_string())?;
    let root = session.project().ok_or("missing root")?.sheet_target();
    let sheet_count = if matches!(recipe, "hierarchy-v1" | "schematic-limits-v1") { 64 } else { 1 };
    let mut sheets = vec![root];
    for index in 1..sheet_count {
        // Keep depth shallow while reaching the actual sheet-count limit. Each child is
        // linked explicitly; global labels exercise project-wide electrical connectivity.
        apply(
            &mut session,
            pcbcraft_engine::hierarchy::CREATE_CHILD,
            json!({"sheet":root.id(),"name":format!("Child {index}"),"x_nm":(index%8)*20_000_000,"y_nm":(index/8)*15_000_000}),
        )?;
        let target = session.project().and_then(|p| p.sheets().last()).ok_or("missing child")?.target;
        sheets.push(target);
    }
    let mut symbols = Vec::new();
    for i in 0..components {
        let sheet = sheets[(i as usize) % sheets.len()];
        apply(
            &mut session,
            PLACE_SYMBOL,
            json!({"sheet":sheet.id(),"definition":"resistor","x_nm":((i/sheet_count)%32)*12_700_000,"y_nm":((i/sheet_count)/32)*12_700_000}),
        )?;
        let symbol =
            session.project().and_then(|p| p.for_sheet(sheet)).and_then(|p| p.symbols().last().map(|s| s.id())).ok_or("missing generated symbol")?;
        if recipe != "schematic-limits-v1" || i < 8 {
            apply(
                &mut session,
                PLACE_FOOTPRINT,
                json!({"sheet":sheet.id(),"symbol":symbol,"definition":if (i+seed)%4==0 {"generic-through-hole-2"} else {"resistor-0402"},"mapping":{"1":"1","2":"2"},"x_nm":(i%32)*2_540_000,"y_nm":(i/32)*2_540_000}),
            )?;
        }
        if recipe == "schematic-limits-v1" && i >= 8 {
            let projection = session.project().and_then(|p| p.for_sheet(sheet)).ok_or("missing sheet")?;
            let placed = projection.symbols().last().ok_or("missing symbol")?;
            apply(
                &mut session,
                pcbcraft_engine::SET_SYMBOL_FIELDS,
                json!({"sheet":sheet.id(),"symbol":symbol,
                "reference":projection.reference(placed),"value":projection.value(placed),"metadata":{"fields":{},"attributes":{"do_not_populate":false,"exclude_from_board":true,"exclude_from_bom":false,"exclude_from_simulation":false,"exclude_from_position_files":false}}}),
            )?;
        }
        symbols.push((sheet, symbol));
    }
    // Place everything before wiring so subsequent placements cannot auto-heal an earlier
    // chain wire and make an explicit connect command redundant.
    for sheet in &sheets {
        let local: Vec<_> = symbols.iter().filter(|(owner, _)| owner == sheet).map(|(_, symbol)| symbol).collect();
        for pair in local.windows(2) {
            apply(
                &mut session,
                CONNECT_PINS,
                json!({"sheet":sheet.id(),"from":{"symbol":pair[0],"terminal":"1"},"to":{"symbol":pair[1],"terminal":"1"}}),
            )?;
        }
        if sheets.len() > 1 {
            apply(
                &mut session,
                pcbcraft_engine::net_labels::ADD,
                json!({"sheet":sheet.id(),"name":"PERF_SHARED","style":"global","x_nm":0,"y_nm":0}),
            )?;
        }
    }
    apply(&mut session, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true}))?;
    apply(&mut session, INITIALIZE_BASELINE, json!({}))?;
    let project = session.snapshot().ok_or("missing fixture")?;
    let board = project.board_target().ok_or("missing board")?;
    let setup = project.board_view(board).and_then(|b| b.setup()).ok_or("missing setup")?;
    let layers = [Side::Front, Side::Back]
        .map(|side| {
            setup
                .layers()
                .iter()
                .find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(side))
                .map(|layer| layer.id)
                .ok_or_else(|| "fixture requires front and back copper".to_owned())
        })
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    // Tracks occupy a separate region from the intended-net pads. Alternating long diagonal
    // runs and short turns create crowded channels without accidentally shorting intended nets.
    for path in 0..paths {
        // Sixteen full 256-point paths reach 4080 tracks/2032 vias. One final
        // 17-point path, switching at each point, reaches both current limits exactly.
        let final_limit_path = recipe == "board-limits-v1" && path == paths - 1;
        let vertices = if final_limit_path { 17 } else { vertices };
        let via_every = if final_limit_path { 1 } else { via_every };
        let points: Vec<_> = (0..vertices)
            .map(|v| Point { x_nm: 100_000_000 + (v / 2) * 1_270_000, y_nm: 20_000_000 + path * 900_000 + (v % 2) * 20_320_000 })
            .collect();
        let changes: Vec<_> =
            (1..vertices).filter(|v| via_every > 0 && v % via_every == 0).enumerate().map(|(i, v)| json!({"at":v,"layer":layers[(i+1)%2]})).collect();
        apply(
            &mut session,
            ADD_TRACKS,
            json!({"board":board.board,"layer":layers[0],"width_nm":if (path+seed)%3==0 {400_000} else {150_000},"points":points,"layer_changes":changes,"via_diameter_nm":600_000,"via_drill_nm":300_000}),
        )?;
    }
    if recipe == "channels-v1" {
        // Two parallel horizontal rails with attached branches bound alternating open/narrow
        // channels. They use unknown-net copper, never a guessed electrical assignment.
        for side in [0, 1] {
            apply(
                &mut session,
                ADD_TRACKS,
                json!({"board":board.board,"layer":layers[0],"width_nm":250_000,
                "points":[{"x_nm":100_000_000,"y_nm":60_000_000+side*1_500_000},{"x_nm":140_000_000,"y_nm":60_000_000+side*1_500_000}]}),
            )?;
            for branch in 0..24 {
                let x = 101_000_000 + branch * 1_500_000;
                let y = 60_000_000 + side * 1_500_000;
                apply(
                    &mut session,
                    ADD_TRACKS,
                    json!({"board":board.board,"layer":layers[0],"width_nm":250_000,
                    "points":[{"x_nm":x,"y_nm":y},{"x_nm":x,"y_nm":y+if side==0 {-3_000_000} else {3_000_000}}]}),
                )?;
            }
        }
    }
    let project = session.snapshot().ok_or("missing fixture")?;
    let bytes = encode_packed(project)?;
    Ok(Artifact {
        schema: "pcbcraft-performance-fixture/1".into(),
        recipe: recipe.into(),
        generator_revision: generator_revision.into(),
        seed: seed as u64,
        board,
        project_sha256: digest(&bytes),
        project: bytes,
        trace_id: super::replay::STANDARD_TRACE_ID.into(),
        trace: Step::standard(),
    })
}

/// Add optimistic revision checking to every fixture edit through the shared command path.
pub(super) fn apply(session: &mut Session, id: &str, mut params: Value) -> Result<()> {
    params["expected_revision"] = json!(session.revision());
    session.apply(id, &params).map(|_| ()).map_err(|e| format!("{id}: {e}"))
}
