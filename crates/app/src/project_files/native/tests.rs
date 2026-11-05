//! Native storage acceptance tests. The operating system picker is bypassed
//! by supplying its selected path to the same file worker code.

use std::{
    process::Command,
    sync::{Barrier, mpsc},
};

use pcbcraft_engine::{
    MOVE_SYMBOL, PLACE_FOOTPRINT, Session,
    persistence::{MAX_PROJECT_BYTES, decode, encode},
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
    /// Save the original fixture and reopen it through the native file completion path.
    fn open(encoding: Encoding) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two-resistors.pc");
        let original = decode(TWO_RESISTORS).unwrap();
        let result = save_at(original, path.clone(), None, false, encoding, || Ok(()));
        assert!(matches!(result, Ok(Outcome::Saved(_))));
        let mut session = Session::default();
        let mut files = ProjectFiles::default();
        let result = load(&path).map(Outcome::Opened);
        assert!(matches!(complete(&mut files, &mut session, result), Some(Update::Opened)));
        Self { dir, path, session, files }
    }

    /// Save the active revision against the baseline recorded by the file job.
    fn save(&mut self, after_staging: impl FnOnce() -> Result<(), String>) -> Option<Update> {
        let snapshot = self.session.snapshot().unwrap();
        let saved = self.files.saved.as_ref().unwrap();
        let result = save_at(
            snapshot.clone(),
            self.path.clone(),
            Some(saved.target.bytes.clone()),
            Arc::ptr_eq(&snapshot, &saved.snapshot),
            saved.target.encoding,
            after_staging,
        );
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

/// Assert exact persisted connectivity, including isolated terminals and the wire ID.
fn assert_connectivity(schematic: &Value) {
    let left = json!({"symbol":"symbol-2", "terminal":"2"});
    let right = json!({"symbol":"symbol-3", "terminal":"1"});
    assert_eq!(schematic["wires"], json!([{"id":"wire-4", "from":left, "to":right}]));
    assert_eq!(
        schematic["netlist"],
        json!({
            "nets":[{"terminals":[left, right]}],
            "unconnected":[{"symbol":"symbol-2", "terminal":"1"}, {"symbol":"symbol-3", "terminal":"2"}]
        })
    );
}

/// A native save and reopen retain stable IDs, placements, and pin membership.
/// An unchanged save keeps every byte, including formatting from the input file.
#[test]
fn two_resistors_round_trip_and_save_without_a_diff() {
    let mut fixture = Fixture::open(Encoding::Json);
    assert_eq!(read(&fixture.path).unwrap(), TWO_RESISTORS);
    let loaded = fixture.session.snapshot().unwrap();
    assert!(loaded.same_document(&decode(TWO_RESISTORS).unwrap()));
    assert_eq!(positions(&loaded), [Point { x_nm: 0, y_nm: 0 }, Point { x_nm: R2_X, y_nm: 0 }]);
    let schematic = inspect(loaded);
    for (index, symbol) in schematic["symbols"].as_array().unwrap().iter().enumerate() {
        assert_eq!(symbol["id"], format!("symbol-{}", index + 2));
        assert_eq!(symbol["component_id"], format!("component-{}", index + 2));
        assert_eq!(symbol["reference"], format!("R{}", index + 1));
        assert_eq!(symbol["kind"], "resistor");
    }
    assert_connectivity(&schematic);

    let mut noncanonical = TWO_RESISTORS.to_vec();
    noncanonical.extend_from_slice(b"\n ");
    fs::write(&fixture.path, &noncanonical).unwrap();
    let reopened = load(&fixture.path).map(Outcome::Opened);
    assert!(matches!(complete(&mut fixture.files, &mut fixture.session, reopened), Some(Update::Opened)));
    assert!(matches!(fixture.save(|| Ok(())), Some(Update::Message(_))));
    assert_eq!(read(&fixture.path).unwrap(), noncanonical);
    assert!(!fixture.files.dirty(&fixture.session));

    let snapshot = fixture.session.snapshot().unwrap();
    assert!(save_at(snapshot, fixture.path.clone(), None, false, Encoding::Json, || Ok(())).is_err());
    assert_eq!(read(&fixture.path).unwrap(), noncanonical);
}

/// Two independent symbol moves change one record each and merge with Git's normal three-way driver.
#[test]
fn independent_placement_edits_merge_and_reopen() {
    let mut fixture = Fixture::open(Encoding::Json);
    let base = fixture.dir.path().join("base.pc");
    let other = fixture.dir.path().join("other.pc");
    fs::write(&base, TWO_RESISTORS).unwrap();
    fs::write(&other, TWO_RESISTORS).unwrap();

    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    assert!(matches!(fixture.save(|| Ok(())), Some(Update::Message(_))));
    let left = fs::read_to_string(&fixture.path).unwrap();
    let original = std::str::from_utf8(TWO_RESISTORS).unwrap();
    let changed: Vec<_> = original.lines().zip(left.lines()).filter(|(before, after)| before != after).collect();
    assert_eq!(original.lines().count(), left.lines().count());
    assert_eq!(changed.len(), 1);
    assert!(changed[0].1.contains("\"id\":2"));
    assert_connectivity(&inspect(load(&fixture.path).unwrap().snapshot));

    let mut branch = Session::default();
    let loaded = load(&other).unwrap();
    branch.open_snapshot(loaded.snapshot);
    move_symbol(&mut branch, "symbol-3", R2_X, STEP);
    let result = save_at(branch.snapshot().unwrap(), other.clone(), Some(loaded.target.bytes), false, Encoding::Json, || Ok(()));
    assert!(matches!(result, Ok(Outcome::Saved(_))));

    let merge = Command::new("git").arg("merge-file").arg("-p").arg(&fixture.path).arg(&base).arg(&other).output().unwrap();
    assert!(merge.status.success(), "{}", String::from_utf8_lossy(&merge.stdout));
    let merged_path = fixture.dir.path().join("merged.pc");
    fs::write(&merged_path, merge.stdout).unwrap();
    let merged = load(&merged_path).unwrap().snapshot;
    assert_eq!(positions(&merged), [Point { x_nm: 0, y_nm: STEP }, Point { x_nm: R2_X, y_nm: STEP }]);
    assert_connectivity(&inspect(merged));
}

/// Invalid input never replaces the open project or clears its unsaved edits.
#[test]
fn failed_opens_preserve_the_active_project_and_dirty_state() {
    let mut fixture = Fixture::open(Encoding::Json);
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let before = fixture.session.snapshot().unwrap();
    let revision = fixture.session.revision();
    let invalid = fixture.dir.path().join("invalid.pc");
    let original = std::str::from_utf8(TWO_RESISTORS).unwrap();
    for payload in [
        original.replace("pcbcraft-project/1", "pcbcraft-project/999").into_bytes(),
        vec![b' '; MAX_PROJECT_BYTES + 1],
        original.replace("\"pin\":\"2\"", "\"pin\":\"9\"").into_bytes(),
    ] {
        fs::write(&invalid, payload).unwrap();
        let result = load(&invalid).map(Outcome::Opened);
        assert!(matches!(complete(&mut fixture.files, &mut fixture.session, result), Some(Update::Error(_))));
        assert!(Arc::ptr_eq(&before, &fixture.session.snapshot().unwrap()));
        assert_eq!(fixture.session.revision(), revision);
        assert!(fixture.files.dirty(&fixture.session));
    }
}

/// Failure after a complete stage leaves the previous file and the current edits intact.
#[test]
fn interrupted_publication_keeps_the_previous_file() {
    for encoding in [Encoding::Json, Encoding::Zip] {
        assert_interrupted_save(encoding);
    }
}

/// Exercise the interruption after staging the selected encoding.
fn assert_interrupted_save(encoding: Encoding) {
    let mut fixture = Fixture::open(encoding);
    let original = read(&fixture.path).unwrap();
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let before = fixture.session.snapshot().unwrap();
    let update = fixture.save(|| Err("simulated failure before publication".to_owned()));
    assert!(matches!(update, Some(Update::Error(message)) if message == "simulated failure before publication"));
    assert_eq!(read(&fixture.path).unwrap(), original);
    assert!(Arc::ptr_eq(&before, &fixture.session.snapshot().unwrap()));
    assert!(fixture.files.dirty(&fixture.session));
}

/// Competing writes from the same baseline leave only the winning session clean.
#[test]
fn one_writer_publishes_against_a_shared_baseline() {
    for encoding in [Encoding::Json, Encoding::Zip] {
        assert_one_writer(encoding);
    }
}

/// Race two writers starting from the same bytes in the selected encoding.
fn assert_one_writer(encoding: Encoding) {
    let mut first = Fixture::open(encoding);
    let loaded = load(&first.path).unwrap();
    let mut second = Session::default();
    let mut second_files = ProjectFiles::default();
    assert!(matches!(complete(&mut second_files, &mut second, Ok(Outcome::Opened(loaded))), Some(Update::Opened)));
    move_symbol(&mut first.session, "symbol-2", 0, STEP);
    move_symbol(&mut second, "symbol-3", R2_X, STEP);
    let first_snapshot = first.session.snapshot().unwrap();
    let second_snapshot = second.snapshot().unwrap();
    let baseline = first.files.saved.as_ref().unwrap().target.bytes.clone();
    let barrier = Barrier::new(2);
    let run = |snapshot| {
        barrier.wait();
        save_at(snapshot, first.path.clone(), Some(baseline.clone()), false, encoding, || Ok(()))
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
    assert_eq!(read(&first.path).unwrap(), encode_file(winner, encoding).unwrap());
}

/// Pause the real worker after staging R1, edit R2, then verify which snapshot reached disk.
#[test]
fn a_late_move_is_not_marked_saved() {
    for encoding in [Encoding::Json, Encoding::Zip] {
        assert_late_move(encoding);
    }
}

/// Pause after staging in the selected encoding so a later edit stays dirty.
fn assert_late_move(encoding: Encoding) {
    let mut fixture = Fixture::open(encoding);
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let staged = fixture.session.snapshot().unwrap();
    let baseline = fixture.files.saved.as_ref().unwrap().target.bytes.clone();
    let path = fixture.path.clone();
    let (send, receiver) = mpsc::channel();
    fixture.files.job = Some(Job { receiver, revision: fixture.session.revision() });

    std::thread::scope(|scope| {
        let (ready, wait_ready) = mpsc::sync_channel(0);
        let (release, wait_release) = mpsc::sync_channel(0);
        scope.spawn(move || {
            let result = save_at(staged, path, Some(baseline), false, encoding, || {
                ready.send(()).map_err(|error| error.to_string())?;
                wait_release.recv().map_err(|error| error.to_string())
            });
            send.send(result).unwrap();
        });
        wait_ready.recv().unwrap();
        move_symbol(&mut fixture.session, "symbol-3", R2_X, STEP);
        release.send(()).unwrap();
    });
    assert!(matches!(fixture.files.poll(&mut fixture.session, &egui::Context::default()), Some(Update::Message(_))));
    let reopened = load(&fixture.path).unwrap().snapshot;
    assert_eq!(positions(&reopened), [Point { x_nm: 0, y_nm: STEP }, Point { x_nm: R2_X, y_nm: 0 }]);
    assert_connectivity(&inspect(reopened));
    assert_eq!(positions(fixture.session.project().unwrap()), [Point { x_nm: 0, y_nm: STEP }, Point { x_nm: R2_X, y_nm: STEP }]);
    assert!(fixture.files.dirty(&fixture.session));
}

/// Converting in both directions preserves IDs, connections, and explicit pin-to-pad mappings.
/// Normal Save retains the ZIP encoding and the exact imported bytes until an edit is made.
#[test]
fn convert_zip_round_trip_and_keep_the_existing_encoding() {
    let mut fixture = Fixture::open(Encoding::Json);
    fixture
        .session
        .apply(
            PLACE_FOOTPRINT,
            &json!({
                "symbol":"symbol-2", "definition":"generic-through-hole-2", "pin_1_pad":2,
                "x_nm":0, "y_nm":0, "expected_revision":fixture.session.revision()
            }),
        )
        .unwrap();
    let snapshot = fixture.session.snapshot().unwrap();
    let json_bytes = encode(snapshot.clone()).unwrap();
    let terminal = snapshot.symbols()[0].terminals()[0];
    let pad = snapshot.pad_for_terminal(terminal).unwrap();

    let path = fixture.dir.path().join("archive.pc");
    save_at(snapshot.clone(), path.clone(), None, false, Encoding::Zip, || Ok(())).unwrap();
    let mut noncanonical = read(&path).unwrap();
    let comment = b"custom ZIP comment";
    let len = noncanonical.len();
    noncanonical[len - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    noncanonical.extend_from_slice(comment);
    fs::write(&path, &noncanonical).unwrap();
    let reopened = load(&path).unwrap();
    assert_eq!(reopened.target.encoding, Encoding::Zip);
    assert_connectivity(&inspect(reopened.snapshot.clone()));
    assert_eq!(reopened.snapshot.pad_for_terminal(terminal), Some(pad));
    assert_eq!(reopened.snapshot.terminal_for_pad(pad), Some(terminal));

    let plain = fixture.dir.path().join("plain.pc");
    save_at(reopened.snapshot.clone(), plain.clone(), None, false, Encoding::Json, || Ok(())).unwrap();
    assert_eq!(read(&plain).unwrap(), json_bytes);
    let baseline = (reopened.snapshot.clone(), reopened.target);
    save(reopened.snapshot.clone(), Some(baseline.clone()), Encoding::Json).unwrap();
    assert_eq!(read(&path).unwrap(), noncanonical);

    fixture.session.open_snapshot(reopened.snapshot);
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    save(fixture.session.snapshot().unwrap(), Some(baseline), Encoding::Json).unwrap();
    let edited = load(&path).unwrap();
    assert_eq!(edited.target.encoding, Encoding::Zip);
    assert_eq!(positions(&edited.snapshot)[0], Point { x_nm: 0, y_nm: STEP });
    assert_connectivity(&inspect(edited.snapshot));
}

/// ZIP validation failures must not adopt an invalid snapshot or mark edits clean.
#[test]
fn failed_zip_opens_preserve_the_active_project_and_dirty_state() {
    let mut fixture = Fixture::open(Encoding::Zip);
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
        rename(b"pcbcraft-container/1", b"pcbcraft-container/9"),
    ] {
        fs::write(&invalid, payload).unwrap();
        let result = load(&invalid).map(Outcome::Opened);
        assert!(matches!(complete(&mut fixture.files, &mut fixture.session, result), Some(Update::Error(_))));
        assert!(Arc::ptr_eq(&before, &fixture.session.snapshot().unwrap()));
        assert_eq!(fixture.session.revision(), revision);
        assert!(fixture.files.dirty(&fixture.session));
    }
}
