//! PCBCraft on the desktop.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{ffi::OsString, path::PathBuf};

use pcbcraft_app::PcbApp;

mod startup_console;

/// The short one-line usage errors this launcher prints to stderr before any window exists.
const NO_OPTIONS: &str = "PCBCraft has no options · usage: pcbcraft [PROJECT]";
const ONE_PROJECT: &str = "PCBCraft opens one project · usage: pcbcraft [PROJECT]";

/// Decide what the desktop command line asks for: `None` keeps the unchanged startup at Home,
/// and one positional path asks to open that project folder, `project.json` or `.pc` archive
/// at startup. The desktop app has no options, so any argument starting with `-` is refused,
/// as is a second positional argument; both print one usage line and exit non-zero. The `-`
/// test reads the lossy spelling, which keeps a leading `-` for every real path; an unreadable
/// path is not a usage error — it opens through the app and reports like any other failed open.
fn startup_project<I: IntoIterator<Item = OsString>>(args: I) -> Result<Option<PathBuf>, &'static str> {
    let args: Vec<OsString> = args.into_iter().skip(1).collect();
    match args.as_slice() {
        [] => Ok(None),
        [arg] if arg.to_string_lossy().starts_with('-') => Err(NO_OPTIONS),
        [arg] => Ok(Some(PathBuf::from(arg))),
        [_, ..] => Err(ONE_PROJECT),
    }
}

fn main() -> eframe::Result {
    // Keep the installed stderr handle alive until the native event loop has stopped.
    let _startup_console = startup_console::attach();
    // Usage errors exit before the window is created, so a mistyped command line never opens one.
    let mut startup_project = match startup_project(std::env::args_os()) {
        Ok(project) => project,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    let integrated_titlebar = cfg!(target_os = "linux");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(pcbcraft_app::APP_TITLE)
            .with_decorations(!integrated_titlebar)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PCBCraft",
        options,
        Box::new(move |cc| {
            let mut app = PcbApp::new(cc).with_integrated_titlebar(integrated_titlebar);
            if let Some(path) = startup_project.take() {
                app = app.with_startup_project(path);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    /// No argument keeps the unchanged startup at Home.
    #[test]
    fn no_argument_keeps_the_home_startup() {
        assert_eq!(startup_project(args(&["pcbcraft"])), Ok(None));
    }

    /// One project path asks for the startup open.
    #[test]
    fn one_path_requests_the_startup_open() {
        assert_eq!(startup_project(args(&["pcbcraft", "boards/uc2/project.json"])), Ok(Some(PathBuf::from("boards/uc2/project.json"))));
    }

    /// Windows separators, drive-relative paths, UNC paths, spaces and Unicode stay one
    /// positional argument. Parsing must not reinterpret a drive or slash as an option.
    #[test]
    fn native_path_spellings_are_preserved() {
        for path in [
            r"C:\boards\my board\project.json",
            r"C:board.pc",
            r"\boards\board.pc",
            r"\\server\share\board.pc",
            r"\\?\C:\boards\board.pc",
            "boards/電路 α.pc",
        ] {
            assert_eq!(startup_project(args(&["pcbcraft", path])), Ok(Some(PathBuf::from(path))));
        }
    }

    /// Native Windows arguments can contain unpaired UTF-16 code units; option detection
    /// may use lossy text, but the accepted path must retain the original OS string.
    #[cfg(windows)]
    #[test]
    fn windows_paths_preserve_unpaired_utf16() {
        use std::os::windows::ffi::OsStringExt;
        let path = OsString::from_wide(&[b'C' as u16, b':' as u16, b'\\' as u16, 0xd800, b'.' as u16, b'p' as u16, b'c' as u16]);
        assert_eq!(startup_project([OsString::from("pcbcraft"), path.clone()]), Ok(Some(PathBuf::from(path))));
    }

    /// A second positional argument is refused with one usage line.
    #[test]
    fn a_second_path_is_a_usage_error() {
        let error = startup_project(args(&["pcbcraft", "boards/uc2", "extra"])).err();
        assert!(error.is_some_and(|error| error.starts_with("PCBCraft opens one project")));
    }

    /// The desktop app has no options, so any flag-like argument is refused, including a lone `-`.
    #[test]
    fn options_are_usage_errors() {
        for option in ["--help", "-h", "-"] {
            let error = startup_project(args(&["pcbcraft", option])).err();
            assert!(error.is_some_and(|error| error.starts_with("PCBCraft has no options")), "option {option:?}");
        }
    }
}
