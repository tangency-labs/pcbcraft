//! Generate a zone-fill demo through production commands, without editing native documents.
use pcbcraft_engine::{
    ADD_DRAWING, CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, Session,
    board::{
        setup::{LayerPurpose, Side},
        update::INITIALIZE_BASELINE,
        zones::{ADD_ZONE, Options},
    },
    persistence::{encode_documents, encode_packed},
    schematic::Point,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

/// Apply an ordinary revision-checked command and propagate a concrete generation failure.
fn edit(session: &mut Session, command: &str, mut params: Value) -> Result<(), Box<dyn std::error::Error>> {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params)?;
    Ok(())
}

/// Write a new demo folder, packed project and ephemeral browser-worker request; refuse to
/// overwrite an existing destination so generated evidence remains reproducible.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination = std::env::args_os().nth(1).map(PathBuf::from).ok_or("usage: zone_fill_fixture NEW_OUTPUT_DIRECTORY")?;
    fs::create_dir(&destination)?;
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Zone fill demo", "create_board":true}))?;
    let mut symbols = Vec::new();
    for index in 0..3 {
        edit(&mut session, PLACE_SYMBOL, json!({"definition":"resistor", "x_nm":index*10_000_000,"y_nm":0}))?;
        let symbol = session.project().ok_or("missing project")?.symbols().last().ok_or("missing symbol")?.id();
        edit(
            &mut session,
            PLACE_FOOTPRINT,
            json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":index*8_000_000,"y_nm":2_000_000}),
        )?;
        symbols.push(symbol);
    }
    for pair in symbols.windows(2) {
        edit(&mut session, CONNECT_PINS, json!({"from":{"symbol":pair[0],"terminal":"1"},"to":{"symbol":pair[1],"terminal":"1"}}))?;
    }
    edit(&mut session, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true}))?;
    edit(&mut session, INITIALIZE_BASELINE, json!({}))?;
    let project = session.snapshot().ok_or("missing snapshot")?;
    let board = project.board_target().ok_or("missing board")?;
    let view = project.board_view(board).ok_or("missing view")?;
    let layers = view.layers();
    let front = layers.iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Front)).ok_or("missing front layer")?.id;
    let outline = layers.iter().find(|l| l.purpose == LayerPurpose::Outline).ok_or("missing outline layer")?.id;
    let endpoint = &view.ratsnest()?.lines[0].from;
    let address = endpoint.pad.inspect();
    let target = view
        .net_class_target_for_copper(&format!("{}:{}", endpoint.footprint, address["pad"].as_str().ok_or("missing pad")?))?
        .ok_or("missing net target")?;
    let boundary = [[-4_000_000, -4_000_000], [24_000_000, -4_000_000], [24_000_000, 8_000_000], [-4_000_000, 8_000_000]]
        .map(|[x_nm, y_nm]| Point { x_nm, y_nm });
    edit(&mut session, ADD_DRAWING, json!({"layer":outline,"points":boundary,"closed":true}))?;
    let polygon = [[-3_000_000, -3_000_000], [23_000_000, -3_000_000], [23_000_000, 7_000_000], [-3_000_000, 7_000_000]]
        .map(|[x_nm, y_nm]| Point { x_nm, y_nm });
    edit(&mut session, ADD_ZONE, json!({"target":target,"layer":front,"points":polygon,"options":Options::default()}))?;
    let snapshot = session.snapshot().ok_or("missing final snapshot")?;
    for (path, bytes) in encode_documents(snapshot.clone())? {
        let path = destination.join("project").join(path);
        fs::create_dir_all(path.parent().ok_or("missing parent")?)?;
        fs::write(path, bytes)?;
    }
    fs::write(destination.join("zone-fill.pc"), encode_packed(snapshot)?)?;
    let mut job = session.prepare_zone_fill(board, false)?;
    fs::write(destination.join("worker-request.json"), job.worker_request()?)?;
    Ok(())
}
