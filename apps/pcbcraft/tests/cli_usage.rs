//! Run the actual launcher with captured handles, including the Windows GUI executable.
//! Invalid usage must finish before native-window initialization on every desktop platform.
#![allow(clippy::unwrap_used)]

use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Piped stderr remains observable even for Windows' release GUI subsystem. Keep the
/// test bounded if a regression starts the event loop instead of rejecting the arguments.
#[test]
fn usage_errors_exit_before_opening_a_window() {
    for (args, expected) in [
        (vec!["--help"], "PCBCraft has no options · usage: pcbcraft [PROJECT]"),
        (vec!["first project.pc", "second.pc"], "PCBCraft opens one project · usage: pcbcraft [PROJECT]"),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pcbcraft"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                assert!(Instant::now() < deadline, "usage error did not exit before window initialization");
            }
            thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap().trim_end(), expected);
    }
}
