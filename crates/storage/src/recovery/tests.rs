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
    // A folder that does not exist makes publishing the new snapshot fail; the open journal
    // file is still appended to.
    journal.dir = dir.path().join("missing");
    let (second, result) = place_and_append(&mut session, &mut journal);
    journal.dir = dir.path().to_path_buf();
    assert!(result.unwrap_err().contains("could not be renewed"));
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
    faults::fail_flush(dir.path(), 1);
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
    faults::fail_flush(dir.path(), 2);
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

/// A recorder whose first entry cannot be created, because the flush after its snapshot or
/// after its journal fails, keeps that error for `sync`, releases the entry and creates it
/// again from the next edit, so later edits are recovered.
#[test]
fn a_failed_first_flush_is_reported_and_later_edits_are_recovered() {
    for nth in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        faults::fail_flush(dir.path(), nth);
        let recorder = Arc::new(JournalRecorder::start(dir.path().to_path_buf(), "entry".to_owned()).unwrap());
        let mut session = Session::default();
        session.apply("project.new", &json!({"name":"Before"})).unwrap();
        session.observe_edits(Some(recorder.clone()));
        session.apply("project.rename", &json!({"name":"Recreated"})).unwrap();
        assert!(recorder.sync().unwrap_err().contains("could not be flushed"), "flush {nth}");
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
    faults::tear_next_record(dir.path());
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
