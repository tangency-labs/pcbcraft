//! Native storage acceptance tests. The operating system picker is bypassed
//! by supplying its selected path to the same file worker code.

use std::{
    process::Command,
    sync::{Barrier, mpsc},
};

use pcbcraft_engine::{MOVE_SYMBOL, Session, persistence::MAX_PROJECT_BYTES, schematic::Point};
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
    fn open() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two-resistors.pc");
        let original = decode(TWO_RESISTORS).unwrap();
        let result = save_at(original, path.clone(), None, false, || Ok(()));
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
        let result =
            save_at(snapshot.clone(), self.path.clone(), Some(saved.target.bytes.clone()), Arc::ptr_eq(&snapshot, &saved.snapshot), after_staging);
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
    let mut fixture = Fixture::open();
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
    assert!(save_at(snapshot, fixture.path.clone(), None, false, || Ok(())).is_err());
    assert_eq!(read(&fixture.path).unwrap(), noncanonical);
}

/// Two independent symbol moves change one record each and merge with Git's normal three-way driver.
#[test]
fn independent_placement_edits_merge_and_reopen() {
    let mut fixture = Fixture::open();
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
    let result = save_at(branch.snapshot().unwrap(), other.clone(), Some(loaded.target.bytes), false, || Ok(()));
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
    let mut fixture = Fixture::open();
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
    let mut fixture = Fixture::open();
    move_symbol(&mut fixture.session, "symbol-2", 0, STEP);
    let before = fixture.session.snapshot().unwrap();
    let update = fixture.save(|| Err("simulated failure before publication".to_owned()));
    assert!(matches!(update, Some(Update::Error(message)) if message == "simulated failure before publication"));
    assert_eq!(read(&fixture.path).unwrap(), TWO_RESISTORS);
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

/// Competing writes from the same baseline leave only the winning session clean.
#[test]
fn one_writer_publishes_against_a_shared_baseline() {
    let mut first = Fixture::open();
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
        save_at(snapshot, first.path.clone(), Some(baseline.clone()), false, || Ok(()))
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
    assert_eq!(read(&first.path).unwrap(), encode(winner).unwrap());
}

/// Pause the real worker after staging R1, edit R2, then verify which snapshot reached disk.
#[test]
fn a_late_move_is_not_marked_saved() {
    let mut fixture = Fixture::open();
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
            let result = save_at(staged, path, Some(baseline), false, || {
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
