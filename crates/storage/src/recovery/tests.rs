use super::*;
use pcbcraft_engine::{DRAW_WIRE, MOVE_SYMBOL, PLACE_SYMBOL, ROTATE_SYMBOL, Session, schematic::GRID_NM};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{BufRead, Seek, SeekFrom},
    process::{Command, Stdio},
};

/// Set in the child process of the kill test to the folder it works in.
const CHILD: &str = "PCBCRAFT_RECOVERY_TEST_CHILD";
const KILL_TEST: &str = "recovery::tests::ten_edits_survive_a_kill";
const READY: &str = "recovery-test-child-ready";
/// Longest wait for the child to journal its edits, and longest the child waits to be killed.
const CHILD_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// A grid point as a command value.
fn at(x: i64, y: i64) -> Value {
    json!({"x_nm": x * GRID_NM, "y_nm": y * GRID_NM})
}

/// The id of the most recently placed symbol; placements are kept in id order.
fn last_symbol(session: &Session) -> String {
    session.project().unwrap().symbols().last().unwrap().id()
}

/// Run ten edits of different kinds through the shared commands: placements, a wire between
/// pins, a move, a rotation, a rename, a board, an undo and a free wire.
fn ten_edits(session: &mut Session) {
    let revision = |session: &Session| json!(session.revision());
    session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision(session)})).unwrap();
    let first = last_symbol(session);
    session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":20 * GRID_NM,"y_nm":0,"expected_revision":revision(session)})).unwrap();
    let second = last_symbol(session);
    session
        .apply(
            DRAW_WIRE,
            &json!({"from":{"symbol":first,"terminal":"2"},"to":{"symbol":second,"terminal":"1"},"bends":[],"expected_revision":revision(session)}),
        )
        .unwrap();
    session.apply(MOVE_SYMBOL, &json!({"symbol":second,"x_nm":20 * GRID_NM,"y_nm":10 * GRID_NM,"expected_revision":revision(session)})).unwrap();
    session.apply(ROTATE_SYMBOL, &json!({"symbol":first,"expected_revision":revision(session)})).unwrap();
    session.apply("project.rename", &json!({"name":"Recovered"})).unwrap();
    session.apply("pcb.create_board", &json!({"expected_revision":revision(session)})).unwrap();
    session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":20 * GRID_NM,"expected_revision":revision(session)})).unwrap();
    session.apply("edit.undo", &json!({})).unwrap();
    session.apply(DRAW_WIRE, &json!({"from":at(-10, -10),"to":at(0, -20),"bends":[at(0, -10)],"expected_revision":revision(session)})).unwrap();
}

/// Every file under `root` with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(tree(&path));
        } else {
            files.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    files
}

/// The child half of the kill test: save a project, make ten unsaved edits with the journal
/// attached, record the expected state, then wait to be killed.
fn child(work: &Path) {
    let recorder = Arc::new(JournalRecorder::start(work.join("recovery"), "kill-test".to_owned()).unwrap());
    let mut session = Session::default();
    session.observe_edits(Some(recorder.clone()));
    session.apply("project.new", &json!({"name":"Kill"})).unwrap();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    crate::save(&work.join("project"), None, &saved, &mut |_| Ok(())).unwrap();
    ten_edits(&mut session);
    recorder.sync().unwrap();
    fs::write(work.join("expected.pc"), encode_packed_documents(encode_documents(session.snapshot().unwrap()).unwrap()).unwrap()).unwrap();
    println!("{READY}");
    // Never outlive a parent that failed to kill this process.
    std::thread::sleep(CHILD_LIMIT);
    std::process::exit(2);
}

/// A process killed after ten unsaved edits leaves a journal that replays, on its snapshot, to
/// exactly the documents the session held; the saved project is untouched and no recovery file
/// is written inside it.
#[test]
fn ten_edits_survive_a_kill() {
    if let Some(work) = std::env::var_os(CHILD) {
        child(Path::new(&work));
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args([KILL_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, work.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // The test harness may print the test name on the same line before the child's output.
    let stdout = process.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let ready = std::io::BufReader::new(stdout).lines().map_while(Result::ok).any(|line| line.contains(READY));
        let _ = sender.send(ready);
    });
    let ready = receiver.recv_timeout(CHILD_LIMIT).unwrap_or(false);
    process.kill().unwrap();
    assert!(!process.wait().unwrap().success());
    assert!(ready, "the child did not journal its edits within {CHILD_LIMIT:?}");

    let project = work.path().join("project");
    let saved_files = tree(&project);
    let recovered = recover(&work.path().join("recovery"), "kill-test").unwrap();
    let expected = decode_packed(&fs::read(work.path().join("expected.pc")).unwrap()).unwrap().documents;
    assert_eq!(recovered.stop, None);
    assert_eq!(recovered.commands.len(), 10);
    assert_eq!(recovered.commands.last().map(String::as_str), Some(DRAW_WIRE));
    assert_eq!(recovered.documents, expected);
    assert_eq!(encode_documents(recovered.project.clone()).unwrap(), expected);
    assert_eq!(recovered.project.name, "Recovered");

    let saved = crate::load(&project).unwrap();
    assert_ne!(saved, expected);
    assert_eq!(decode_documents(&saved).unwrap().name, "Kill");
    assert_eq!(tree(&project), saved_files);
    assert!(saved_files.keys().all(|path| !path.to_string_lossy().contains("kill-test")));
}

/// A journal with the given edits applied on top of a new project, and the documents after each.
fn journal_with(dir: &Path, edits: usize) -> (EditJournal, Vec<Documents>) {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let mut states = vec![encode_documents(session.snapshot().unwrap()).unwrap()];
    let mut journal = EditJournal::create(dir, "entry", states[0].clone()).unwrap();
    for index in 0..edits {
        let x = i64::try_from(index).unwrap() * 10 * GRID_NM;
        let revision =
            session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x,"y_nm":0,"expected_revision":session.revision()})).unwrap();
        let project = session.snapshot().unwrap();
        let documents = encode_documents(project.clone()).unwrap();
        journal.append(&CommittedEdit { command: PLACE_SYMBOL.to_owned(), params: json!({}), revision, project }, documents.clone()).unwrap();
        states.push(documents);
    }
    (journal, states)
}

/// A record cut short by a crash is dropped and the edits before it are kept.
#[test]
fn a_torn_last_record_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, states) = journal_with(dir.path(), 3);
    drop(journal);
    let path = entry_file(dir.path(), "entry", "journal");
    let length = fs::metadata(&path).unwrap().len();
    File::options().write(true).open(&path).unwrap().set_len(length - 40).unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.documents, states[2]);
    assert_eq!(recovered.commands.len(), 2);
    assert!(matches!(recovered.stop, Some(Stop::Unreadable { index: 2, .. })));
}

/// A damaged byte inside a record fails its frame hash; replay keeps the state before it.
#[test]
fn a_damaged_record_ends_replay_at_the_last_good_state() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, states) = journal_with(dir.path(), 3);
    drop(journal);
    let path = entry_file(dir.path(), "entry", "journal");
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 100;
    bytes[last] ^= 0x20;
    fs::write(&path, bytes).unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.documents, states[2]);
    assert!(matches!(recovered.stop, Some(Stop::Unreadable { index: 2, .. })));
}

/// A well-formed record that does not continue the state before it stops replay there, and
/// the records after it are not applied.
#[test]
fn a_record_from_another_state_stops_replay() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, states) = journal_with(dir.path(), 2);
    let edit =
        CommittedEdit { command: "project.rename".to_owned(), params: json!({}), revision: 99, project: decode_documents(&states[0]).unwrap() };
    let foreign =
        frame(&encode_record(&edit, &states[0], &documents_sha256(&states[0]), &states[1], &documents_sha256(&states[1])).unwrap()).unwrap();
    journal.file.seek(SeekFrom::End(0)).unwrap();
    journal.file.write_all(&foreign).unwrap();
    drop(journal);
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.documents, states[2]);
    assert_eq!(recovered.stop, Some(Stop::Mismatch { index: 2, command: "project.rename".to_owned() }));
}

/// A journal left from before a newer snapshot is ignored: the snapshot already holds its edits.
#[test]
fn a_journal_for_an_older_snapshot_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, states) = journal_with(dir.path(), 2);
    let journal_bytes = fs::read(entry_file(dir.path(), "entry", "journal")).unwrap();
    let mut journal = journal;
    journal.rebase(states[2].clone()).unwrap();
    drop(journal);
    fs::write(entry_file(dir.path(), "entry", "journal"), journal_bytes).unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.documents, states[2]);
    assert_eq!(recovered.stop, Some(Stop::OtherSnapshot));
    assert!(recovered.commands.is_empty());
}

/// A journal in a newer format is refused, naming it, rather than half read.
#[test]
fn a_newer_journal_format_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    drop(journal_with(dir.path(), 1));
    fs::write(entry_file(dir.path(), "entry", "journal"), b"pcbcraft-recovery/2\n{}").unwrap();
    let error = recover(dir.path(), "entry").unwrap_err();
    assert!(error.contains("pcbcraft-recovery/2"), "{error}");
}

/// One writer per entry: a second writer is refused while the first holds the lock.
#[test]
fn an_entry_has_one_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, states) = journal_with(dir.path(), 0);
    assert!(EditJournal::create(dir.path(), "entry", states[0].clone()).unwrap_err().contains("in use"));
    drop(journal);
    assert!(EditJournal::create(dir.path(), "entry", states[0].clone()).is_ok());
}

/// Ids that could name a path outside the recovery folder are refused before any I/O.
#[test]
fn entry_ids_are_plain_file_stems() {
    let dir = tempfile::tempdir().unwrap();
    for id in ["", "../escape", "a/b", "a.b", &"x".repeat(65)] {
        assert!(EditJournal::create(dir.path(), id, Documents::new()).is_err());
        assert!(recover(dir.path(), id).is_err());
    }
    assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let project = session.project().unwrap();
    let (first, second) = (new_entry_id(project), new_entry_id(project));
    assert_ne!(first, second);
    assert!(valid_id(&first) && first.starts_with(&project.id().to_string()));
}

/// A reopened project starts a new snapshot; edits after it replay on that snapshot.
#[test]
fn an_open_replaces_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Arc::new(JournalRecorder::start(dir.path().to_path_buf(), "entry".to_owned()).unwrap());
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Before"})).unwrap();
    session.observe_edits(Some(recorder.clone()));
    session.apply("project.rename", &json!({"name":"Edited"})).unwrap();
    let mut other = Session::default();
    other.apply("project.new", &json!({"name":"Opened"})).unwrap();
    session.open_snapshot(other.snapshot().unwrap());
    session.apply("project.rename", &json!({"name":"After"})).unwrap();
    session.observe_edits(None);
    Arc::into_inner(recorder).unwrap().finish().unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.commands, ["project.rename"]);
    assert_eq!(recovered.project.name, "After");
    assert_eq!(recovered.project.id(), other.project().unwrap().id());
}
