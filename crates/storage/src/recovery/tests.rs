use super::{faults::Fault, *};
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

/// The child half of the kill test: save a project and its file baseline, then make ten unsaved
/// edits with the journal attached, record the expected state and wait to be killed.
fn child(work: &Path) {
    let recorder = Arc::new(JournalRecorder::start(work.join("recovery"), "kill-test".to_owned()).unwrap());
    let mut session = Session::default();
    session.observe_edits(Some(recorder.clone()));
    session.apply("project.new", &json!({"name":"Kill"})).unwrap();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    crate::save(&work.join("project"), None, &saved, &mut |_| Ok(())).unwrap();
    fs::write(work.join("saved-files.json"), serde_json::to_vec(&tree(&work.join("project"))).unwrap()).unwrap();
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
    let saved_files: BTreeMap<PathBuf, Vec<u8>> = serde_json::from_slice(&fs::read(work.path().join("saved-files.json")).unwrap()).unwrap();
    assert_eq!(tree(&project), saved_files);
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

/// Tokenized board setup Apply recovers its document changes without recording spent authority.
#[test]
fn setup_preview_token_does_not_enter_recovery_records() {
    use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, PREVIEW_BOARD_SETUP, SET_BOARD_SETUP};
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":session.revision()})).unwrap();
    let setup = session.project().unwrap().board_setup().unwrap().with_copper_count(4).unwrap();
    let revision = session.revision();
    let recorder = Arc::new(JournalRecorder::start(dir.path().to_owned(), "entry".to_owned()).unwrap());
    session.observe_edits(Some(recorder.clone()));
    let report = session.execute(PREVIEW_BOARD_SETUP, &json!({"setup":setup,"expected_revision":revision})).unwrap();
    let token = report["board_setup_preview"]["plan_token"].as_str().unwrap();
    session.apply(SET_BOARD_SETUP, &json!({"setup":setup,"plan_token":token,"expected_revision":revision})).unwrap();
    session.observe_edits(None);
    Arc::try_unwrap(recorder).unwrap().finish().unwrap();

    let bytes = fs::read(entry_file(dir.path(), "entry", "journal")).unwrap();
    let mut frames = bytes.strip_prefix(MAGIC).unwrap();
    read_frame(&mut frames).unwrap().unwrap();
    let (record, _) = decode_record(&read_frame(&mut frames).unwrap().unwrap()).unwrap();
    assert_eq!(record.command, SET_BOARD_SETUP);
    assert_eq!(record.params, json!({"setup":setup,"expected_revision":revision}));
    assert!(read_frame(&mut frames).unwrap().is_none());
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.stop, None);
    assert_eq!(recovered.documents, encode_documents(session.snapshot().unwrap()).unwrap());
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
    let file = journal.file.as_mut().unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(&foreign).unwrap();
    drop(journal);
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.documents, states[2]);
    assert_eq!(recovered.stop, Some(Stop::Mismatch { index: 2, command: "project.rename".to_owned() }));
}

/// Append the bytes `extra` makes from the states of a two-edit journal, and recover it.
fn recover_with_appended(extra: impl FnOnce(&[Documents]) -> Vec<u8>) -> (Recovered, Vec<Documents>) {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, states) = journal_with(dir.path(), 2);
    let file = journal.file.as_mut().unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(&extra(&states)).unwrap();
    drop(journal);
    (recover(dir.path(), "entry").unwrap(), states)
}

/// A record that writes or removes a path outside the project's documents is unreadable,
/// even with intact framing and hashes that continue the state.
#[test]
fn a_record_path_outside_the_project_is_unreadable() {
    for outside_written in [true, false] {
        let (recovered, states) = recover_with_appended(|states| {
            let inside = states[2].keys().next().unwrap().clone();
            let (written, removed) = if outside_written { ("../x".to_owned(), vec![]) } else { (inside, vec!["../x".to_owned()]) };
            let before = documents_sha256(&states[2]);
            let record = Record {
                command: "project.rename".to_owned(),
                version: COMMAND_VERSION,
                params: json!({}),
                revision: 99,
                before_sha256: before.clone(),
                after_sha256: before,
                written: vec![Written { path: written, size: 1 }],
                removed,
            };
            let meta = serde_json::to_vec(&record).unwrap();
            let mut payload = u32::try_from(meta.len()).unwrap().to_le_bytes().to_vec();
            payload.extend_from_slice(&meta);
            payload.push(b'x');
            frame(&payload).unwrap()
        });
        assert_eq!(recovered.documents, states[2]);
        match recovered.stop {
            Some(Stop::Unreadable { index: 2, reason }) => assert!(reason.starts_with("../x:"), "{reason}"),
            stop => panic!("outside path in written = {outside_written}: {stop:?}"),
        }
    }
}

/// A frame whose length is above the limit is unreadable without allocating that length.
#[test]
fn an_oversized_frame_is_unreadable() {
    let length = u32::try_from(MAX_FRAME_BYTES + 1).unwrap();
    let (recovered, states) = recover_with_appended(|_| length.to_le_bytes().to_vec());
    assert_eq!(recovered.documents, states[2]);
    assert_eq!(recovered.stop, Some(Stop::Unreadable { index: 2, reason: "record exceeds its size limit".to_owned() }));
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
    fs::write(entry_file(dir.path(), "entry", "journal"), b"pcbcraft-recovery/3\n{}").unwrap();
    let error = recover(dir.path(), "entry").unwrap_err();
    assert!(error.contains("pcbcraft-recovery/3"), "{error}");
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

/// Ids that could name a path outside the recovery folder, differ from another only in case, or
/// name a Windows device are refused before any I/O.
#[test]
fn entry_ids_are_plain_file_stems() {
    let dir = tempfile::tempdir().unwrap();
    for id in ["", "../escape", "a/b", "a.b", "a\\b", "Entry", "con", "nul", "com1", "lpt9", &"x".repeat(65)] {
        let refusal = format!("`{id}` is not a recovery entry id.");
        assert_eq!(EditJournal::create(dir.path(), id, Documents::new()).unwrap_err(), refusal);
        assert_eq!(recover(dir.path(), id).unwrap_err(), refusal);
    }
    assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let longest = "x".repeat(MAX_ID_LEN);
    drop(EditJournal::create(dir.path(), &longest, encode_documents(session.snapshot().unwrap()).unwrap()).unwrap());
    assert!(recover(dir.path(), &longest).unwrap().stop.is_none());
    assert!(valid_id("console") && valid_id("com") && valid_id("com10"));
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

/// A one-segment route prepared and committed outside `Session::apply`, on a fresh board with
/// copper layers. Returns the committed revision.
fn commit_route(session: &mut Session) -> u64 {
    use pcbcraft_engine::board::setup::LayerPurpose;
    session.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":session.revision()})).unwrap();
    let project = session.project().unwrap();
    let view = project.board_view(project.board_target().unwrap()).unwrap();
    let layer = view.setup().unwrap().layers().iter().find(|layer| layer.purpose == LayerPurpose::Copper).unwrap().id;
    let params = json!({"board":view.target.board,"layer":layer,"width_nm":250_000,
        "points":[{"x_nm":-20_000_000,"y_nm":-20_000_000},{"x_nm":-10_000_000,"y_nm":-20_000_000}],"expected_revision":session.revision()});
    let mut job = session.prepare_copper_edit(pcbcraft_engine::ADD_TRACKS, &params).unwrap();
    while !job.step(1024) {}
    let prepared = job.take_result().unwrap().unwrap();
    session.commit_copper_edit(prepared).unwrap()
}

/// Changes made outside `Session::apply` reach the journal: a prepared copper edit as its
/// command, and save metadata adopted after a save as a new snapshot. Edits after each still
/// replay, so nothing later is lost to a mismatch.
#[test]
fn copper_commits_and_save_adoption_are_journaled() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Arc::new(JournalRecorder::start(dir.path().to_path_buf(), "entry".to_owned()).unwrap());
    let mut session = Session::default();
    session.observe_edits(Some(recorder.clone()));
    session.apply("project.new", &json!({"name":"Routed","create_board":true})).unwrap();
    commit_route(&mut session);
    session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    recorder.sync().unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.stop, None);
    assert_eq!(recovered.commands[recovered.commands.len() - 2..], [pcbcraft_engine::ADD_TRACKS, PLACE_SYMBOL]);
    assert_eq!(recovered.documents, encode_documents(session.snapshot().unwrap()).unwrap());

    let original = session.snapshot().unwrap();
    let mut saved = Project::clone(&original);
    saved.name = "Saved As".to_owned();
    let saved = Arc::new(saved);
    assert!(session.adopt_saved_metadata(&original, &saved));
    session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":20 * GRID_NM,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    session.observe_edits(None);
    Arc::into_inner(recorder).unwrap().finish().unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.stop, None);
    assert_eq!(recovered.commands, [PLACE_SYMBOL]);
    assert_eq!(recovered.project.name, "Saved As");
    assert_eq!(recovered.documents, encode_documents(session.snapshot().unwrap()).unwrap());
}

/// Place one more resistor and record it, returning the documents after it and the append result.
fn place_and_append(session: &mut Session, journal: &mut EditJournal) -> (Documents, Result<(), String>) {
    let x = i64::try_from(session.project().unwrap().symbols().len()).unwrap() * 10 * GRID_NM;
    let revision = session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let project = session.snapshot().unwrap();
    let documents = encode_documents(project.clone()).unwrap();
    let result = journal.append(&CommittedEdit { command: PLACE_SYMBOL.to_owned(), params: json!({}), revision, project }, documents.clone());
    (documents, result)
}

/// A new journal on a new project, with the size limit set to `limit` bytes past its header.
fn limited_journal(dir: &Path, limit: u64) -> (Session, EditJournal) {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let mut journal = EditJournal::create(dir, "entry", encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    journal.floor = 0;
    journal.rebase_after = journal.journal_bytes + limit;
    (session, journal)
}

/// The size of the entry's journal file.
fn journal_len(dir: &Path) -> u64 {
    fs::metadata(entry_file(dir, "entry", "journal")).unwrap().len()
}

/// The journal has a size limit: the edit that takes it past the limit becomes a new snapshot
/// with an empty journal, and the recovered state is the same before and after the switch.
#[test]
fn crossing_the_size_limit_starts_a_new_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
    let header = journal_len(dir.path());
    let (first, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let before = recover(dir.path(), "entry").unwrap();
    assert_eq!((before.documents, before.commands.len()), (first.clone(), 1));
    assert!(journal_len(dir.path()) > header);

    journal.rebase_after = journal.journal_bytes;
    let (second, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let after = recover(dir.path(), "entry").unwrap();
    assert_eq!((after.documents, after.commands.len(), after.stop), (second, 0, None));
    assert_eq!(journal_len(dir.path()), journal.journal_bytes);
    assert!(journal.rebase_after >= 2 * fs::metadata(entry_file(dir.path(), "entry", "pc")).unwrap().len());

    let (third, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len()), (third, 1));
    assert_eq!(rebase_limit(10, REBASE_FLOOR_BYTES), REBASE_FLOOR_BYTES);
    assert_eq!(rebase_limit(REBASE_FLOOR_BYTES, REBASE_FLOOR_BYTES), REBASE_RATIO * REBASE_FLOOR_BYTES);
}

/// A snapshot that cannot be written at the limit leaves the old snapshot and journal valid,
/// holding the edit that crossed it; the count is kept, so the next edit tries again.
#[test]
fn a_failed_snapshot_at_the_limit_keeps_the_old_pair() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
    place_and_append(&mut session, &mut journal).1.unwrap();
    let snapshot = fs::read(entry_file(dir.path(), "entry", "pc")).unwrap();
    journal.rebase_after = journal.journal_bytes;
    // Publishing the new snapshot fails before its rename; the open journal is still appended to.
    faults::arm(dir.path(), Fault::Publish, 1);
    let (second, result) = place_and_append(&mut session, &mut journal);
    assert!(result.unwrap_err().contains("could not be renewed: The recovery file could not be staged."));
    assert_eq!(fs::read(entry_file(dir.path(), "entry", "pc")).unwrap(), snapshot);
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (second, 2, None));

    let (third, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len()), (third, 0));
    assert_ne!(fs::read(entry_file(dir.path(), "entry", "pc")).unwrap(), snapshot);
}

/// A kill after the new snapshot is published at the limit, before its journal, leaves the old
/// journal naming the old snapshot. Recovery ignores that journal and returns the new snapshot,
/// which holds every recorded edit.
#[test]
fn a_kill_between_the_new_snapshot_and_its_journal_recovers_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
    place_and_append(&mut session, &mut journal).1.unwrap();
    place_and_append(&mut session, &mut journal).1.unwrap();
    let old_journal = fs::read(entry_file(dir.path(), "entry", "journal")).unwrap();
    journal.rebase_after = journal.journal_bytes;
    let (third, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    drop(journal);
    // The old journal as it stood before the crossing edit; at the kill it would also hold that
    // edit's record, and recovery ignores the whole file either way.
    fs::write(entry_file(dir.path(), "entry", "journal"), old_journal).unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.documents, third);
    assert_eq!(recovered.stop, Some(Stop::OtherSnapshot));
    assert!(recovered.commands.is_empty());
}

/// Seconds on the schedule's clock.
fn secs(seconds: u64) -> std::time::Duration {
    std::time::Duration::from_secs(seconds)
}

/// Nothing is due without a change. A change is due one interval after it once editing pauses,
/// a burst of edits defers it at most to a second interval, and Off waits for a long operation.
#[test]
fn a_schedule_waits_for_a_change_an_interval_and_a_pause() {
    let mut schedule = Schedule::new(1);
    assert!(!schedule.due(secs(3600)));
    assert_eq!(schedule.wait(secs(0)), None);

    schedule.changed(secs(10));
    assert!(!schedule.due(secs(69)));
    assert_eq!(schedule.wait(secs(30)), Some(secs(40)));
    assert!(schedule.due(secs(70)));
    schedule.changed(secs(69));
    assert!(!schedule.due(secs(70)), "an edit just now holds it for the quiet period");
    assert!(schedule.due(secs(71)));
    schedule.taken();
    assert!(!schedule.due(secs(500)));

    schedule.changed(secs(100));
    for second in 101..160 {
        schedule.changed(secs(second));
    }
    assert!(!schedule.due(secs(160)));
    for second in 160..220 {
        schedule.changed(secs(second));
    }
    assert!(schedule.due(secs(220)), "a burst never delays it past a second interval");

    let mut off = Schedule::new(0);
    off.changed(secs(0));
    assert!(!off.due(secs(86_400)));
    assert_eq!(off.wait(secs(5)), None);
    off.long_operation(LONG_OPERATION - std::time::Duration::from_millis(1), secs(6));
    assert!(!off.due(secs(86_400)));
    off.long_operation(LONG_OPERATION, secs(7));
    assert!(off.due(secs(7)));
    assert_eq!(off.wait(secs(7)), Some(std::time::Duration::ZERO));
    off.taken();
    assert!(!off.due(secs(8)));
    assert_eq!(off.wait(secs(8)), None);

    let mut five = Schedule::new(5);
    five.changed(secs(0));
    assert!(!five.due(secs(299)) && five.due(secs(300)));
    five.set_minutes(10);
    assert!(!five.due(secs(300)) && five.due(secs(600)));
}

/// While the journal remains intact, a snapshot replaces the base and empties the journal
/// when the state differs; asking again with the same state writes nothing.
#[test]
fn an_intact_journal_skips_an_unchanged_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, states) = journal_with(dir.path(), 2);
    assert!(journal.snapshot(states[2].clone()).unwrap(), "the journal contains edits absent from the snapshot");
    let snapshot = fs::read(entry_file(dir.path(), "entry", "pc")).unwrap();
    assert_eq!(decode_snapshot(&snapshot).unwrap().1, states[2]);
    let written = tree(dir.path());
    assert!(!journal.snapshot(states[2].clone()).unwrap());
    assert_eq!(tree(dir.path()), written, "an unchanged snapshot writes nothing");
    drop(journal);
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.stop), (states[2].clone(), None));
}

/// An intact journal is not grounds to skip a snapshot if it records a different result,
/// such as when an intervening undo to the base was not journaled.
#[test]
fn a_snapshot_can_restore_the_base_when_the_journal_records_later_edits() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, states) = journal_with(dir.path(), 2);
    assert_eq!(recover(dir.path(), "entry").unwrap().documents, states[2]);
    assert!(journal.snapshot(states[0].clone()).unwrap());
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.documents, states[0]);
    assert!(recovered.commands.is_empty());
    assert_eq!(recovered.stop, None);
}

/// A recorder's snapshot holds the session's current state with an empty journal after it, the
/// saved project is untouched, and discarding the recorder deletes the entry.
#[test]
fn a_recorder_snapshot_holds_the_current_state() {
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("recovery");
    let recorder = Arc::new(JournalRecorder::start(dir.clone(), "entry".to_owned()).unwrap());
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Saved"})).unwrap();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    crate::save(&work.path().join("project"), None, &saved, &mut |_| Ok(())).unwrap();
    let saved_files = tree(&work.path().join("project"));
    session.observe_edits(Some(recorder.clone()));
    ten_edits(&mut session);
    recorder.snapshot(session.snapshot().unwrap());
    recorder.sync().unwrap();
    let current = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(decode_snapshot(&fs::read(entry_file(&dir, "entry", "pc")).unwrap()).unwrap().1, current);
    let recovered = recover(&dir, "entry").unwrap();
    assert!(recovered.commands.is_empty());
    assert_eq!((recovered.documents, recovered.stop), (current, None));

    let written = tree(&dir);
    recorder.snapshot(session.snapshot().unwrap());
    recorder.sync().unwrap();
    assert_eq!(tree(&dir), written, "nothing changed since the last snapshot");
    assert_eq!(tree(&work.path().join("project")), saved_files);

    session.observe_edits(None);
    assert!(remove_entry(&dir, "entry").unwrap_err().contains("in use"), "a running session's entry stays");
    Arc::into_inner(recorder).unwrap().discard().unwrap();
    let left: Vec<_> = fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from(".entries.lock")], "only the folder gate stays after discarding");
    assert!(remove_entry(&dir, "entry").is_ok());
    assert!(remove_entry(&dir, "../entry").unwrap_err().contains("is not a recovery entry id"));
}

/// Entry creation and removal both respect the folder gate, so neither can open an old entry
/// lock while another operation closes and removes its path. Releasing the gate permits both.
#[test]
fn the_folder_gate_excludes_opening_and_discarding_entries() {
    let dir = tempfile::tempdir().unwrap();
    let (_, states) = journal_with(dir.path(), 0);
    let before = tree(dir.path());
    let gate = entry_gate(dir.path()).unwrap();
    assert_eq!(EditJournal::create(dir.path(), "entry", states[0].clone()).unwrap_err(), "Recovery entries are being opened or removed.");
    assert_eq!(remove_entry(dir.path(), "entry").unwrap_err(), "Recovery entries are being opened or removed.");
    assert_eq!(tree(dir.path()), before);
    drop(gate);
    let journal = EditJournal::create(dir.path(), "entry", states[0].clone()).unwrap();
    assert!(remove_entry(dir.path(), "entry").unwrap_err().contains("in use by another session"));
    drop(journal);
    remove_entry(dir.path(), "entry").unwrap();
    let journal = EditJournal::create(dir.path(), "entry", states[0].clone()).unwrap();
    assert!(EditJournal::create(dir.path(), "entry", states[0].clone()).unwrap_err().contains("in use by another session"));
    drop(journal);
    remove_entry(dir.path(), "entry").unwrap();
}

/// A recorder with no successful claim must not remove an entry from an earlier run, even
/// when it attempted to acquire the name while that entry's owner was live.
#[test]
fn discarding_without_ownership_preserves_an_existing_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (owner, states) = journal_with(dir.path(), 1);
    let unstarted = JournalRecorder::start(dir.path().to_path_buf(), "entry".into()).unwrap();
    unstarted.discard().unwrap();
    let contender = JournalRecorder::start(dir.path().to_path_buf(), "entry".into()).unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Contender"})).unwrap();
    contender.snapshot(session.snapshot().unwrap());
    assert!(contender.sync().unwrap_err().contains("already exists"));
    drop(owner);
    contender.discard().unwrap();
    assert_eq!(recover(dir.path(), "entry").unwrap().documents, states[1]);
}

/// After the drain deadline, cleanup cannot start. Expired retirement leaves readable data.
#[test]
fn expired_retirement_preserves_the_owned_entry() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = JournalRecorder::start(dir.path().to_path_buf(), "entry".into()).unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Keep"})).unwrap();
    recorder.snapshot(session.snapshot().unwrap());
    recorder.retire(Some(Instant::now())).wait(Duration::from_secs(2)).unwrap();
    assert_eq!(recover(dir.path(), "entry").unwrap().project.name, "Keep");
}

/// A stalled consumer retains bounded work, but the newest state and queued barrier survive.
#[test]
fn the_mailbox_coalesces_a_burst_behind_a_barrier() {
    let mailbox = Mailbox::default();
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"First"})).unwrap();
    let first = session.snapshot().unwrap();
    mailbox.push(Message::Opened(first));
    let (barrier, _done) = mpsc::sync_channel(1);
    mailbox.push(Message::Sync(barrier));
    for _ in 0..(PENDING_LIMIT * 3) {
        mailbox.push(Message::Snapshot(session.snapshot().unwrap()));
    }
    session.apply("project.rename", &json!({"name":"Latest"})).unwrap();
    mailbox.push(Message::Snapshot(session.snapshot().unwrap()));
    let pending = mailbox.state.lock().unwrap();
    assert!(pending.queue.len() <= PENDING_LIMIT + 2);
    assert!(matches!(pending.queue.get(1), Some(Message::Sync(_))));
    assert!(matches!(pending.queue.back(), Some(Message::Snapshot(project) | Message::Opened(project)) if project.name == "Latest"));
}

/// With coalescing, a run of queued edits reaches the worker as its newest edit only; a barrier
/// still splits runs. Without it, every edit is handed over in order.
#[test]
fn coalescing_hands_the_worker_only_the_newest_queued_edit() {
    for coalesce in [false, true] {
        let mailbox = Mailbox { coalesce, ..Mailbox::default() };
        let mut session = Session::default();
        session.apply("project.new", &json!({})).unwrap();
        let mut revisions = Vec::new();
        let mut edits = Vec::new();
        for index in 0..4 {
            let revision = session
                .apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index * 10 * GRID_NM,"y_nm":0,"expected_revision":session.revision()}))
                .unwrap();
            revisions.push(revision);
            edits.push(CommittedEdit { command: PLACE_SYMBOL.to_owned(), params: json!({}), revision, project: session.snapshot().unwrap() });
        }
        let last = edits.pop().unwrap();
        for edit in edits {
            mailbox.push(Message::Committed(edit));
        }
        let (barrier, _done) = mpsc::sync_channel(1);
        mailbox.push(Message::Sync(barrier));
        mailbox.push(Message::Committed(last));
        mailbox.close(None);
        let mut handed = Vec::new();
        while let Ok(message) = mailbox.next() {
            handed.push(match message {
                Message::Committed(edit) => Some(edit.revision),
                Message::Sync(_) => None,
                Message::Opened(_) | Message::Snapshot(_) => panic!("only edits and a barrier were queued"),
            });
        }
        let expected = if coalesce {
            vec![Some(revisions[2]), None, Some(revisions[3])]
        } else {
            vec![Some(revisions[0]), Some(revisions[1]), Some(revisions[2]), None, Some(revisions[3])]
        };
        assert_eq!(handed, expected, "coalesce {coalesce}");
    }
}

/// With a stalled worker above `PENDING_LIMIT`, a default (non-coalescing) recorder retains
/// every queued edit and recovers all individual command records; an opted-in coalescing mailbox
/// still compacts a burst above `PENDING_LIMIT`.
#[test]
fn non_coalescing_recorder_keeps_all_edits_above_pending_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::default();
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"AllEdits"})).unwrap();
    mailbox.push(Message::Opened(session.snapshot().unwrap()));
    let count = PENDING_LIMIT + 5;
    let mut edits = Vec::with_capacity(count);
    for index in 0..count {
        let name = format!("Edit {index}");
        let revision = session.apply("project.rename", &json!({"name": name})).unwrap();
        let edit =
            CommittedEdit { command: "project.rename".to_owned(), params: json!({"name": name}), revision, project: session.snapshot().unwrap() };
        edits.push(edit.clone());
        mailbox.push(Message::Committed(edit));
    }
    assert_eq!(mailbox.state.lock().unwrap().queue.len(), count + 1);
    mailbox.close(None);
    record(dir.path(), "entry", &mailbox, &|| {}, None).unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.stop, None);
    assert_eq!(recovered.commands, vec!["project.rename".to_owned(); count]);
    assert_eq!(recovered.project.name, format!("Edit {}", count - 1));
    assert_eq!(recovered.documents, encode_documents(session.snapshot().unwrap()).unwrap());

    let coalescing = Mailbox { coalesce: true, ..Mailbox::default() };
    for edit in edits {
        coalescing.push(Message::Committed(edit));
    }
    let pending = coalescing.state.lock().unwrap();
    assert!(pending.queue.len() <= PENDING_LIMIT);
    assert!(matches!(pending.queue.front(), Some(Message::Opened(_))));
}

/// A coalescing recorder recovers exactly the session's documents.
#[test]
fn a_coalescing_recorder_recovers_the_current_state() {
    let dir = tempfile::tempdir().unwrap();
    let options = RecorderOptions { coalesce_edits: true, ..RecorderOptions::default() };
    let recorder = Arc::new(JournalRecorder::start_with(dir.path().to_path_buf(), "entry".into(), options, || {}).unwrap());
    let mut session = Session::default();
    session.observe_edits(Some(recorder.clone()));
    session.apply("project.new", &json!({"name":"Coalesced"})).unwrap();
    ten_edits(&mut session);
    recorder.sync().unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.stop, None);
    assert!((1..=10).contains(&recovered.commands.len()));
    assert_eq!(recovered.documents, encode_documents(session.snapshot().unwrap()).unwrap());
    session.observe_edits(None);
    Arc::into_inner(recorder).unwrap().discard().unwrap();
}

/// Mark every file of entry `id` as last written `age` ago.
fn age_entry(dir: &Path, id: &str, age: Duration) {
    let when = std::time::SystemTime::now() - age;
    for ext in ["pc", "journal", "lock"] {
        if let Ok(file) = File::options().write(true).open(entry_file(dir, id, ext)) {
            file.set_modified(when).unwrap();
        }
    }
}

/// The entry ids with any file in `dir`.
fn entry_ids(dir: &Path) -> std::collections::BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.unwrap().file_name().into_string().unwrap().split_once('.').map(|(id, _)| id.to_owned()))
        .filter(|id| !id.is_empty())
        .collect()
}

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Pruning removes unheld entries past the age limit, then all but the newest few; an entry a
/// running session holds stays however old it is, and folder-gate contention removes nothing.
#[test]
fn pruning_keeps_held_entries_and_bounds_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let held = EditJournal::create(dir.path(), "held", documents.clone()).unwrap();
    age_entry(dir.path(), "held", DAY * 30);
    for (id, days) in [("old", 30), ("newest", 0), ("newer", 1), ("new", 2)] {
        drop(EditJournal::create(dir.path(), id, documents.clone()).unwrap());
        age_entry(dir.path(), id, DAY * days + Duration::from_secs(60));
    }
    drop(EditJournal::create(dir.path(), "orphan", documents.clone()).unwrap());
    fs::remove_file(entry_file(dir.path(), "orphan", "lock")).unwrap();
    fs::remove_file(entry_file(dir.path(), "orphan", "journal")).unwrap();
    age_entry(dir.path(), "orphan", DAY * 30);
    let retention = Retention { max_age: DAY * 14, max_entries: 2 };

    let before = tree(dir.path());
    let gate = entry_gate(dir.path()).unwrap();
    assert_eq!(prune(dir.path(), retention), Ok(0));
    drop(gate);
    assert_eq!(tree(dir.path()), before, "a busy folder is left alone");

    assert_eq!(prune(dir.path(), retention), Ok(3));
    assert_eq!(entry_ids(dir.path()), std::collections::BTreeSet::from(["held", "newer", "newest"].map(str::to_owned)));
    assert_eq!(recover(dir.path(), "newest").unwrap().documents, documents);
    drop(held);
    assert_eq!(prune(dir.path(), retention), Ok(1), "released, the old entry goes");
    assert_eq!(entry_ids(dir.path()), std::collections::BTreeSet::from(["newer", "newest"].map(str::to_owned)));
    assert_eq!(prune(&dir.path().join("missing"), retention), Ok(0));
}

/// Even when the scan cap stops before visiting an entry's newest file, retention inspects all
/// three known paths for each discovered ID so a recently updated entry is not treated as expired.
/// An entry whose age cannot be determined due to a metadata error is also left in place.
#[test]
fn pruning_reads_complete_entry_times_under_scan_cap() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, states) = journal_with(dir.path(), 1);
    drop(journal);

    // Discover the filesystem's directory iteration order for the three files of "entry",
    // age the first two past max_age, keep the third fresh, and cap the scan before the third.
    let mut positions: Vec<(usize, String)> = fs::read_dir(dir.path())
        .unwrap()
        .enumerate()
        .filter_map(|(index, entry)| {
            let name = entry.unwrap().file_name().into_string().ok()?;
            let (id, ext) = name.rsplit_once('.')?;
            (id == "entry" && matches!(ext, "pc" | "journal" | "lock")).then(|| (index + 1, ext.to_owned()))
        })
        .collect();
    assert_eq!(positions.len(), 3);
    let old_time = std::time::SystemTime::now() - DAY * 20;
    for (_, ext) in &positions[..2] {
        File::options().write(true).open(entry_file(dir.path(), "entry", ext)).unwrap().set_modified(old_time).unwrap();
    }
    let fresh_ext = positions.pop().unwrap().1;
    let cutoff = positions.last().unwrap().0;
    let retention = Retention { max_age: DAY * 14, max_entries: 16 };

    assert_eq!(prune_with_scan_limit(dir.path(), retention, cutoff), Ok(0));
    assert!(entry_file(dir.path(), "entry", &fresh_ext).exists());
    assert_eq!(recover(dir.path(), "entry").unwrap().documents, states[1]);

    // If a metadata error prevents determining an entry's age, leave the entry in place.
    #[cfg(unix)]
    {
        drop(EditJournal::create(dir.path(), "unreadable", states[1].clone()).unwrap());
        fs::remove_file(entry_file(dir.path(), "unreadable", "journal")).unwrap();
        fs::remove_file(entry_file(dir.path(), "unreadable", "lock")).unwrap();
        File::options().write(true).open(entry_file(dir.path(), "unreadable", "pc")).unwrap().set_modified(old_time).unwrap();
        std::os::unix::fs::symlink("unreadable.journal", entry_file(dir.path(), "unreadable", "journal")).unwrap();
        assert_eq!(prune(dir.path(), retention), Ok(0));
        assert!(entry_file(dir.path(), "unreadable", "pc").exists());
    }
}

/// Retention verifies recovery-entry headers before applying age or count limits, so an ordinary
/// saved `.pc` project archive in the same directory is never pruned while expired recovery
/// entries are removed.
#[test]
fn pruning_ignores_ordinary_project_archives() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Client Board"})).unwrap();
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();

    // Place a valid ordinary `.pc` project archive aged 15 days in the past, with no journal/lock.
    let ordinary_path = dir.path().join("client-board.pc");
    let ordinary_bytes = pcbcraft_engine::persistence::encode_packed_documents(documents.clone()).unwrap();
    fs::write(&ordinary_path, &ordinary_bytes).unwrap();
    let old_time = std::time::SystemTime::now() - DAY * 15;
    File::options().write(true).open(&ordinary_path).unwrap().set_modified(old_time).unwrap();

    // Place a genuine expired recovery entry alongside it.
    drop(EditJournal::create(dir.path(), "expired-session", documents).unwrap());
    age_entry(dir.path(), "expired-session", DAY * 15);

    let options = RecorderOptions { retention: Some(Retention { max_age: DAY * 14, max_entries: 16 }), ..RecorderOptions::default() };
    let recorder = JournalRecorder::start_with(dir.path().to_path_buf(), "fresh".into(), options, || {}).unwrap();
    recorder.sync().unwrap();

    assert!(ordinary_path.exists(), "ordinary project archive must not be deleted");
    assert_eq!(fs::read(&ordinary_path).unwrap(), ordinary_bytes, "ordinary project archive must remain byte-identical");
    for ext in ["pc", "journal", "lock"] {
        assert!(!entry_file(dir.path(), "expired-session", ext).exists(), "expired recovery .{ext} should be pruned");
    }
    recorder.finish().unwrap();
}

/// A recorder started with a retention prunes earlier runs' entries before its first barrier
/// returns; the default recorder prunes nothing.
#[test]
fn a_recorder_with_retention_prunes_before_recording() {
    for retention in [None, Some(Retention { max_age: DAY * 14, max_entries: 16 })] {
        let dir = tempfile::tempdir().unwrap();
        let (journal, _) = journal_with(dir.path(), 1);
        drop(journal);
        age_entry(dir.path(), "entry", DAY * 30);
        let options = RecorderOptions { retention, ..RecorderOptions::default() };
        let recorder = JournalRecorder::start_with(dir.path().to_path_buf(), "fresh".into(), options, || {}).unwrap();
        recorder.sync().unwrap();
        assert_eq!(entry_file(dir.path(), "entry", "pc").exists(), retention.is_none(), "{retention:?}");
        recorder.finish().unwrap();
    }
}

/// A timed snapshot's post-rename flush error retains the published state and is not mistaken
/// for a successful no-op on retry, whether the snapshot or journal flush failed.
#[test]
fn a_timed_snapshot_retries_after_a_post_rename_flush_failure() {
    for nth in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, states) = journal_with(dir.path(), 2);
        faults::arm(dir.path(), Fault::Flush, nth);
        assert!(journal.snapshot(states[2].clone()).unwrap_err().contains("could not be flushed"));
        assert!(journal.file.is_none());
        assert_eq!(recover(dir.path(), "entry").unwrap().documents, states[2]);
        assert!(journal.snapshot(states[2].clone()).unwrap());
        let recovered = recover(dir.path(), "entry").unwrap();
        assert_eq!((recovered.documents, recovered.stop, recovered.commands), (states[2].clone(), None, vec![]));
        assert!(!journal.snapshot(states[2].clone()).unwrap());
    }
}

/// On Unix, a periodic snapshot cannot reactivate an entry that an append found removed.
#[cfg(unix)]
#[test]
fn a_timed_snapshot_keeps_a_detected_removed_entry_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
    fs::remove_file(entry_file(dir.path(), "entry", "journal")).unwrap();
    let (_, result) = place_and_append(&mut session, &mut journal);
    assert_eq!(result.unwrap_err(), REMOVED_ENTRY);
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(journal.snapshot(documents).unwrap_err(), REMOVED_ENTRY);
    assert!(!entry_file(dir.path(), "entry", "journal").exists());
}

/// On Unix, a timed snapshot checks entry lock ownership before skipping or publishing:
/// removing and recreating the recovery directory (with or without a replacement lock file)
/// disables the entry and reports a removal fault instead of publishing unprotected files.
#[cfg(unix)]
#[test]
fn a_timed_snapshot_checks_entry_lock_ownership_before_publishing() {
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("recreated-dirty");
    let (mut journal, states) = journal_with(&dir, 1);
    fs::remove_dir_all(&dir).unwrap();
    fs::create_dir_all(&dir).unwrap();
    assert_eq!(journal.snapshot(states[1].clone()).unwrap_err(), REMOVED_ENTRY);
    assert!(journal.removed);
    assert!(journal.file.is_none());
    assert!(!entry_file(&dir, "entry", "pc").exists());
    assert!(!entry_file(&dir, "entry", "journal").exists());

    let dir_unchanged = work.path().join("recreated-unchanged");
    let (mut intact, intact_states) = journal_with(&dir_unchanged, 0);
    assert!(!intact.snapshot(intact_states[0].clone()).unwrap(), "intact unchanged entry skips publication");
    fs::remove_dir_all(&dir_unchanged).unwrap();
    fs::create_dir_all(&dir_unchanged).unwrap();
    fs::write(entry_file(&dir_unchanged, "entry", "lock"), b"").unwrap();
    assert_eq!(intact.snapshot(intact_states[0].clone()).unwrap_err(), REMOVED_ENTRY);
    assert!(intact.removed);
    assert!(intact.file.is_none());
    assert!(!entry_file(&dir_unchanged, "entry", "pc").exists());
    assert!(!entry_file(&dir_unchanged, "entry", "journal").exists());

    let recorder_dir = work.path().join("recreated-recorder");
    let recorder = Arc::new(JournalRecorder::start(recorder_dir.clone(), "entry".to_owned()).unwrap());
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Owned"})).unwrap();
    session.observe_edits(Some(recorder.clone()));
    session.apply("project.rename", &json!({"name":"Edited"})).unwrap();
    recorder.sync().unwrap();
    fs::remove_dir_all(&recorder_dir).unwrap();
    fs::create_dir_all(&recorder_dir).unwrap();
    recorder.snapshot(session.snapshot().unwrap());
    assert_eq!(recorder.sync().unwrap_err(), REMOVED_ENTRY);
    assert_eq!(recorder.fault(), Some(RecordingFault { message: REMOVED_ENTRY.to_owned(), removed: true }));
    assert!(!entry_file(&recorder_dir, "entry", "pc").exists());
    assert!(!entry_file(&recorder_dir, "entry", "journal").exists());
    session.observe_edits(None);
    Arc::into_inner(recorder).unwrap().finish().unwrap();
}

/// The first snapshot of a recorder that has seen nothing yet creates the entry.
#[test]
fn a_first_snapshot_creates_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = JournalRecorder::start(dir.path().to_path_buf(), "entry".to_owned()).unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Fresh"})).unwrap();
    recorder.snapshot(session.snapshot().unwrap());
    recorder.finish().unwrap();
    assert_eq!(recover(dir.path(), "entry").unwrap().project.name, "Fresh");
}

/// A folder flush that fails after the new snapshot was renamed into place adopts that
/// snapshot: the old journal, which recovery would now ignore, is closed, and the next edit
/// starts another snapshot instead of appending edits recovery could not see.
#[test]
fn a_failed_flush_after_the_snapshot_rename_adopts_the_new_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
    place_and_append(&mut session, &mut journal).1.unwrap();
    let snapshot = fs::read(entry_file(dir.path(), "entry", "pc")).unwrap();
    journal.rebase_after = journal.journal_bytes;
    faults::arm(dir.path(), Fault::Flush, 1);
    let (second, result) = place_and_append(&mut session, &mut journal);
    assert!(result.unwrap_err().contains("could not be flushed"));
    assert_ne!(fs::read(entry_file(dir.path(), "entry", "pc")).unwrap(), snapshot);
    assert!(journal.file.is_none());
    assert_eq!(journal.sha256(), documents_sha256(&second));
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.stop), (second, Some(Stop::OtherSnapshot)));

    place_and_append(&mut session, &mut journal).1.unwrap();
    let (fourth, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (fourth, 1, None));
}

/// A folder flush that fails after the new journal was renamed into place, at the size limit,
/// leaves the new snapshot with an empty journal that no handle is open on: recovery returns
/// the new snapshot, the next edit starts another snapshot, and the edit after it replays.
#[test]
fn a_failed_flush_after_the_journal_rename_leaves_the_new_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
    place_and_append(&mut session, &mut journal).1.unwrap();
    journal.rebase_after = journal.journal_bytes;
    // The first flush is the new snapshot's, the second the new journal's.
    faults::arm(dir.path(), Fault::Flush, 2);
    let (second, result) = place_and_append(&mut session, &mut journal);
    assert!(result.unwrap_err().contains("could not be flushed"));
    assert!(journal.file.is_none());
    assert_eq!(journal.sha256(), documents_sha256(&second));
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (second, 0, None));

    let (third, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (third, 0, None));
    let (fourth, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (fourth, 1, None));
}

/// A recorder whose first snapshot fails, at the flush after its snapshot or after its journal,
/// keeps that error for `sync` and keeps the entry's lock, so no other writer can take the
/// entry; the next edit writes the snapshot again, and later edits are recovered.
#[test]
fn a_failed_first_flush_is_reported_and_later_edits_are_recovered() {
    for nth in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        faults::arm(dir.path(), Fault::Flush, nth);
        let recorder = Arc::new(JournalRecorder::start(dir.path().to_path_buf(), "entry".to_owned()).unwrap());
        let mut session = Session::default();
        session.apply("project.new", &json!({"name":"Before"})).unwrap();
        session.observe_edits(Some(recorder.clone()));
        assert!(recorder.sync().unwrap_err().contains("could not be flushed"), "flush {nth}");
        let second = EditJournal::create(dir.path(), "entry", Documents::new()).unwrap_err();
        assert_eq!(second, "Recovery entry `entry` is in use by another session.", "flush {nth}");
        session.apply("project.rename", &json!({"name":"Recreated"})).unwrap();
        session.apply("project.rename", &json!({"name":"After"})).unwrap();
        session.observe_edits(None);
        Arc::into_inner(recorder).unwrap().finish().unwrap();
        let recovered = recover(dir.path(), "entry").unwrap();
        assert_eq!((recovered.commands, recovered.stop), (vec!["project.rename".to_owned()], None), "flush {nth}");
        assert_eq!(recovered.documents, encode_documents(session.snapshot().unwrap()).unwrap(), "flush {nth}");
    }
}

/// An append that fails after writing part of its frame closes the journal: recovery stops at
/// the torn record with the last good state, the next edit becomes a new snapshot, and the
/// edit after it replays.
#[test]
fn a_record_write_cut_short_closes_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
    let (first, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let before = journal_len(dir.path());
    faults::arm(dir.path(), Fault::Tear, 1);
    let result = place_and_append(&mut session, &mut journal).1;
    assert!(result.unwrap_err().contains("cut short"));
    assert!(journal.file.is_none());
    assert!(journal_len(dir.path()) > before);
    assert_eq!(journal.sha256(), documents_sha256(&first));
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!(recovered.documents, first);
    assert!(matches!(recovered.stop, Some(Stop::Unreadable { index: 1, .. })), "{:?}", recovered.stop);

    let (third, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (third, 0, None));
    let (fourth, result) = place_and_append(&mut session, &mut journal);
    result.unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (fourth, 1, None));
}

/// Reopening the unchanged project after edits uses a fresh publication. Failures before the
/// snapshot rename preserve both files and the open journal; failures after it adopt the new
/// state and close the journal. A stale journal cannot resurrect the discarded edits, even
/// though its base hash matches. The second publish failure exercises the crash boundary
/// between renaming the snapshot and replacing its journal.
#[test]
fn reopening_the_unchanged_project_drops_its_edits() {
    for fault in [None, Some((Fault::Publish, 1)), Some((Fault::Flush, 1)), Some((Fault::Publish, 2)), Some((Fault::Flush, 2))] {
        let dir = tempfile::tempdir().unwrap();
        let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
        let original = session.snapshot().unwrap();
        let opened = encode_documents(original.clone()).unwrap();
        let snapshot = fs::read(entry_file(dir.path(), "entry", "pc")).unwrap();
        place_and_append(&mut session, &mut journal).1.unwrap();
        let (edited, result) = place_and_append(&mut session, &mut journal);
        result.unwrap();
        let old_journal = fs::read(entry_file(dir.path(), "entry", "journal")).unwrap();
        if let Some((fault, nth)) = fault {
            faults::arm(dir.path(), fault, nth);
        }
        let result = journal.rebase(opened.clone());
        let recovered = recover(dir.path(), "entry").unwrap();
        if let Some((fault, _)) = fault {
            let expected = match fault {
                Fault::Publish => "The recovery file could not be staged.",
                _ => "The recovery folder could not be flushed.",
            };
            assert_eq!(result.unwrap_err(), expected);
        } else {
            result.unwrap();
        }
        if fault == Some((Fault::Publish, 1)) {
            assert_eq!(fs::read(entry_file(dir.path(), "entry", "pc")).unwrap(), snapshot);
            assert_eq!(fs::read(entry_file(dir.path(), "entry", "journal")).unwrap(), old_journal);
            assert!(journal.file.is_some());
            assert_eq!(journal.sha256(), documents_sha256(&edited));
            assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (edited, 2, None));
        } else {
            let replacement = fs::read(entry_file(dir.path(), "entry", "pc")).unwrap();
            assert_ne!(decode_snapshot(&replacement).unwrap().0.generation, decode_snapshot(&snapshot).unwrap().0.generation);
            assert_eq!(journal.file.is_none(), fault.is_some());
            assert_eq!(journal.sha256(), documents_sha256(&opened));
            let stop = if matches!(fault, Some((Fault::Flush, 1) | (Fault::Publish, 2))) { Some(Stop::OtherSnapshot) } else { None };
            assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (opened, 0, stop));
        }
        session.open_snapshot(original);
        let (next, result) = place_and_append(&mut session, &mut journal);
        result.unwrap();
        assert_eq!(recover(dir.path(), "entry").unwrap().documents, next);
    }
}

/// A stale journal left by a failed publication stays stale when another snapshot returns to
/// its original content. Identity is checked across repeated failed rebases, not just once.
#[test]
fn a_stale_journal_is_never_replayed_onto_a_snapshot_with_its_content() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, mut journal) = limited_journal(dir.path(), u64::MAX / 2);
    let opened = encode_documents(session.snapshot().unwrap()).unwrap();
    place_and_append(&mut session, &mut journal).1.unwrap();
    let edited = encode_documents(session.snapshot().unwrap()).unwrap();
    faults::arm(dir.path(), Fault::Publish, 2);
    assert_eq!(journal.rebase(edited.clone()).unwrap_err(), "The recovery file could not be staged.");
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.stop), (edited, Some(Stop::OtherSnapshot)));

    faults::arm(dir.path(), Fault::Flush, 1);
    assert_eq!(journal.rebase(opened.clone()).unwrap_err(), "The recovery folder could not be flushed.");
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (opened.clone(), 0, Some(Stop::OtherSnapshot)));
    journal.rebase(opened.clone()).unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (opened, 0, None));
}

/// On Unix, removing the recovery folder leaves the open journal writable but unreachable.
/// The next append reports the loss and disables the entry. Recreating the folder cannot reuse
/// the old lock handle: a rebase to the original state and later edits are refused, and the
/// recorder reports the loss through `sync`/`finish`.
#[cfg(unix)]
#[test]
fn a_removed_recovery_folder_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("recovery");
    let (mut session, mut journal) = limited_journal(&folder, u64::MAX / 2);
    let opened = encode_documents(session.snapshot().unwrap()).unwrap();
    place_and_append(&mut session, &mut journal).1.unwrap();
    fs::remove_dir_all(&folder).unwrap();
    let result = place_and_append(&mut session, &mut journal).1;
    assert_eq!(result.unwrap_err(), REMOVED_ENTRY);
    assert!(journal.file.is_none());
    assert_eq!(place_and_append(&mut session, &mut journal).1.unwrap_err(), REMOVED_ENTRY);
    fs::create_dir(&folder).unwrap();
    assert_eq!(journal.rebase(opened).unwrap_err(), REMOVED_ENTRY);
    assert!(fs::read_dir(&folder).unwrap().next().is_none());

    let recorder = Arc::new(JournalRecorder::start(folder.clone(), "entry".to_owned()).unwrap());
    session.observe_edits(Some(recorder.clone()));
    recorder.sync().unwrap();
    fs::remove_dir_all(&folder).unwrap();
    session.apply("project.rename", &json!({"name":"Lost"})).unwrap();
    session.observe_edits(None);
    assert_eq!(Arc::into_inner(recorder).unwrap().finish().unwrap_err(), REMOVED_ENTRY);
}

/// Old unwrapped snapshots and generation-less journals are refused explicitly; the caller can
/// offer discard without changing the saved project. Future snapshot versions are refused too.
#[test]
fn unsupported_recovery_formats_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, states) = journal_with(dir.path(), 0);
    drop(journal);
    let snapshot_path = entry_file(dir.path(), "entry", "pc");
    let snapshot = fs::read(&snapshot_path).unwrap();
    fs::write(&snapshot_path, encode_packed_documents(states[0].clone()).unwrap()).unwrap();
    assert_eq!(recover(dir.path(), "entry").unwrap_err(), "This recovery entry uses pcbcraft-recovery/1, which this version can't read.");
    fs::write(&snapshot_path, b"pcbcraft-recovery/3\n{}").unwrap();
    assert_eq!(recover(dir.path(), "entry").unwrap_err(), "Unsupported recovery snapshot format: pcbcraft-recovery/3.");
    fs::write(&snapshot_path, snapshot).unwrap();
    fs::write(entry_file(dir.path(), "entry", "journal"), b"pcbcraft-recovery/1\n{}").unwrap();
    assert_eq!(recover(dir.path(), "entry").unwrap_err(), "This recovery entry uses pcbcraft-recovery/1, which this version can't read.");
}

/// The snapshot envelope is checked before replay: a torn header, invalid metadata, a changed
/// document hash or a truncated archive cannot be returned as a recovered project.
#[test]
fn a_damaged_snapshot_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, _) = journal_with(dir.path(), 0);
    drop(journal);
    let snapshot_path = entry_file(dir.path(), "entry", "pc");
    let snapshot = fs::read(&snapshot_path).unwrap();
    let (header, documents) = decode_snapshot(&snapshot).unwrap();
    let packed = encode_packed_documents(documents).unwrap();
    for (metadata, reason) in [
        (json!({"generation":header.generation,"base_sha256":"wrong"}), "do not match its header"),
        (json!({"base_sha256":header.base_sha256}), "missing field `generation`"),
        (json!({"generation":null,"base_sha256":header.base_sha256}), "invalid type: null"),
    ] {
        let mut damaged = MAGIC.to_vec();
        damaged.extend(frame(&serde_json::to_vec(&metadata).unwrap()).unwrap());
        damaged.extend_from_slice(&packed);
        fs::write(&snapshot_path, damaged).unwrap();
        assert!(recover(dir.path(), "entry").unwrap_err().contains(reason));
    }
    for (length, reason) in [(MAGIC.len() - 1, "snapshot is cut short"), (MAGIC.len() + 2, "record is cut short")] {
        fs::write(&snapshot_path, &snapshot[..length]).unwrap();
        assert!(recover(dir.path(), "entry").unwrap_err().contains(reason));
    }
    let mut damaged = snapshot.clone();
    damaged[MAGIC.len() + 8] ^= 1;
    fs::write(&snapshot_path, damaged).unwrap();
    assert!(recover(dir.path(), "entry").unwrap_err().contains("record is damaged"));
    fs::write(&snapshot_path, &snapshot[..snapshot.len() - 100]).unwrap();
    assert_eq!(recover(dir.path(), "entry").unwrap_err(), "missing ZIP end of directory");
    fs::write(&snapshot_path, snapshot).unwrap();
    assert!(recover(dir.path(), "entry").unwrap().stop.is_none());
}

/// The snapshot header accepts unknown fields up to its exact byte cap, refusing the first byte
/// beyond it before decoding the archive. The project bytes remain production-codec output.
#[test]
fn snapshot_header_size_has_an_exact_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, states) = journal_with(dir.path(), 0);
    drop(journal);
    let path = entry_file(dir.path(), "entry", "pc");
    let (header, _) = decode_snapshot(&fs::read(&path).unwrap()).unwrap();
    let mut value = serde_json::to_value(header).unwrap();
    value["padding"] = json!("");
    let unpadded = serde_json::to_vec(&value).unwrap().len();
    let padding = SNAPSHOT_OVERHEAD_BYTES - MAGIC.len() - 36 - unpadded;
    for extra in [0, 1] {
        value["padding"] = json!("x".repeat(padding + extra));
        let mut snapshot = MAGIC.to_vec();
        snapshot.extend(frame(&serde_json::to_vec(&value).unwrap()).unwrap());
        assert_eq!(snapshot.len(), SNAPSHOT_OVERHEAD_BYTES + extra);
        snapshot.extend(encode_packed_documents(states[0].clone()).unwrap());
        fs::write(&path, snapshot).unwrap();
        if extra == 0 {
            assert_eq!(recover(dir.path(), "entry").unwrap().documents, states[0]);
        } else {
            assert_eq!(recover(dir.path(), "entry").unwrap_err(), "Recovery snapshot header exceeds its size limit.");
        }
    }
}

/// A matching generation is insufficient when the journal's base hash differs; no edit from
/// that journal is applied even if its framing is intact.
#[test]
fn a_matching_generation_still_requires_the_document_hash() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, states) = journal_with(dir.path(), 1);
    drop(journal);
    let path = entry_file(dir.path(), "entry", "journal");
    let old = fs::read(&path).unwrap();
    let mut rest = &old[MAGIC.len()..];
    let mut header: Header = serde_json::from_slice(&read_frame(&mut rest).unwrap().unwrap()).unwrap();
    header.base_sha256 = "wrong".to_owned();
    let mut changed = MAGIC.to_vec();
    changed.extend(frame(&serde_json::to_vec(&header).unwrap()).unwrap());
    changed.extend_from_slice(rest);
    fs::write(path, changed).unwrap();
    let recovered = recover(dir.path(), "entry").unwrap();
    assert_eq!((recovered.documents, recovered.commands.len(), recovered.stop), (states[0].clone(), 0, Some(Stop::OtherSnapshot)));
}
