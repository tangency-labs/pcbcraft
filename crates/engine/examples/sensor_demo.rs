//! Generate the primary MCU/I²C demonstration with native commands and ordinary codecs.
use std::{error::Error, path::PathBuf};
#[path = "sensor_demo/board.rs"]
mod board;
mod demo_support;
#[path = "sensor_demo/library.rs"]
mod library;
#[path = "sensor_demo/routing.rs"]
mod routing;
#[path = "sensor_demo/schematic.rs"]
mod schematic;

/// Refuse existing destinations and publish only a connected, checked and completely routed demo.
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let destination = args.next().map(PathBuf::from).ok_or("usage: sensor_demo NEW_OUTPUT_DIRECTORY")?;
    if args.next().is_some() {
        return Err("usage: sensor_demo NEW_OUTPUT_DIRECTORY".into());
    }
    let mut demo = schematic::build()?;
    board::build(&mut demo)?;
    routing::build(&mut demo)?;
    demo_support::publish(demo.session, destination, "sensor-node", 10, 43)
}
