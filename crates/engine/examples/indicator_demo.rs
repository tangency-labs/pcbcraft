//! Build a complete original two-transistor flasher through the same commands as the GUI and MCP.

use std::{error::Error, fs, path::PathBuf};

use pcbcraft_engine::{
    Session,
    persistence::{encode_documents, encode_packed},
};

#[path = "indicator_demo/board.rs"]
mod board;
#[path = "indicator_demo/routing.rs"]
mod routing;
#[path = "indicator_demo/schematic.rs"]
mod schematic;

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
    let connectivity = project.electrical_connectivity();
    if connectivity.nets.len() != 8 || !connectivity.unconnected.is_empty() {
        return Err("demo circuit has unexpected or unconnected nets".into());
    }
    let target = project.board_target().ok_or("demo has no board target")?;
    let ratsnest = project.board_view(target).and_then(|board| board.ratsnest()).ok_or("demo has no accepted board connectivity")?;
    if ratsnest.terminal_count != 30 || !ratsnest.missing.is_empty() || !ratsnest.lines.is_empty() {
        return Err(format!("demo is not completely routed: {} airwires, {} missing terminals", ratsnest.lines.len(), ratsnest.missing.len()).into());
    }
    let report = session.check_design(&serde_json::json!({}))?;
    if report["total"].as_u64() != Some(0) {
        return Err(format!("demo board failed its design check: {report}").into());
    }
    let documents = encode_documents(project.clone())?;
    let archive = encode_packed(project)?;
    fs::create_dir(&destination).map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    let publish = || -> Result<(), Box<dyn Error>> {
        for (relative, bytes) in documents {
            let path = destination.join("dual-led").join(relative);
            fs::create_dir_all(path.parent().ok_or("document has no parent")?)?;
            fs::write(path, bytes)?;
        }
        fs::write(destination.join("dual-led.pc"), archive)?;
        Ok(())
    };
    publish().map_err(|error| format!("incomplete output at {}: {error}", destination.display()))?;
    println!("Open {}", destination.join("dual-led/project.json").display());
    Ok(())
}

/// Compose the schematic and board through their shared, revision-checked command paths.
fn build_demo() -> Result<Session, Box<dyn Error>> {
    let mut demo = schematic::build()?;
    board::build(&mut demo)?;
    routing::build(&mut demo)?;
    Ok(demo.session)
}
