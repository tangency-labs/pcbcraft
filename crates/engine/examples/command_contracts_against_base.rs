//! Compare two command contract snapshots: `command_contracts_against_base BASE.json HEAD.json`.
//!
//! CI runs it with the target branch's `docs/schemas/command-contracts.json` as BASE and the
//! change's as HEAD, so a breaking change committed together with a rewritten snapshot fails
//! unless the change also raises the command API version. It prints every difference and exits
//! with status 1 on a problem, 2 on unreadable input.

use std::process::ExitCode;

use pcbcraft_engine::command_api::compare_snapshots;
use serde_json::Value;

/// Read and parse one snapshot file.
fn read(path: &str) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))
}

/// Compare the two snapshot files named on the command line.
fn main() -> ExitCode {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    let [base, head] = paths.as_slice() else {
        eprintln!("usage: command_contracts_against_base BASE.json HEAD.json");
        return ExitCode::from(2);
    };
    let (base, head) = match (read(base), read(head)) {
        (Ok(base), Ok(head)) => (base, head),
        (Err(error), _) | (_, Err(error)) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    match compare_snapshots(&base, &head) {
        Ok(changes) => {
            for change in &changes {
                println!("{change}");
            }
            println!("command contracts: {} difference(s), none breaking without a raised API version", changes.len());
            ExitCode::SUCCESS
        }
        Err(problems) => {
            for problem in &problems {
                eprintln!("{problem}");
            }
            ExitCode::from(1)
        }
    }
}
