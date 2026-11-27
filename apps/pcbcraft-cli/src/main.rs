//! PCBCraft without a window. `pcbcraft-cli mcp` serves MCP on stdin and stdout.

use std::{ffi::OsString, path::Path, process::ExitCode};

use pcbcraft_engine::{SessionScope, persistence::OLDEST_READABLE_PROJECT_FORMAT};
use pcbcraft_mcp::{
    Backend, Headless, Server,
    live::{Remote, TOKEN_ENV},
};

const USAGE: &str = "Usage: pcbcraft-cli mcp [--read-only] [--project FILE_OR_FOLDER]
       pcbcraft-cli mcp --connect PORT [--token-file FILE] [--wait SECONDS]
                        Serve MCP on stdin and stdout. --read-only applies only to headless sessions.
                        --connect reads the editor's credential from
                        --token-file, or else from PCBCRAFT_CONTROL_TOKEN.
       pcbcraft-cli convert SOURCE DESTINATION
                        Convert a previous split project to a new destination. A .pc destination
                        creates a ZIP package; other names create a new project folder.
       pcbcraft-cli import-kicad SCHEMATIC DESTINATION
                        Import a self-contained KiCad 6-10 schematic (.kicad_sch) as a new project
                        at DESTINATION, a .pc package or a new folder, and list what was left out.
       pcbcraft-cli --help, -h
                        Print this usage information.
       pcbcraft-cli --version";

/// Include the oldest readable split format wherever usage is printed.
fn usage() -> String {
    format!(
        "{USAGE}\n\nThe oldest split project entry-point format this build opens is {OLDEST_READABLE_PROJECT_FORMAT}, in project folders and .pc packages."
    )
}

/// Dispatch the CLI and report file or server errors on stderr.
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [command, source, destination] if command == "convert" => {
            let destination = Path::new(destination);
            let encoding = pcbcraft_storage::conversion::encoding_for(destination);
            pcbcraft_storage::conversion::convert(Path::new(source), destination, encoding).map(|()| {
                println!("Converted project written to {}", destination.display());
            })
        }
        [command, source, destination] if command == "import-kicad" => import_kicad(Path::new(source), Path::new(destination)),
        [command, flag, port, options @ ..] if command == "mcp" && flag == "--connect" => match connect_options(options) {
            Some((token_file, wait)) => connect_editor(port, token_file, wait).and_then(|backend| serve(Box::new(backend))),
            None => Err(format!("unrecognized --connect options\n{}", usage())),
        },
        [command, options @ ..] if command == "mcp" => {
            let Some((scope, path)) = headless_options(options) else {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            };
            let project = path.map(|path| load_project(path, scope)).transpose();
            project.and_then(|project| serve(Box::new(Headless::with_scope(scope, project))))
        }
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{}", usage());
            return ExitCode::SUCCESS;
        }
        [flag] if flag == "--version" => {
            println!("pcbcraft-cli {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{}", usage());
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

/// Import a KiCad schematic file through the shared engine command and write the result as a
/// new project; what the import left out is printed. Nothing is written when the file is
/// refused, too large or the destination is taken.
fn import_kicad(source: &Path, destination: &Path) -> Result<(), String> {
    use std::io::Read;
    let failed = |e: std::io::Error| format!("{}: {e}", source.display());
    if !std::fs::metadata(source).map_err(failed)?.is_file() {
        return Err(format!("{} is not a regular file", source.display()));
    }
    // Read one byte past the limit, so a file that grows or lies about its size is still bounded.
    let limit = pcbcraft_engine::kicad_import::MAX_TEXT_BYTES;
    let mut bytes = Vec::new();
    std::fs::File::open(source).map_err(failed)?.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(failed)?;
    if bytes.len() > limit {
        return Err(format!("{} is larger than the {} MiB a KiCad schematic import accepts", source.display(), limit >> 20));
    }
    let text = String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8 text", source.display()))?;
    let name = source.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".to_owned());
    let mut session = pcbcraft_engine::Session::default();
    let result =
        session.execute(pcbcraft_engine::kicad_import::IMPORT, &serde_json::json!({"name": name, "text": text})).map_err(|e| e.to_string())?;
    let project = session.snapshot().ok_or("the import produced no project")?;
    pcbcraft_storage::conversion::create(project, destination, pcbcraft_storage::conversion::encoding_for(destination))?;
    println!("Imported project written to {}", destination.display());
    for note in result["report"].as_array().into_iter().flatten() {
        println!("  left out or changed: {} ({}×)", note["what"].as_str().unwrap_or_default(), note["count"]);
    }
    Ok(())
}

/// Read headless options in either order, refusing duplicates and all live-only options.
fn headless_options(mut options: &[OsString]) -> Option<(SessionScope, Option<&Path>)> {
    let (mut scope, mut path) = (SessionScope::Edit, None);
    while let Some((flag, rest)) = options.split_first() {
        match (flag, rest) {
            (flag, _) if flag == "--read-only" && scope == SessionScope::Edit => {
                scope = SessionScope::ReadOnly;
                options = rest;
            }
            (flag, [project, tail @ ..]) if flag == "--project" && path.is_none() => {
                path = Some(Path::new(project));
                options = tail;
            }
            _ => return None,
        }
    }
    Some((scope, path))
}

/// Validate a bounded native project before serving it. Read-only startup must not recover an
/// interrupted folder save or create a writer lock; ask the owner to open such a folder first.
fn load_project(path: &Path, scope: SessionScope) -> Result<std::sync::Arc<pcbcraft_engine::Project>, String> {
    let project = match scope {
        SessionScope::ReadOnly => pcbcraft_storage::preview(path),
        SessionScope::Edit => pcbcraft_storage::open(path),
    };
    project.map_err(|error| format!("could not open {}: {error}", path.display()))
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

    /// A headless scope and project may be given in either order but not repeated or mixed
    /// with live options. The default headless scope remains editable.
    #[test]
    fn headless_options_keep_read_only_separate_from_live_connections() {
        assert_eq!(headless_options(&[]), Some((SessionScope::Edit, None)));
        for flags in [["--read-only", "--project", "example.pc"], ["--project", "example.pc", "--read-only"]] {
            assert_eq!(headless_options(&options(&flags)), Some((SessionScope::ReadOnly, Some(Path::new("example.pc")))));
        }
        assert_eq!(headless_options(&options(&["--project", "--review.pc"])), Some((SessionScope::Edit, Some(Path::new("--review.pc")))));
        for bad in [&["--read-only", "--read-only"][..], &["--project"], &["--project", "x", "--project", "y"], &["--read-only", "--connect", "10"]] {
            assert_eq!(headless_options(&options(bad)), None, "{bad:?}");
        }
        assert_eq!(connect_options(&options(&["--read-only"])), None);
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
