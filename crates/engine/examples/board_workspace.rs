//! Generate an original inspection example through shared commands and production encoders.
use pcbcraft_engine::{
    Session,
    persistence::{encode_documents, encode_packed},
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

/// Write a resistor/connector example to a new folder and portable archive. The starter board
/// stores pad centers only; this example does not claim an outline, routing or fabrication geometry.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination = std::env::args_os().nth(1).map(PathBuf::from).ok_or("usage: board_workspace NEW_OUTPUT_DIRECTORY")?;
    fs::create_dir(&destination)?;
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Board inspection"}))?;
    for (command, mut params) in [
        (pcbcraft_engine::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2})),
        (pcbcraft_engine::PLACE_SYMBOL, json!({"definition":"resistor","x_nm":-12_700_000,"y_nm":0})),
        (pcbcraft_engine::PLACE_SYMBOL, json!({"definition":"connector","x_nm":12_700_000,"y_nm":0})),
    ] {
        params["expected_revision"] = json!(session.revision());
        session.apply(command, &params)?;
    }
    let symbols: Vec<_> = session.project().ok_or("missing project")?.symbols().iter().map(|symbol| symbol.id()).collect();
    for pin in ["1", "2"] {
        let params =
            json!({"from":{"symbol":symbols[0],"terminal":pin},"to":{"symbol":symbols[1],"terminal":pin},"expected_revision":session.revision()});
        session.apply(pcbcraft_engine::CONNECT_PINS, &params)?;
    }
    for (index, symbol) in symbols.iter().enumerate() {
        let mut params: Value =
            json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":(index as i64)*12_700_000,"y_nm":0});
        params["expected_revision"] = json!(session.revision());
        session.apply(pcbcraft_engine::PLACE_FOOTPRINT, &params)?;
    }
    let snapshot = session.snapshot().ok_or("missing snapshot")?;
    for (path, bytes) in encode_documents(snapshot.clone())? {
        let path = destination.join("board-inspection").join(path);
        fs::create_dir_all(path.parent().ok_or("document has no parent")?)?;
        fs::write(path, bytes)?;
    }
    fs::write(destination.join("board-inspection.pc.zip"), encode_packed(snapshot)?)?;
    Ok(())
}
