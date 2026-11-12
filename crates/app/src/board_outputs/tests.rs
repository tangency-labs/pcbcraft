use super::*;
use std::fs;

/// Captured tests use the real one-slot completion queue, including cancellation, failure and
/// disconnection. Every completed outcome releases the slot for another export.
#[test]
fn completion_releases_the_export_slot_on_every_outcome() {
    let ctx = egui::Context::default();
    let mut exports = Exports::default();
    for result in [Ok(Outcome::Saved), Ok(Outcome::Cancelled), Err("save failed".into())] {
        exports.start(&ctx, "board.zip".into(), b"archive".to_vec()).unwrap();
        assert!(exports.busy());
        assert!(exports.poll().is_none());
        assert!(exports.start(&ctx, "overlap.zip".into(), vec![]).is_err());
        let saved = matches!(&result, Ok(Outcome::Saved));
        let cancelled = matches!(&result, Ok(Outcome::Cancelled));
        exports.complete_captured(result).unwrap();
        let completed = exports.poll().unwrap();
        assert_eq!(matches!(&completed, Ok(Outcome::Saved)), saved);
        assert_eq!(matches!(&completed, Ok(Outcome::Cancelled)), cancelled);
        if !saved && !cancelled {
            assert_eq!(completed.unwrap_err(), "save failed");
        }
        assert!(!exports.busy());
        assert!(exports.poll().is_none());
    }
    exports.start(&ctx, "board.zip".into(), vec![]).unwrap();
    drop(exports.completion.take());
    assert!(exports.poll().unwrap().unwrap_err().contains("stopped before reporting"));
    assert!(!exports.busy());
}

/// Assembly archives retain their explicit variant name, success wording and owner through cancellation.
#[test]
fn assembly_exports_capture_the_variant_name_and_completion_purpose() {
    let ctx = egui::Context::default();
    let mut exports = Exports::default();
    let name = assembly_archive_name("Prototype Project", Some("Rev C"));
    assert_eq!(name, "Prototype_Project-Rev_C-assembly.zip");

    for saved in [true, false] {
        exports.start_for(&ctx, Purpose::Assembly, name.clone(), b"archive".to_vec()).unwrap();
        assert_eq!(exports.captured.as_ref().unwrap(), &(name.clone(), b"archive".to_vec()));
        exports.complete_captured(Ok(if saved { Outcome::Saved } else { Outcome::Cancelled })).unwrap();
        let (purpose, outcome) = exports.poll_with_purpose().unwrap().unwrap();
        assert_eq!(purpose, Purpose::Assembly);
        assert_eq!(purpose.success_message(), "Saved assembly outputs");
        assert_eq!(matches!(outcome, Outcome::Saved), saved);
        assert!(!exports.busy());
    }
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
