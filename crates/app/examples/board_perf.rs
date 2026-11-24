//! Generate artifacts once, then measure the same bytes on each candidate revision.
use pcbcraft_app::performance::{Environment, generate, run};

/// Explicit file arguments keep artifacts and operator-provided environment metadata reviewable.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, artifact, output] if command == "shove_render_fixture" => {
            let derived = pcbcraft_app::performance::shove::rendered_fixture(&std::fs::read_to_string(artifact)?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, derived.as_bytes())?;
        }
        [command, artifact, revision, output] if command == "shove_fixture" => {
            let derived = pcbcraft_app::performance::shove::fixture(&std::fs::read_to_string(artifact)?, revision)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, derived.as_bytes())?;
        }
        [command, artifact, metadata, iterations, output] if command == "shove" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = pcbcraft_app::performance::shove::run(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        }
        [command, artifact, output] if command == "commit_fixture" => {
            let derived = pcbcraft_app::performance::route_commit_artifact(&std::fs::read_to_string(artifact)?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, derived.as_bytes())?;
        }
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
        [command, artifact, metadata, iterations, output] if command == "walkaround" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = pcbcraft_app::performance::walkaround::run(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        }
        [command, artifact, metadata, iterations, output] if command == "walkaround_jobs" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = pcbcraft_app::performance::walkaround::jobs(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        }
        [command, artifact, metadata, iterations, output] if command == "pointer_latency" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = pcbcraft_app::performance::pointer::run(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        }
        [command, artifact, metadata, iterations, output] if command == "checks" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = pcbcraft_app::performance::checker::run(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        }
        [command, artifact, metadata, iterations, output] if command == "budgets" => {
            let environment: Environment = serde_json::from_slice(&std::fs::read(metadata)?)?;
            let report = pcbcraft_app::performance::budget::suite(&std::fs::read_to_string(artifact)?, environment, iterations.parse()?)?;
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
                "usage: board_perf generate RECIPE GENERATOR_REV ARTIFACT | (commit_fixture ARTIFACT OUTPUT | shove_fixture ARTIFACT GENERATOR_REV OUTPUT | shove_render_fixture ARTIFACT OUTPUT) | (run|budgets|checks|routes|walkaround|walkaround_jobs|shove|pointer_latency) ARTIFACT ENVIRONMENT_JSON ITERATIONS REPORT".into()
            );
        }
    }
    Ok(())
}
