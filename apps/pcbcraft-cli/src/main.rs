//! PCBCraft without a window. `pcbcraft-cli mcp` serves MCP on stdin and stdout.

use std::{ffi::OsString, path::Path, process::ExitCode};

use pcbcraft_mcp::{
    Backend, Headless, Server,
    live::{Remote, TOKEN_ENV},
};

const USAGE: &str = "Usage: pcbcraft-cli mcp [--project FILE_OR_FOLDER | --connect PORT [--token-file FILE] [--wait SECONDS]]
                        Serve MCP on stdin and stdout. --connect reads the editor's credential from
                        --token-file, or else from PCBCRAFT_CONTROL_TOKEN.
       pcbcraft-cli convert SOURCE DESTINATION
                        Convert a previous split project to a new destination. A .pc destination
                        creates a ZIP package; other names create a new project folder.
       pcbcraft-cli --version";

/// Dispatch the CLI and report file or server errors on stderr.
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [command, source, destination] if command == "convert" => {
            let destination = Path::new(destination);
            let encoding = if destination.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pc")) {
                pcbcraft_engine::persistence::Encoding::Zip
            } else {
                pcbcraft_engine::persistence::Encoding::Json
            };
            pcbcraft_storage::conversion::convert(Path::new(source), destination, encoding).map(|()| {
                println!("Converted project written to {}", destination.display());
            })
        }
        [command] if command == "mcp" => serve(Box::new(Headless::default())),
        [command, flag, path] if command == "mcp" && flag == "--project" => {
            load_project(Path::new(path)).and_then(|backend| serve(Box::new(backend)))
        }
        [command, flag, port, options @ ..] if command == "mcp" && flag == "--connect" => match connect_options(options) {
            Some((token_file, wait)) => connect_editor(port, token_file, wait).and_then(|backend| serve(Box::new(backend))),
            None => Err(format!("unrecognized --connect options\n{USAGE}")),
        },
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

/// `--connect`'s optional `--token-file FILE` and `--wait SECONDS`, each at most once; `None`
/// for anything else.
fn connect_options(options: &[OsString]) -> Option<(Option<&Path>, Option<u64>)> {
    let (mut token_file, mut wait) = (None, None);
    for pair in options.chunks(2) {
        match pair {
            [flag, path] if flag == "--token-file" && token_file.is_none() => token_file = Some(Path::new(path)),
            [flag, seconds] if flag == "--wait" && wait.is_none() => wait = Some(seconds.to_str()?.trim().parse().ok()?),
            _ => return None,
        }
    }
    Some((token_file, wait))
}

/// Read the explicitly selected port and the credential: from the token file the editor wrote
/// (its in-app chat), or else from the environment the person's MCP client sets. `wait`: how
/// long a reply may take, for an endpoint that holds requests for approval.
fn connect_editor(port: &std::ffi::OsStr, token_file: Option<&Path>, wait: Option<u64>) -> Result<Remote, String> {
    let port = port.to_str().and_then(|port| port.parse().ok()).ok_or("--connect requires a local port number")?;
    let remote = match token_file {
        Some(path) => Remote::from_token_file(port, path)?,
        None => {
            let token = std::env::var(TOKEN_ENV).map_err(|_| format!("Set {TOKEN_ENV} from the editor's Local MCP configuration"))?;
            Remote::new(port, &token)?
        }
    };
    Ok(match wait {
        Some(seconds) => remote.with_wait(seconds),
        None => remote,
    })
}

/// Serve the selected backend on stdio.
fn serve(backend: Box<dyn Backend>) -> Result<(), String> {
    Server::new(backend).serve(std::io::stdin().lock(), std::io::stdout().lock()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    /// Connect takes a token file and a wait once each.
    #[test]
    fn connect_takes_a_token_file_and_a_wait_once_each() {
        assert_eq!(connect_options(&[]), Some((None, None)));
        let given = options(&["--token-file", "/run/a.token", "--wait", "600"]);
        assert_eq!(connect_options(&given), Some((Some(Path::new("/run/a.token")), Some(600))));
        let reversed = options(&["--wait", "30", "--token-file", "b"]);
        assert_eq!(connect_options(&reversed), Some((Some(Path::new("b")), Some(30))));
        for bad in [&["--token-file"][..], &["--wait", "soon"], &["--wait", "1", "--wait", "2"], &["--token", "abc"], &["--token-file", "a", "extra"]]
        {
            assert_eq!(connect_options(&options(bad)), None, "{bad:?}");
        }
    }
}
