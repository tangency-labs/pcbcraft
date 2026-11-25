//! PCBCraft on the desktop.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{ffi::OsString, path::PathBuf};

use pcbcraft_app::{PcbApp, desktop_window_chrome};

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

/// Swapchain surface settings for the desktop window.
///
/// Windows and Linux keep `LOW_LATENCY` (`desired_maximum_frame_latency = 1`, eframe's own default),
/// so panning, zooming and dragging follow the pointer a frame sooner. On macOS, Metal's `CAMetalLayer`
/// maps `desired_maximum_frame_latency = 1` to `maximumDrawableCount = 2` (double buffering instead of
/// Apple's default three drawables): in full-screen direct-to-display scanout on Apple Silicon, and in
/// windowed mode when GPU work approaches a 120 Hz frame budget, `nextDrawable()` blocks until vblank
/// and ProMotion displays drop to 60 or 40 FPS. `HIGH_THROUGHPUT` (`desired_maximum_frame_latency = 2`,
/// three drawables) keeps Metal aligned with Apple's default and `egui-wgpu`'s iOS rule.
fn surface_config(macos: bool) -> eframe::egui_wgpu::SurfaceConfig {
    if macos { eframe::egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT } else { eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY }
}

/// Configure the desktop window size, app-owned title bar and renderer surface.
fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: desktop_window_chrome(
            eframe::egui::ViewportBuilder::default()
                .with_title(pcbcraft_app::APP_TITLE)
                .with_inner_size([1280.0, 800.0])
                .with_min_inner_size(pcbcraft_app::MAIN_WINDOW_MIN_SIZE),
        ),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration::default().with_surface_config(surface_config(cfg!(target_os = "macos"))),
        ..Default::default()
    }
}

fn main() -> eframe::Result {
    // Usage errors exit before the window is created, so a mistyped command line never opens one.
    let mut startup_project = match startup_project(std::env::args_os()) {
        Ok(project) => project,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    eframe::run_native(
        "PCBCraft",
        native_options(),
        Box::new(move |cc| {
            // Configure the heap-owned app in place: by-value builder chains retain
            // additional full-sized temporaries on Windows' 1 MiB debug startup stack.
            let mut app = Box::new(PcbApp::new(cc));
            app.set_integrated_titlebar(true);
            if let Some(path) = startup_project.take() {
                app.set_startup_project(path);
            }
            Ok(app)
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

#[cfg(test)]
mod surface_tests {
    /// macOS Metal uses three drawables (frame latency 2) so full-screen direct-to-display on
    /// ProMotion displays does not stall `nextDrawable()` down to 60 or 40 FPS; other platforms
    /// keep `LOW_LATENCY` (frame latency 1).
    #[test]
    fn surface_config_uses_three_drawables_on_macos() {
        let macos = super::surface_config(true);
        assert_eq!(macos.desired_maximum_frame_latency, Some(2));
        assert_eq!(macos.present_mode, eframe::wgpu::PresentMode::AutoVsync);

        let other = super::surface_config(false);
        assert_eq!(other.desired_maximum_frame_latency, Some(1));
        assert_eq!(other.present_mode, eframe::wgpu::PresentMode::AutoVsync);
    }

    /// The window the launcher opens uses this platform's [`super::surface_config`].
    #[test]
    fn native_options_use_the_platform_surface_config() {
        let expected = super::surface_config(cfg!(target_os = "macos"));
        assert_eq!(super::native_options().wgpu_options.surface, expected);
    }
}
