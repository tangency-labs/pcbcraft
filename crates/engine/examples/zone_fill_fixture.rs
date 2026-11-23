//! Generate a zone-fill demo through production commands, without editing native documents.
use pcbcraft_engine::{
    ADD_DRAWING, CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, Session,
    board::{
        setup::{LayerPurpose, Side},
        update::INITIALIZE_BASELINE,
        zones::{ADD_ZONE, Connection, Options},
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
    let mut args = std::env::args().skip(1);
    let destination = args.next().map(PathBuf::from).ok_or("usage: zone_fill_fixture NEW_OUTPUT_DIRECTORY [FOOTPRINTS [solid|thermal]]")?;
    let count: i64 = args.next().as_deref().unwrap_or("3").parse()?;
    if !(3..=1024).contains(&count) {
        return Err("the fixture requires 3–1024 footprints".into());
    }
    let connection = match args.next().as_deref() {
        Some("solid") => Connection::Solid,
        Some("thermal") | None => Connection::Thermal,
        _ => return Err("expected solid or thermal".into()),
    };
    if args.next().is_some() {
        return Err("unexpected fixture argument".into());
    }
    fs::create_dir(&destination)?;
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Zone fill demo", "create_board":true}))?;
    let mut symbols = Vec::new();
    let columns = count.min(32);
    let pitch = if count == 3 { 8_000_000 } else { 3_000_000 };
    for index in 0..count {
        let (column, row) = (index % columns, index / columns);
        edit(&mut session, PLACE_SYMBOL, json!({"definition":"resistor", "x_nm":column*10_000_000,"y_nm":row*12_000_000}))?;
        let symbol = session.project().ok_or("missing project")?.symbols().last().ok_or("missing symbol")?.id();
        edit(
            &mut session,
            PLACE_FOOTPRINT,
            json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":column*pitch,"y_nm":row*pitch+2_000_000}),
        )?;
        symbols.push(symbol);
    }
    let ground: Vec<_> = symbols.iter().step_by(if count <= 4 { 1 } else { 4 }).collect();
    for pair in ground.windows(2) {
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
    let ratsnest = view.ratsnest()?;
    let endpoint = &ratsnest.lines.first().ok_or("missing connection in the fixture's plane net")?.from;
    let address = endpoint.pad.inspect();
    let target = view
        .net_class_target_for_copper(&format!("{}:{}", endpoint.footprint, address["pad"].as_str().ok_or("missing pad")?))?
        .ok_or("missing net target")?;
    let east = (columns - 1) * pitch + 8_000_000;
    let south = ((count - 1) / columns) * pitch + 8_000_000;
    let rectangle = |inset| {
        let first = -4_000_000 + inset;
        let (last_x, last_y) = (east - inset, south - inset);
        [[first, first], [last_x, first], [last_x, last_y], [first, last_y]].map(|[x_nm, y_nm]| Point { x_nm, y_nm })
    };
    edit(&mut session, ADD_DRAWING, json!({"layer":outline,"points":rectangle(0),"closed":true}))?;
    edit(
        &mut session,
        ADD_ZONE,
        json!({"target":target,"layer":front,"points":rectangle(1_000_000),"options":Options{connection,..Options::default()}}),
    )?;
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
