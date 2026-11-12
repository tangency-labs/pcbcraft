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
        _ => return Err("recipe must be reference-v1, dense-v1 or capacity-v1".into()),
    };
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true,"name":recipe})).map_err(|e| e.to_string())?;
    let mut symbols = Vec::new();
    for i in 0..components {
        apply(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":(i%32)*12_700_000,"y_nm":(i/32)*12_700_000}))?;
        let symbol = session.project().and_then(|p| p.symbols().last()).ok_or("missing generated symbol")?.id();
        apply(
            &mut session,
            PLACE_FOOTPRINT,
            json!({"symbol":symbol,"definition":if (i+seed)%4==0 {"generic-through-hole-2"} else {"resistor-0402"},"mapping":{"1":"1","2":"2"},"x_nm":(i%32)*2_540_000,"y_nm":(i/32)*2_540_000}),
        )?;
        symbols.push(symbol);
    }
    // Place everything before wiring so subsequent placements cannot auto-heal an earlier
    // chain wire and make an explicit connect command redundant.
    for pair in symbols.windows(2) {
        apply(&mut session, CONNECT_PINS, json!({"from":{"symbol":pair[0],"terminal":"1"},"to":{"symbol":pair[1],"terminal":"1"}}))?;
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
