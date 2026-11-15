//! Build a complete original two-transistor flasher through the same commands as the GUI and MCP.

use std::{error::Error, path::PathBuf};

use pcbcraft_engine::Session;

mod demo_support;

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
    demo_support::publish(session, destination, "dual-led", 8, 30)
}

/// Compose the schematic and board through their shared, revision-checked command paths.
fn build_demo() -> Result<Session, Box<dyn Error>> {
    let mut demo = schematic::build()?;
    board::build(&mut demo)?;
    routing::build(&mut demo)?;
    Ok(demo.session)
}
