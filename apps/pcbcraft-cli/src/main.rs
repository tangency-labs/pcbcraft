//! PCBCraft without a window. `pcbcraft-cli mcp` serves MCP on stdin and stdout.

use std::{ffi::OsString, path::Path, process::ExitCode};

use pcbcraft_mcp::{
    Backend, Headless, Server,
    live::{Remote, TOKEN_ENV, WAIT_ENV},
};

const USAGE: &str =
    "Usage: pcbcraft-cli mcp [--project FILE_OR_FOLDER | --connect PORT]    Serve MCP on stdin and stdout\n       pcbcraft-cli --version";

/// Dispatch the CLI and report file or server errors on stderr.
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [command] if command == "mcp" => serve(Box::new(Headless::default())),
        [command, flag, path] if command == "mcp" && flag == "--project" => {
            load_project(Path::new(path)).and_then(|backend| serve(Box::new(backend)))
        }
        [command, flag, port] if command == "mcp" && flag == "--connect" => connect_editor(port).and_then(|backend| serve(Box::new(backend))),
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
    let result = pcbcraft_storage::open(path).map(Headless::with_project);
    result.map_err(|error| format!("could not open {}: {error}", path.display()))
}

/// Read the explicitly selected port and the credential supplied by the MCP client, and the
/// optional reply wait an approving endpoint asks for. An unreadable wait keeps the default.
fn connect_editor(port: &std::ffi::OsStr) -> Result<Remote, String> {
    let port = port.to_str().and_then(|port| port.parse().ok()).ok_or("--connect requires a local port number")?;
    let token = std::env::var(TOKEN_ENV).map_err(|_| format!("Set {TOKEN_ENV} from the editor's Local MCP configuration"))?;
    let remote = Remote::new(port, &token)?;
    Ok(match std::env::var(WAIT_ENV).ok().and_then(|wait| wait.trim().parse().ok()) {
        Some(seconds) => remote.with_wait(seconds),
        None => remote,
    })
}

/// Serve the selected backend on stdio.
fn serve(backend: Box<dyn Backend>) -> Result<(), String> {
    Server::new(backend).serve(std::io::stdin().lock(), std::io::stdout().lock()).map_err(|error| error.to_string())
}
