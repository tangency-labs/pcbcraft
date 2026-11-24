use super::*;
use std::fs;

/// Build an immutable export capture, returning any archive error to the test.
fn capture(text: &str) -> Result<Archive, String> {
    Archive::new(&[pcbcraft_engine::board::fabrication::Output { name: "board.gbr".into(), contents: text.into() }])
}

/// Ordinary re-export replaces the same selected ZIP with the complete new capture and leaves
/// no companion or staging file. An old sidecar neither blocks the replacement nor changes.
#[test]
fn publish_one_zip_and_reexport_at_the_same_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chosen.zip");
    let first = capture("first\n").unwrap();
    let replacement = capture("replacement\n").unwrap();
    assert_ne!(first.bytes(), replacement.bytes());
    for archive in [&first, &replacement] {
        publish_archive_with_sync(&path, archive.bytes(), sync_directory).unwrap();
        assert_eq!(fs::read(&path).unwrap(), archive.bytes());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    let legacy = dir.path().join("chosen.zip.manifest.json");
    fs::write(&legacy, b"old companion").unwrap();
    publish_archive_with_sync(&path, first.bytes(), sync_directory).unwrap();
    assert_eq!(fs::read(&path).unwrap(), first.bytes());
    assert_eq!(fs::read(&legacy).unwrap(), b"old companion");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

/// The browser request passes the entire capture once, with its suggested name and ZIP MIME
/// type. Bridge errors are reported, not promoted to success; no browser runs in this test.
#[test]
fn browser_requests_one_zip_and_propagates_failure() {
    let archive = capture("one download\n").unwrap();
    for response in [Ok(()), Err("bridge refused")] {
        let mut calls = 0;
        let result = request_zip_download("board.zip", &archive, |bytes, name, media_type| {
            calls += 1;
            assert_eq!(bytes, archive.bytes());
            assert_eq!(name, "board.zip");
            assert_eq!(media_type, "application/zip");
            response
        });
        assert_eq!(calls, 1);
        if response.is_ok() {
            assert!(matches!(result, Ok(Outcome::DownloadRequested)));
        } else {
            assert!(result.unwrap_err().contains("bridge refused"));
        }
    }
}

/// Captured tests use the real one-slot completion queue, including cancellation, failure and
/// disconnection. Every completed outcome releases the slot for another export.
#[test]
fn completion_releases_the_export_slot_on_every_outcome() {
    let ctx = egui::Context::default();
    let mut session = Session::default();
    session.apply("project.new", &serde_json::json!({"name":"Outputs","create_board":true})).unwrap();
    let source = Source { board: session.project().unwrap().board_target().unwrap(), opening: session.history_root_revision() };
    let destination = Path::new("board.zip");
    let mut exports = Exports::default();
    for result in [Ok(Outcome::Saved(destination.to_owned())), Ok(Outcome::DownloadRequested), Ok(Outcome::Cancelled), Err("save failed".into())] {
        exports.start(&ctx, source, "board.zip".into(), capture("archive").unwrap(), None).unwrap();
        assert!(exports.busy());
        assert!(exports.poll().is_none());
        assert_eq!(
            exports.start(&ctx, source, "overlap.zip".into(), capture("").unwrap(), None).unwrap_err(),
            "Finish the current fabrication export first."
        );
        let saved = matches!(&result, Ok(Outcome::Saved(_)));
        let cancelled = matches!(&result, Ok(Outcome::Cancelled));
        let requested = matches!(&result, Ok(Outcome::DownloadRequested));
        exports.complete_captured(result).unwrap();
        let completed = exports.poll().unwrap();
        assert_eq!(matches!(&completed, Ok(Outcome::Saved(_))), saved);
        assert_eq!(matches!(&completed, Ok(Outcome::Cancelled)), cancelled);
        assert_eq!(matches!(&completed, Ok(Outcome::DownloadRequested)), requested);
        assert_eq!(exports.latest(&session), Some(destination));
        if !saved && !cancelled && !requested {
            assert_eq!(completed.unwrap_err(), "save failed");
        }
        assert!(!exports.busy());
        assert!(exports.poll().is_none());
    }
    exports.start(&ctx, source, "board.zip".into(), capture("").unwrap(), None).unwrap();
    drop(exports.completion.take());
    assert!(exports.poll().unwrap().unwrap_err().contains("stopped before reporting"));
    assert!(!exports.busy());
    assert_eq!(exports.latest(&session), Some(destination));
    session.open_snapshot(session.snapshot().unwrap());
    assert_eq!(exports.latest(&session), None);
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
    assert!(publish_archive_with_sync(&folder, b"refused", sync_directory).unwrap_err().contains("not a regular file"));
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
    publish_archive_with_sync(&archive, b"new archive", sync_directory).unwrap();
    assert_eq!(fs::metadata(&archive).unwrap().permissions().mode() & 0o777, fs::metadata(&ordinary).unwrap().permissions().mode() & 0o777);
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o664)).unwrap();
    publish_archive_with_sync(&archive, b"replacement", sync_directory).unwrap();
    assert_eq!(fs::metadata(&archive).unwrap().permissions().mode() & 0o777, 0o664);
    assert_eq!(fs::read(&archive).unwrap(), b"replacement");
}
