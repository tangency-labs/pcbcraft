//! `pcbcraft-cli --help` and `-h` report usage on stdout and succeed, while unrecognized
//! arguments keep usage on stderr with status 2. These run the built executable so the
//! observable stream and exit status are what a terminal would see.
#![allow(clippy::unwrap_used)]

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pcbcraft-cli")).args(args).output().unwrap()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

/// Both help aliases print the usage they are listed in, to stdout, with a success status
/// and nothing on stderr. It states the oldest readable split entry point. Returning
/// immediately means help starts no server, opens no project and waits on no input.
#[test]
fn help_aliases_print_usage_to_stdout_and_succeed() {
    for flag in ["--help", "-h"] {
        let output = run(&[flag]);
        assert!(output.status.success(), "{flag}: {:?}", output.status);
        assert_eq!(output.status.code(), Some(0), "{flag}");
        let stdout = text(&output.stdout);
        assert!(stdout.starts_with("Usage: pcbcraft-cli"), "{flag}: {stdout}");
        assert!(stdout.contains("--help") && stdout.contains("-h"), "{flag}: {stdout}");
        assert!(
            stdout.trim_end().ends_with(
                "The oldest split project entry-point format this build opens is pcbcraft-project/5, in project folders and .pc packages."
            ),
            "{flag}: {stdout}"
        );
        assert!(output.stderr.is_empty(), "{flag}: {}", text(&output.stderr));
    }
}

/// Unknown arguments stay an error: usage on stderr, nothing on stdout, status 2.
#[test]
fn unknown_arguments_keep_usage_on_stderr_and_status_two() {
    for args in [&["nonsense"][..], &["mcp", "--bogus"][..], &["--help", "extra"][..], &["convert", "only-source"][..]] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(text(&output.stderr).starts_with("Usage: pcbcraft-cli"), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {}", text(&output.stdout));
    }
}

/// The help text an alias prints is the same usage an error reports.
#[test]
fn help_matches_the_error_usage() {
    let help = run(&["--help"]);
    let error = run(&["nonsense"]);
    assert_eq!(help.stdout, error.stderr);
}
