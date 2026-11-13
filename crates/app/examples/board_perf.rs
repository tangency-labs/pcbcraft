//! Generate artifacts once, then measure the same bytes on each candidate revision.
use pcbcraft_app::performance::{Environment, generate, run};

/// Explicit file arguments keep artifacts and operator-provided environment metadata reviewable.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, recipe, revision, path] if command == "generate" => {
            let artifact = generate(recipe, revision)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            std::io::Write::write_all(&mut file, artifact.as_bytes())?;
        }
        [command, artifact, metadata, iterations, output] if command == "routes" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = pcbcraft_app::performance::routing::run(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        }
        [command, artifact, metadata, iterations, output] if command == "checks" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = pcbcraft_app::performance::checker::run(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        }
        [command, artifact, metadata, iterations, output] if command == "run" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = run(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        }
        _ => {
            return Err(
                "usage: board_perf generate RECIPE GENERATOR_REV ARTIFACT | (run|checks|routes) ARTIFACT ENVIRONMENT_JSON ITERATIONS REPORT".into()
            );
        }
    }
    Ok(())
}
