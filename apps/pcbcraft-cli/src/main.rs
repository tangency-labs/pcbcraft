//! PCBCraft without a window. `pcbcraft-cli mcp` serves MCP on stdin and stdout.

use std::{ffi::OsString, fs::File, path::Path, process::ExitCode};

use pcbcraft_engine::persistence::{decode, read_bytes};
use pcbcraft_mcp::{Headless, Server};

const USAGE: &str = "Usage: pcbcraft-cli mcp [--project FILE]    Serve MCP on stdin and stdout\n       pcbcraft-cli --version";

/// Dispatch the CLI and report file or server errors on stderr.
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [command] if command == "mcp" => serve(Headless::default()),
        [command, flag, path] if command == "mcp" && flag == "--project" => load_project(Path::new(path)).and_then(serve),
        [flag] if flag == "--version" => {
            println!("pcbcraft-cli {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pcbcraft-cli: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Validate a bounded native project before making it available to MCP.
fn load_project(path: &Path) -> Result<Headless, String> {
    let result = (|| {
        let file = File::open(path).map_err(|error| error.to_string())?;
        let bytes = read_bytes(file).map_err(|error| error.to_string())?;
        decode(&bytes).map(Headless::with_project)
    })();
    result.map_err(|error| format!("could not open {}: {error}", path.display()))
}

/// Serve one isolated session on stdio.
fn serve(backend: Headless) -> Result<(), String> {
    Server::new(Box::new(backend)).serve(std::io::stdin().lock(), std::io::stdout().lock()).map_err(|error| error.to_string())
}
