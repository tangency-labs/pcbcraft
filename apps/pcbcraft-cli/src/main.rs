//! PCBCraft without a window. `pcbcraft-cli mcp` serves MCP on stdin and stdout.

use std::process::ExitCode;

use pcbcraft_mcp::{Headless, Server};

const USAGE: &str = "Usage: pcbcraft-cli mcp        Serve MCP on stdin and stdout\n       pcbcraft-cli --version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["mcp"] => {
            let stdin = std::io::stdin();
            match Server::new(Box::new(Headless::default())).serve(stdin.lock(), std::io::stdout().lock()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("pcbcraft-cli: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ["--version"] => {
            println!("pcbcraft-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
