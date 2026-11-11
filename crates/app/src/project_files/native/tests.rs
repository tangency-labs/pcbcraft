//! Native storage acceptance tests. The operating system picker is bypassed
//! by supplying its selected path to the same file worker code.

use std::sync::{Barrier, mpsc};

use pcbcraft_engine::{
    MOVE_SYMBOL, PLACE_FOOTPRINT, Session,
    persistence::{MAX_DOCUMENT_BYTES, decode, encode_packed},
    schematic::Point,
};
use serde_json::{Value, json};

use super::*;
use crate::project_files::{Job, ProjectFiles, Update};

const TWO_RESISTORS: &[u8] = include_bytes!("../../../tests/fixtures/two-resistors.pc");
const STEP: i64 = 1_270_000;
const R2_X: i64 = 25_400_000;

struct Fixture {
    dir: tempfile::TempDir,
    path: PathBuf,
    session: Session,
    files: ProjectFiles,
}

impl Fixture {
    /// Package the original fixture and reopen it through the native file completion path.
    fn open() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two-resistors.pc");
        let original = decode(TWO_RESISTORS).unwrap();
        save_new(original, None, SaveFormat::Archive, path.clone()).unwrap();
        let mut session = Session::default();
        let mut files = ProjectFiles::default();
        let result = load(&path).map(Outcome::Opened);
        assert!(matches!(complete(&mut files, &mut session, result), Some(Update::Opened)));
        Self { dir, path, session, files }
    }

    /// Save through the production archive path against its retained snapshot and baseline.
    fn save(&mut self, after_staging: impl FnOnce() -> Result<(), String>) -> Option<Update> {
        let snapshot = self.session.snapshot().unwrap();
        let saved = self.files.saved.as_ref().unwrap();
        let mut after_staging = Some(after_staging);
        let result = save_target(snapshot, Some(&saved.snapshot), saved.target.clone(), &mut |_| after_staging.take().unwrap()());
        complete(&mut self.files, &mut self.session, result)
    }
}

/// Deliver a completed worker result using the editor's normal adoption logic.
fn complete(files: &mut ProjectFiles, session: &mut Session, result: Result<Outcome, String>) -> Option<Update> {
    let (send, receiver) = mpsc::channel();
    files.job = Some(Job { receiver, revision: session.revision() });
    send.send(result).unwrap();
    files.poll(session, &egui::Context::default())
}

fn move_symbol(session: &mut Session, id: &str, x_nm: i64, y_nm: i64) {
    session.apply(MOVE_SYMBOL, &json!({"symbol":id, "x_nm":x_nm, "y_nm":y_nm, "expected_revision":session.revision()})).unwrap();
}

fn positions(project: &Project) -> Vec<Point> {
    project.symbols().iter().map(|symbol| symbol.position).collect()
}

fn inspect(project: Arc<Project>) -> Value {
    let mut session = Session::default();
    session.open_snapshot(project);
    session.inspect()["project"]["schematic"].clone()
}

/// Assert exact persisted connectivity, including isolated terminals and the wire ID. The drawn
/// `path` is derived from the symbols, not stored, so it is left out.
fn assert_connectivity(schematic: &Value) {
    let left = json!({"symbol":"symbol-2", "terminal":"2"});
    let right = json!({"symbol":"symbol-3", "terminal":"1"});
    let mut wires = schematic["wires"].clone();
    for wire in wires.as_array_mut().into_iter().flatten() {
        wire.as_object_mut().map(|wire| wire.remove("path"));
    }
    assert_eq!(wires, json!([{"id":"wire-4", "from":left, "to":right}]));
    assert_eq!(
        schematic["netlist"],
        json!({
            "nets":[{"terminals":[left, right]}],
            "unconnected":[{"symbol":"symbol-2", "terminal":"1"}, {"symbol":"symbol-3", "terminal":"2"}]
        })
    );
}

/// A no-op archive save preserves all bytes, including a custom ZIP comment.
#[test]
fn two_resistors_round_trip_and_save_without_a_diff() {
    let mut fixture = Fixture::open();
    let loaded = fixture.session.snapshot().unwrap();
    assert!(loaded.same_document(&decode(TWO_RESISTORS).unwrap()));
    assert_eq!(positions(&loaded), [Point { x_nm: 0, y_nm: 0 }, Point { x_nm: R2_X, y_nm: 0 }]);
    assert_connectivity(&inspect(loaded));
    let mut noncanonical = read(&fixture.path).unwrap();
    let comment = b"custom ZIP comment";
    let len = noncanonical.len();
    noncanonical[len - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    noncanonical.extend_from_slice(comment);
    fs::write(&fixture.path, &noncanonical).unwrap();
    let reopened = load(&fixture.path).map(Outcome::Opened);
    complete(&mut fixture.files, &mut fixture.session, reopened);
    assert!(matches!(fixture.save(|| Ok(())), Some(Update::Message(_))));
    assert_eq!(read(&fixture.path).unwrap(), noncanonical);
    assert!(!fixture.files.dirty(&fixture.session));
    assert!(save_new(fixture.session.snapshot().unwrap(), None, SaveFormat::Archive, fixture.path.clone()).is_err());
    assert_eq!(read(&fixture.path).unwrap(), noncanonical);
}

/// Invalid current packages reach their schema validators, while oversized inputs hit the
/// native read bound. Neither failure replaces the project or clears its unsaved edits.
#[test]
fn failed_opens_preserve_the_active_project_and_dirty_state() {
    let mut fixture = Fixture::open();
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let before = fixture.session.snapshot().unwrap();
    let revision = fixture.session.revision();
    let invalid = fixture.dir.path().join("invalid.pc");
    let original = encode_documents(decode(TWO_RESISTORS).unwrap()).unwrap();
    let sheet = original.keys().find(|path| path.starts_with("sheets/")).unwrap().clone();
    let mut cases = Vec::new();
    for kind in ["format", "component", "unit", "pin"] {
        let mut documents = original.clone();
        let path = if kind == "format" { ENTRY_POINT } else { &sheet };
        let mut value: Value = serde_json::from_slice(&documents[path]).unwrap();
        match kind {
            "format" => value["format"] = json!("pcbcraft-project/999"),
            "component" => value["symbols"][0]["component"] = json!(999),
            "unit" => value["symbols"][0]["unit"] = json!("unknown"),
            "pin" => {
                assert!(value["wires"][0]["from"].get("pin").is_some());
                value["wires"][0]["from"]["pin"] = json!("9");
            }
            _ => continue,
        }
        documents.insert(path.to_owned(), serde_json::to_vec(&value).unwrap());
        cases.push((kind, encode_packed_documents(documents).unwrap()));
    }
    cases.push(("size limit", vec![b' '; MAX_DOCUMENT_BYTES + 1]));
    for (kind, payload) in cases {
        fs::write(&invalid, payload).unwrap();
        let error = load(&invalid).err().unwrap();
        assert!(!error.contains("Unsupported project layout"), "{kind}: {error}");
        assert!(error.to_ascii_lowercase().contains(kind), "{kind}: {error}");
        assert!(matches!(complete(&mut fixture.files, &mut fixture.session, Err(error)), Some(Update::Error(_))));
        assert!(Arc::ptr_eq(&before, &fixture.session.snapshot().unwrap()));
        assert_eq!(fixture.session.revision(), revision);
        assert!(fixture.files.dirty(&fixture.session));
    }
}

/// Old standalone documents, container-v1 packages and incomplete split folders fail consistently
/// through desktop and headless readers without rewriting their sources or the active session.
#[test]
fn legacy_layouts_are_refused_without_adoption_or_source_changes() {
    let mut fixture = Fixture::open();
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let before = fixture.session.snapshot().unwrap();
    let revision = fixture.session.revision();
    let legacy_zip = pcbcraft_engine::persistence::encode_file(decode(TWO_RESISTORS).unwrap(), Encoding::Zip).unwrap();
    for (name, bytes) in [("legacy.pc", TWO_RESISTORS), ("renamed.pc", legacy_zip.as_slice())] {
        let path = fixture.dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        assert!(pcbcraft_storage::open(&path).is_err());
        let result = load(&path).map(Outcome::Opened);
        assert!(matches!(complete(&mut fixture.files, &mut fixture.session, result), Some(Update::Error(_))));
        assert!(Arc::ptr_eq(&before, &fixture.session.snapshot().unwrap()));
        assert_eq!(fixture.session.revision(), revision);
        assert!(fixture.files.dirty(&fixture.session));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    let root = fixture.dir.path().join("old-folder");
    fs::create_dir(&root).unwrap();
    let documents = encode_documents(before.clone()).unwrap();
    let entry = root.join("project.pc");
    fs::write(&entry, &documents[ENTRY_POINT]).unwrap();
    for path in [&root, &entry] {
        assert!(load(path).is_err());
        assert!(pcbcraft_storage::open(path).is_err());
    }
    assert_eq!(fs::read(entry).unwrap(), documents[ENTRY_POINT]);
    assert!(!root.join(ENTRY_POINT).exists());
}

/// Failure after a complete stage leaves the previous file and the current edits intact.
#[test]
fn interrupted_publication_keeps_the_previous_file() {
    assert_interrupted_save();
}

/// Exercise interruption after staging a current archive.
fn assert_interrupted_save() {
    let mut fixture = Fixture::open();
    let original = read(&fixture.path).unwrap();
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let before = fixture.session.snapshot().unwrap();
    let update = fixture.save(|| Err("simulated failure before publication".to_owned()));
    assert!(matches!(update, Some(Update::Error(message)) if message == "simulated failure before publication"));
    assert_eq!(read(&fixture.path).unwrap(), original);
    assert!(Arc::ptr_eq(&before, &fixture.session.snapshot().unwrap()));
    assert!(fixture.files.dirty(&fixture.session));
}

/// Editing a shared project preserves the destination's permission bits.
#[cfg(unix)]
#[test]
fn edited_save_preserves_project_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let mut fixture = Fixture::open();
    let mode = 0o640;
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(mode)).unwrap();
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);

    assert!(matches!(fixture.save(|| Ok(())), Some(Update::Message(_))));
    assert_eq!(fs::metadata(&fixture.path).unwrap().permissions().mode() & 0o777, mode);
    assert_eq!(positions(&load(&fixture.path).unwrap().snapshot)[0], Point { x_nm: 0, y_nm: STEP });
}

/// On Windows an archive's save lock is created hidden, like a folder's, through
/// `pcbcraft_storage::open_lock_file`.
#[cfg(windows)]
#[test]
fn windows_archive_save_lock_is_created_hidden() {
    use std::os::windows::fs::MetadataExt;
    // Windows' FILE_ATTRIBUTE_HIDDEN.
    const HIDDEN: u32 = 0x2;
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("board.pc");
    publish(&archive, None, &encode_packed(decode(TWO_RESISTORS).unwrap()).unwrap(), || Ok(())).unwrap();
    let attributes = fs::metadata(dir.path().join(".board.pc.pcbcraft-lock")).unwrap().file_attributes();
    assert_ne!(attributes & HIDDEN, 0, "lock attributes {attributes:#x}");
    assert!(load(&archive).is_ok());
}

/// Competing writes from the same baseline leave only the winning session clean.
#[test]
fn one_writer_publishes_against_a_shared_baseline() {
    assert_one_writer();
}

/// Race two archive writers starting from the same bytes.
fn assert_one_writer() {
    let mut first = Fixture::open();
    let loaded = load(&first.path).unwrap();
    let mut second = Session::default();
    let mut second_files = ProjectFiles::default();
    assert!(matches!(complete(&mut second_files, &mut second, Ok(Outcome::Opened(loaded))), Some(Update::Opened)));
    move_symbol(&mut first.session, "symbol-2", 0, STEP);
    move_symbol(&mut second, "symbol-3", R2_X, STEP);
    let first_snapshot = first.session.snapshot().unwrap();
    let second_snapshot = second.snapshot().unwrap();
    let target = first.files.saved.as_ref().unwrap().target.clone();
    let original = first.files.saved.as_ref().unwrap().snapshot.clone();
    let barrier = Barrier::new(2);
    let run = |snapshot| {
        barrier.wait();
        save_target(snapshot, Some(&original), target.clone(), &mut |_| Ok(()))
    };
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| run(first_snapshot.clone()));
        let b = scope.spawn(|| run(second_snapshot.clone()));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_ne!(a.is_ok(), b.is_ok());
    let first_won = a.is_ok();
    complete(&mut first.files, &mut first.session, a);
    complete(&mut second_files, &mut second, b);
    assert_eq!(first.files.dirty(&first.session), !first_won);
    assert_eq!(second_files.dirty(&second), first_won);
    let winner = if first_won { first_snapshot } else { second_snapshot };
    assert_eq!(read(&first.path).unwrap(), encode_packed(winner).unwrap());
}

/// Pause the real worker after staging R1, move and rename R2, then verify which snapshot reached disk.
#[test]
fn a_late_move_is_not_marked_saved() {
    assert_late_move();
}

/// Pause after staging an archive so a later edit stays dirty.
fn assert_late_move() {
    let mut fixture = Fixture::open();
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let staged = fixture.session.snapshot().unwrap();
    let target = fixture.files.saved.as_ref().unwrap().target.clone();
    let original = fixture.files.saved.as_ref().unwrap().snapshot.clone();
    let (send, receiver) = mpsc::channel();
    fixture.files.job = Some(Job { receiver, revision: fixture.session.revision() });

    std::thread::scope(|scope| {
        let (ready, wait_ready) = mpsc::sync_channel(0);
        let (release, wait_release) = mpsc::sync_channel(0);
        scope.spawn(move || {
            let result = save_target(staged, Some(&original), target, &mut |_| {
                ready.send(()).map_err(|error| error.to_string())?;
                wait_release.recv().map_err(|error| error.to_string())
            });
            send.send(result).unwrap();
        });
        wait_ready.recv().unwrap();
        move_symbol(&mut fixture.session, "symbol-3", R2_X, STEP);
        fixture
            .session
            .apply(
                pcbcraft_engine::SET_SYMBOL_FIELDS,
                &json!({
                    "symbol": "symbol-3", "reference": "R7", "value": "10k", "expected_revision": fixture.session.revision()
                }),
            )
            .unwrap();
        release.send(()).unwrap();
    });
    assert!(matches!(fixture.files.poll(&mut fixture.session, &egui::Context::default()), Some(Update::Message(_))));
    let reopened = load(&fixture.path).unwrap().snapshot;
    assert_eq!(positions(&reopened), [Point { x_nm: 0, y_nm: STEP }, Point { x_nm: R2_X, y_nm: 0 }]);
    assert_eq!(reopened.reference(&reopened.symbols()[1]), "R2");
    assert_connectivity(&inspect(reopened));
    let active = fixture.session.project().unwrap();
    assert_eq!(active.reference(&active.symbols()[1]), "R7");
    assert_eq!(positions(fixture.session.project().unwrap()), [Point { x_nm: 0, y_nm: STEP }, Point { x_nm: R2_X, y_nm: STEP }]);
    assert!(fixture.files.dirty(&fixture.session));
}

/// Folder/archive conversion retains pin mapping and stable identities; subsequent Save keeps
/// the packed form and updates its saved baseline after an authored move.
#[test]
fn convert_zip_round_trip_and_keep_the_existing_encoding() {
    let mut fixture = Fixture::open();
    fixture
        .session
        .apply(
            PLACE_FOOTPRINT,
            &json!({"symbol":"symbol-2", "definition":"generic-through-hole-2", "pin_1_pad":2,
        "x_nm":0,"y_nm":0,"expected_revision":fixture.session.revision()}),
        )
        .unwrap();
    let snapshot = fixture.session.snapshot().unwrap();
    let terminal = snapshot.symbols()[0].terminals()[0];
    let pad = snapshot.pad_for_terminal(terminal).unwrap();
    let root = fixture.dir.path().join("unpacked");
    save_new(snapshot.clone(), None, SaveFormat::Folder, root.clone()).unwrap();
    let folder = load(&root.join(ENTRY_POINT)).unwrap();
    assert_eq!(folder.snapshot.pad_for_terminal(terminal), Some(pad));
    let path = fixture.dir.path().join("converted.pc");
    let previous = (folder.snapshot, folder.target);
    save_new(snapshot, Some(&previous), SaveFormat::Archive, path.clone()).unwrap();
    let reopened = load(&path).unwrap();
    assert_eq!(reopened.target.encoding, Encoding::Zip);
    assert_eq!(reopened.snapshot.terminal_for_pad(pad), Some(terminal));
    assert_connectivity(&inspect(reopened.snapshot.clone()));
    let before = reopened.snapshot.clone();
    fixture.session.open_snapshot(before.clone());
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    save_target(fixture.session.snapshot().unwrap(), Some(&before), reopened.target, &mut |_| Ok(())).unwrap();
    let edited = load(&path).unwrap();
    assert_eq!(edited.target.encoding, Encoding::Zip);
    assert_eq!(positions(&edited.snapshot)[0], Point { x_nm: 0, y_nm: STEP });
    assert_connectivity(&inspect(edited.snapshot));
}

/// ZIP validation failures must not adopt an invalid snapshot or mark edits clean.
#[test]
fn failed_zip_opens_preserve_the_active_project_and_dirty_state() {
    let mut fixture = Fixture::open();
    let original = read(&fixture.path).unwrap();
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let before = fixture.session.snapshot().unwrap();
    let revision = fixture.session.revision();
    let invalid = fixture.dir.path().join("invalid.pc");

    let rename = |name: &[u8], replacement: &[u8]| {
        let mut changed = original.clone();
        for index in (0..changed.len() - name.len()).rev() {
            if &changed[index..index + name.len()] == name {
                changed[index..index + name.len()].copy_from_slice(replacement);
            }
        }
        changed
    };
    for payload in [
        original[..original.len() - 5].to_vec(),
        rename(b"project.json", b"../evil.json"),
        rename(b"project.json", b"foreign.json"),
        rename(b"project.json", b"project.js0n"),
        rename(b"pcbcraft-container/3", b"pcbcraft-container/9"),
    ] {
        fs::write(&invalid, payload).unwrap();
        let result = load(&invalid).map(Outcome::Opened);
        assert!(matches!(complete(&mut fixture.files, &mut fixture.session, result), Some(Update::Error(_))));
        assert!(Arc::ptr_eq(&before, &fixture.session.snapshot().unwrap()));
        assert_eq!(fixture.session.revision(), revision);
        assert!(fixture.files.dirty(&fixture.session));
    }
}

/// Open and save targets keep the drive form people chose. Guards #154: Windows
/// `canonicalize` returns `\\?\C:\…`, which leaked into Recent projects and saved history.
#[test]
fn normalized_targets_keep_the_plain_drive_form() {
    let dir = tempfile::tempdir().unwrap();
    let target = normalize(&dir.path().join("board.pc")).unwrap();
    assert!(target.is_absolute());
    assert!(!target.to_string_lossy().starts_with(r"\\?\"), "verbatim target: {}", target.display());
    assert_eq!(target.file_name(), Some(std::ffi::OsStr::new("board.pc")));
}

/// The plain spelling is used only when Windows reads it as the same file. Guards the #155
/// review: every Windows version drops a trailing dot or space from the plain form, so those
/// targets keep `\\?\`. Simplifying only the parent folder missed this.
#[cfg(windows)]
#[test]
fn targets_whose_plain_spelling_differs_keep_the_verbatim_form() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["board.pc.", "board.pc "] {
        let target = normalize(&dir.path().join(name)).unwrap();
        assert!(target.to_string_lossy().starts_with(r"\\?\"), "{name}: {}", target.display());
        assert_eq!(target.file_name(), Some(std::ffi::OsStr::new(name)));
    }
}

/// A device-named target such as `CON.pc` keeps `\\?\` exactly when this Windows reads
/// its plain spelling as the device. Windows 10 and earlier do; Windows 11 reads it as an
/// ordinary file, and then the plain spelling is the one people chose.
#[cfg(windows)]
#[test]
fn device_named_targets_follow_how_windows_reads_them() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["CON.pc", "NUL.pc", "COM1.pc"] {
        let target = normalize(&dir.path().join(name)).unwrap();
        let text = target.to_string_lossy();
        let plain = std::path::PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));
        let device = std::path::absolute(&plain).unwrap() != plain;
        assert_eq!(target.to_string_lossy().starts_with(r"\\?\"), device, "{name}: {}", target.display());
        assert_eq!(target.file_name(), Some(std::ffi::OsStr::new(name)));
    }
}
