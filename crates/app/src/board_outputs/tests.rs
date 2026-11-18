use super::*;
use std::fs;

/// Build an immutable export capture, returning any archive error to the test.
fn capture(text: &str) -> Result<Archive, String> {
    Archive::new(&[pcbcraft_engine::board::fabrication::Output { name: "board.gbr".into(), contents: text.into() }])
}

/// The companion binds the selected ZIP filename. Moving an old companion out of the way
/// permits an intentional ZIP replacement without losing the old manifest.
#[test]
fn publish_package_binds_the_selected_filename_and_allows_deliberate_reexport() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chosen.zip");
    let sidecar = dir.path().join("chosen.zip.manifest.json");
    let backup = dir.path().join("previous-manifest.json");
    let first = capture("first\n").unwrap();
    publish_package(&path, &first).unwrap();
    let old_manifest = first.manifest_json("chosen.zip").unwrap();
    assert_eq!(fs::read(&path).unwrap(), first.bytes());
    assert_eq!(fs::read(&sidecar).unwrap(), old_manifest);
    let replacement = capture("replacement\n").unwrap();
    let error = publish_package(&path, &replacement).unwrap_err();
    assert!(error.contains("will not be replaced"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), first.bytes());
    assert_eq!(fs::read(&sidecar).unwrap(), old_manifest);
    fs::rename(&sidecar, &backup).unwrap();
    publish_package(&path, &replacement).unwrap();
    assert_eq!(fs::read(&path).unwrap(), replacement.bytes());
    assert_eq!(fs::read(&sidecar).unwrap(), replacement.manifest_json("chosen.zip").unwrap());
    assert_eq!(fs::read(&backup).unwrap(), old_manifest);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}

/// An unselected unrelated file is preserved, and a companion directory blocks even an
/// otherwise valid ZIP replacement before either output is touched.
#[test]
fn existing_companion_refuses_before_creating_or_replacing_zip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.zip");
    let sidecar = dir.path().join("board.zip.manifest.json");
    let archive = capture("new").unwrap();
    fs::write(&sidecar, b"unrelated data").unwrap();
    let error = publish_package(&path, &archive).unwrap_err();
    assert!(error.contains("board.zip.manifest.json") && error.contains("will not be replaced"), "{error}");
    assert!(!path.exists());
    assert_eq!(fs::read(&sidecar).unwrap(), b"unrelated data");
    fs::remove_file(&sidecar).unwrap();
    fs::create_dir(&sidecar).unwrap();
    fs::write(&path, b"prior ZIP").unwrap();
    let error = publish_package(&path, &archive).unwrap_err();
    assert!(error.contains("board.zip.manifest.json") && error.contains("will not be replaced"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), b"prior ZIP");
    assert!(sidecar.is_dir());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

/// A dangling symlink is an occupied companion path even though its target does not exist.
#[cfg(unix)]
#[test]
fn dangling_companion_symlink_is_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.zip");
    let sidecar = dir.path().join("board.zip.manifest.json");
    let missing = dir.path().join("not-created");
    std::os::unix::fs::symlink(&missing, &sidecar).unwrap();
    let error = publish_package(&path, &capture("new").unwrap()).unwrap_err();
    assert!(error.contains("will not be replaced"), "{error}");
    assert!(!path.exists() && !missing.exists());
    assert_eq!(fs::read_link(&sidecar).unwrap(), missing);
}

/// On Linux's usual 255-byte-component test filesystem, check the last good and first bad
/// companion names while both ZIP names themselves fit.
#[cfg(target_os = "linux")]
#[test]
fn long_companion_name_fails_without_replacing_zip() {
    let dir = tempfile::tempdir().unwrap();
    let archive = capture("new").unwrap();
    let good = format!("{}.zip", "a".repeat(237));
    let bad = format!("{}.zip", "b".repeat(238));
    publish_package(&dir.path().join(&good), &archive).unwrap();
    assert_eq!(fs::read(dir.path().join(format!("{good}.manifest.json"))).unwrap(), archive.manifest_json(&good).unwrap());
    let path = dir.path().join(&bad);
    fs::write(&path, b"prior ZIP").unwrap();
    let error = publish_package(&path, &archive).unwrap_err();
    assert!(error.contains(&format!("{bad}.manifest.json")) && error.contains("Choose a shorter"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), b"prior ZIP");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}

/// A companion arriving after preflight is not overwritten. The real second install fails
/// after the new ZIP is installed and drops the staged manifest.
#[test]
fn racing_companion_is_not_clobbered_after_zip_install() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.zip");
    let sidecar = dir.path().join("board.zip.manifest.json");
    let archive = capture("new").unwrap();
    let error = publish_package_with_sync(&path, &archive, |_| {
        assert_eq!(fs::read(&path).unwrap(), archive.bytes());
        fs::write(&sidecar, b"newly arrived data")
    })
    .unwrap_err();
    assert!(error.contains("ZIP published, but its manifest failed to install"), "{error}");
    assert_eq!(fs::read(&sidecar).unwrap(), b"newly arrived data");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

/// Refusal at the selected ZIP does not install the prepared companion or leak staging files.
#[test]
fn invalid_archive_destination_does_not_install_companion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.zip");
    let sidecar = dir.path().join("board.zip.manifest.json");
    fs::create_dir(&path).unwrap();
    let error = publish_package(&path, &capture("new").unwrap()).unwrap_err();
    assert!(error.contains("Manifest not installed") && error.contains("not a regular file"), "{error}");
    assert!(!sidecar.exists() && path.is_dir());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

/// At either injected directory-sync error, the corresponding install has already happened.
/// The first failure stops before companion installation; the second leaves both new files.
#[test]
fn package_reports_each_post_install_sync_failure() {
    for fail_at in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.zip");
        let sidecar = dir.path().join("board.zip.manifest.json");
        fs::write(&path, b"prior ZIP").unwrap();
        let archive = capture("new").unwrap();
        let mut calls = 0;
        let error = publish_package_with_sync(&path, &archive, |parent| {
            assert_eq!(parent, dir.path());
            calls += 1;
            if calls == fail_at { Err(std::io::Error::other("injected directory sync failure")) } else { Ok(()) }
        })
        .unwrap_err();
        assert_eq!(calls, fail_at);
        assert!(error.contains("injected directory sync failure"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), archive.bytes());
        if fail_at == 1 {
            assert!(error.contains("Manifest not installed") && error.contains("Archive published"), "{error}");
            assert!(!sidecar.exists());
        } else {
            assert!(error.contains("ZIP and manifest installed") && error.contains("durability is unconfirmed"), "{error}");
            assert_eq!(fs::read(&sidecar).unwrap(), archive.manifest_json("board.zip").unwrap());
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), fail_at);
    }
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
