//! Shared platform trash adapter for confirmed native folder operations.

use std::{fs, path::Path};

/// Move a folder through the platform trash service with no permanent-delete fallback.
pub(crate) fn directory(root: &Path) -> Result<(), String> {
    let path = if root.is_absolute() { root.to_path_buf() } else { std::env::current_dir().map_err(|e| e.to_string())?.join(root) };
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let mut command = {
        let mut command = std::process::Command::new("gio");
        command.arg("trash").arg("--").arg(&path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("osascript");
        command.args(["-e", "on run argv\ntell application \"Finder\" to delete (POSIX file (item 1 of argv) as alias)\nend run", "--"]).arg(&path);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("powershell.exe");
        command
        .args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($env:PCBCRAFT_TRASH_PATH, 'OnlyErrorDialogs', 'SendToRecycleBin')"])
        .env("PCBCRAFT_TRASH_PATH", &path);
        command
    };
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos", target_os = "windows")))]
    return Err(format!("System trash is unavailable for {}", path.display()));
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos", target_os = "windows"))]
    if wait_for_trash(&mut command, std::time::Duration::from_secs(30))?.success()
        && fs::symlink_metadata(root).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        Ok(())
    } else {
        Err("System trash did not move the folder".into())
    }
}

/// Bound the platform trash subprocess so an unavailable desktop service cannot hold the
/// operation slot indefinitely. Filesystem reads/renames remain native blocking worker I/O.
pub(crate) fn wait_for_trash(command: &mut std::process::Command, timeout: std::time::Duration) -> Result<std::process::ExitStatus, String> {
    let mut child = command.spawn().map_err(|e| format!("Could not start system trash: {e}"))?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(error) => format!("Could not inspect system trash: {error}"),
                    _ => "System trash timed out; inspect the folder before retrying.".into(),
                });
            }
        }
    }
}

#[cfg(all(test, unix))]
mod trash_process_tests {
    /// A stalled trash subprocess is terminated without ever applying a permanent-delete fallback.
    #[test]
    fn stalled_trash_process_times_out() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "exec sleep 5"]);
        assert!(super::wait_for_trash(&mut command, std::time::Duration::from_millis(20)).unwrap_err().contains("timed out"));
    }
}
