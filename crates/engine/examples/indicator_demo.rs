//! Build the original indicator editing demo through the same commands as the GUI and MCP.

use std::{error::Error, fs, path::PathBuf};

use pcbcraft_engine::{
    DRAW_WIRE, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_SYMBOL_FIELDS, Session,
    persistence::{encode_documents, encode_packed},
    schematic::GRID_NM,
};
use serde_json::{Value, json};

/// Generate a new project identity and publish its folder and archive in a new directory.
/// Existing destinations are refused; encoding finishes before any output is created.
/// A filesystem failure may leave a partial output directory, which is reported to the caller.
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let destination = args.next().map(PathBuf::from).ok_or("usage: indicator_demo NEW_OUTPUT_DIRECTORY")?;
    if args.next().is_some() {
        return Err("usage: indicator_demo NEW_OUTPUT_DIRECTORY".into());
    }
    let session = build_demo()?;
    let project = session.snapshot().ok_or("demo has no project")?;
    let documents = encode_documents(project.clone())?;
    let archive = encode_packed(project)?;
    fs::create_dir(&destination).map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    let publish = || -> Result<(), Box<dyn Error>> {
        for (relative, bytes) in documents {
            let path = destination.join("dual-led").join(relative);
            fs::create_dir_all(path.parent().ok_or("document has no parent")?)?;
            fs::write(path, bytes)?;
        }
        fs::write(destination.join("dual-led.pc.zip"), archive)?;
        Ok(())
    };
    publish().map_err(|error| format!("incomplete output at {}: {error}", destination.display()))?;
    println!("Open {}", destination.join("dual-led/project.pc").display());
    Ok(())
}

/// Arrange two resistor/LED branches and a capacitor between explicit shared rail junctions.
/// All mutations use revision-checked engine commands; no saved documents are edited directly.
fn build_demo() -> Result<Session, Box<dyn Error>> {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Dual LED · editing demo"}))?;
    let input = place(&mut session, "connector", "J1", "Input", 0, -18)?;
    let r1 = place(&mut session, "resistor", "R1", "1k", -7, -6)?;
    let d1 = place(&mut session, "led", "D1", "Green", 7, -6)?;
    let r2 = place(&mut session, "resistor", "R2", "1k", -7, 6)?;
    let d2 = place(&mut session, "led", "D2", "Amber", 7, 6)?;
    let capacitor = place(&mut session, "capacitor", "C1", "100nF", 0, 18)?;

    let left_top = wire(&mut session, pin(&input, "1"), point(-18, -6), &[(-18, -18)])?;
    let right_top = wire(&mut session, pin(&input, "2"), point(18, -6), &[(18, -18)])?;
    wire(&mut session, left_top.clone(), pin(&r1, "1"), &[])?;
    wire(&mut session, pin(&r1, "2"), pin(&d1, "1"), &[])?;
    wire(&mut session, pin(&d1, "2"), right_top.clone(), &[])?;
    let left_bottom = wire(&mut session, left_top, point(-18, 6), &[])?;
    let right_bottom = wire(&mut session, right_top, point(18, 6), &[])?;
    wire(&mut session, left_bottom.clone(), pin(&r2, "1"), &[])?;
    wire(&mut session, pin(&r2, "2"), pin(&d2, "1"), &[])?;
    wire(&mut session, pin(&d2, "2"), right_bottom.clone(), &[])?;
    wire(&mut session, left_bottom, pin(&capacitor, "1"), &[(-18, 18)])?;
    wire(&mut session, pin(&capacitor, "2"), right_bottom, &[(18, 18)])?;

    // These generic two-pad placements exercise identity mapping, not physical package suitability.
    for (symbol, x, y) in [(&input, 0, -8), (&r1, -6, -2), (&d1, 6, -2), (&r2, -6, 4), (&d2, 6, 4), (&capacitor, 0, 10)] {
        session.apply(
            PLACE_FOOTPRINT,
            &json!({"symbol": symbol, "definition": "generic-through-hole-2", "pin_1_pad": 1,
                "x_nm": x * GRID_NM, "y_nm": y * GRID_NM, "expected_revision": session.revision()}),
        )?;
    }
    Ok(session)
}

/// Place a starter symbol, set its displayed fields, and return its inspected stable identity.
fn place(session: &mut Session, definition: &str, reference: &str, value: &str, x: i64, y: i64) -> Result<String, Box<dyn Error>> {
    session
        .apply(PLACE_SYMBOL, &json!({"definition": definition, "x_nm": x * GRID_NM, "y_nm": y * GRID_NM, "expected_revision": session.revision()}))?;
    let symbol = session.project().and_then(|project| project.symbols().last()).ok_or("placed symbol missing")?.id();
    session.apply(SET_SYMBOL_FIELDS, &json!({"symbol": symbol, "reference": reference, "value": value, "expected_revision": session.revision()}))?;
    Ok(symbol)
}

/// Commit an orthogonal route and return its destination identity, including a newly created junction.
fn wire(session: &mut Session, from: Value, to: Value, bends: &[(i64, i64)]) -> Result<Value, Box<dyn Error>> {
    let bends: Vec<_> = bends.iter().map(|&(x, y)| point(x, y)).collect();
    session.apply(DRAW_WIRE, &json!({"from": from, "to": to, "bends": bends, "expected_revision": session.revision()}))?;
    let wire = session.project().and_then(|project| project.wires().last()).ok_or("drawn wire missing")?;
    Ok(wire.endpoints()[1].inspect())
}

/// Represent a coordinate in units of the starter schematic grid.
fn point(x: i64, y: i64) -> Value {
    json!({"x_nm": x * GRID_NM, "y_nm": y * GRID_NM})
}

/// Refer to a pin by identity, rather than relying on coincident drawing coordinates.
fn pin(symbol: &str, terminal: &str) -> Value {
    json!({"symbol": symbol, "terminal": terminal})
}
