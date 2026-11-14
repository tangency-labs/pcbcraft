//! Read-only navigation diagnosis of an existing native project.
use std::{fs::OpenOptions, io::Write, path::Path};

/// Explicit paths and create-new output preserve the source project and prior samples.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 && !(args.len() == 4 && args[3] == "--zoom-all") {
        return Err("usage: project_perf PROJECT ENVIRONMENT_JSON REPORT [--zoom-all]".into());
    }
    let project = pcbcraft_storage::preview(Path::new(&args[0]))?;
    let environment = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let report = if args.len() == 4 {
        pcbcraft_app::performance::project::zoom_all(project, environment)?
    } else {
        pcbcraft_app::performance::project::navigation(project, environment)?
    };
    OpenOptions::new().write(true).create_new(true).open(&args[2])?.write_all(&serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
