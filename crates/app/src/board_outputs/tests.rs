use super::*;
use std::fs;

/// Build an immutable export capture, returning any archive error to the test.
fn capture(text: &str) -> Result<Archive, String> {
    Archive::new(&[pcbcraft_engine::board::fabrication::Output { name: "board.gbr".into(), contents: text.into() }])
}

/// Both artifacts are published from the capture, with the exact save-dialog filename in JSON.
/// Replacing them does not leave staging files, even when a previous pair already exists.
#[test]
fn publish_package_binds_the_selected_filename_and_replaces_both_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chosen.zip");
    let sidecar = dir.path().join("chosen.zip.manifest.json");
    for text in ["first\n", "replacement\n"] {
        let archive = capture(text).unwrap();
        publish_package(&path, &archive).unwrap();
        assert_eq!(fs::read(&path).unwrap(), archive.bytes());
        assert_eq!(fs::read(&sidecar).unwrap(), archive.manifest_json("chosen.zip").unwrap());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

/// A sidecar failure is an error even after the ZIP is replaced; a stale sidecar does not
/// describe that ZIP. The failure leaves no staging files, and retrying recovers the pair.
#[test]
fn sidecar_failure_reports_partial_publication_and_stale_digest_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.zip");
    let sidecar = dir.path().join("board.zip.manifest.json");
    let old = capture("old\n").unwrap();
    publish_package(&path, &old).unwrap();
    let old_manifest = fs::read(&sidecar).unwrap();
    // A non-file companion destination reliably fails on every native test platform.
    fs::remove_file(&sidecar).unwrap();
    fs::create_dir(&sidecar).unwrap();
    let replacement = capture("replacement\n").unwrap();
    let error = publish_package(&path, &replacement).unwrap_err();
    assert!(error.contains("ZIP published") && error.contains("manifest failed") && error.contains("not a regular file"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), replacement.bytes());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    fs::remove_dir(&sidecar).unwrap();
    fs::write(&sidecar, &old_manifest).unwrap();
    let stale: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    assert_ne!(stale["archive"]["sha256"], replacement.manifest("board.zip").archive.sha256);
    assert_eq!(stale, serde_json::to_value(old.manifest("board.zip")).unwrap());
    publish_package(&path, &replacement).unwrap();
    assert_eq!(fs::read(&sidecar).unwrap(), replacement.manifest_json("board.zip").unwrap());
}

/// An archive destination failure does not update the sidecar or report a complete pair.
#[test]
fn archive_failure_keeps_the_existing_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.zip");
    let sidecar = dir.path().join("board.zip.manifest.json");
    fs::create_dir(&path).unwrap();
    fs::write(&sidecar, b"old manifest").unwrap();
    let error = publish_package(&path, &capture("new").unwrap()).unwrap_err();
    assert!(error.contains("Sidecar not updated") && error.contains("not a regular file"), "{error}");
    assert_eq!(fs::read(&sidecar).unwrap(), b"old manifest");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

/// An unrepresentable filename is refused before either destination is touched, not recorded
/// with lossy replacement characters that would name a different archive.
#[cfg(unix)]
#[test]
fn non_utf8_archive_name_is_refused_before_publication() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(std::ffi::OsString::from_vec(vec![b'x', 0xff]));
    let error = publish_package(&path, &capture("new").unwrap()).unwrap_err();
    assert!(error.contains("filename must be valid UTF-8"), "{error}");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// Captured tests use the real one-slot completion queue, including cancellation, failure and
/// disconnection. Every completed outcome releases the slot for another export.
#[test]
fn completion_releases_the_export_slot_on_every_outcome() {
    let ctx = egui::Context::default();
    let mut exports = Exports::default();
    for result in [Ok(Outcome::Saved), Ok(Outcome::DownloadsRequested), Ok(Outcome::Cancelled), Err("save failed".into())] {
        exports.start(&ctx, "board.zip".into(), capture("archive").unwrap()).unwrap();
        assert!(exports.busy());
        assert!(exports.poll().is_none());
        assert_eq!(exports.start(&ctx, "overlap.zip".into(), capture("").unwrap()).unwrap_err(), "Finish the current fabrication export first.");
        let saved = matches!(&result, Ok(Outcome::Saved));
        let cancelled = matches!(&result, Ok(Outcome::Cancelled));
        let requested = matches!(&result, Ok(Outcome::DownloadsRequested));
        exports.complete_captured(result).unwrap();
        let completed = exports.poll().unwrap();
        assert_eq!(matches!(&completed, Ok(Outcome::Saved)), saved);
        assert_eq!(matches!(&completed, Ok(Outcome::Cancelled)), cancelled);
        assert_eq!(matches!(&completed, Ok(Outcome::DownloadsRequested)), requested);
        if !saved && !cancelled && !requested {
            assert_eq!(completed.unwrap_err(), "save failed");
        }
        assert!(!exports.busy());
        assert!(exports.poll().is_none());
    }
    exports.start(&ctx, "board.zip".into(), capture("").unwrap()).unwrap();
    drop(exports.completion.take());
    assert!(exports.poll().unwrap().unwrap_err().contains("stopped before reporting"));
    assert!(!exports.busy());
}

/// A directory-sync failure occurs after a complete replacement, is reported to the owner, and
/// leaves no temporary files behind. A refused non-file destination is untouched.
#[test]
fn publish_reports_post_rename_sync_errors_and_preserves_invalid_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.zip");
    fs::write(&path, b"old").unwrap();
    let error = publish_archive_with_sync(&path, b"complete replacement", |parent| {
        assert_eq!(parent, dir.path());
        assert_eq!(fs::read(&path).unwrap(), b"complete replacement");
        Err(std::io::Error::other("injected directory sync failure"))
    })
    .unwrap_err();
    assert!(error.contains("Archive published") && error.contains("injected directory sync failure"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    let folder = dir.path().join("folder.zip");
    fs::create_dir(&folder).unwrap();
    assert!(publish_archive(&folder, b"refused").unwrap_err().contains("not a regular file"));
    assert!(folder.is_dir());
    assert_eq!(fs::read(&path).unwrap(), b"complete replacement");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

/// Each child has its own umask, avoiding unsafe or process-global permission changes in the
/// concurrent test runner. New archives match ordinary creation; overwrites retain prior modes.
#[cfg(unix)]
#[test]
fn archive_permissions_follow_umask_and_preserve_overwrites() {
    for mask in ["027", "077"] {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("sh")
            .args(["-c", "umask \"$1\"; shift; exec \"$@\"", "archive-permissions", mask])
            .arg(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "board_outputs::tests::archive_permission_probe", "--nocapture"])
            .env("PCBCRAFT_ARCHIVE_PERMISSIONS_ROOT", dir.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{mask}: {} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
}

/// Child entry point for a shell-scoped umask; it never changes the test runner's own umask.
#[cfg(unix)]
#[test]
#[ignore = "helper for archive_permissions_follow_umask_and_preserve_overwrites"]
fn archive_permission_probe() {
    use std::os::unix::fs::PermissionsExt;
    let Some(root) = std::env::var_os("PCBCRAFT_ARCHIVE_PERMISSIONS_ROOT") else { return };
    let root = std::path::PathBuf::from(root);
    let ordinary = root.join("ordinary.zip");
    fs::write(&ordinary, b"ordinary").unwrap();
    let archive = root.join("fabrication.zip");
    publish_archive(&archive, b"new archive").unwrap();
    assert_eq!(fs::metadata(&archive).unwrap().permissions().mode() & 0o777, fs::metadata(&ordinary).unwrap().permissions().mode() & 0o777);
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o664)).unwrap();
    publish_archive(&archive, b"replacement").unwrap();
    assert_eq!(fs::metadata(&archive).unwrap().permissions().mode() & 0o777, 0o664);
    assert_eq!(fs::read(&archive).unwrap(), b"replacement");
}
