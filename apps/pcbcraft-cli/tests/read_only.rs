//! Exercise read-only CLI startup on both native storage layouts.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);

impl Drop for Scratch {
    /// Remove this test's private directory, including after an assertion failure.
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Fingerprint the entire archive, including the files inside it and their metadata.
fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// A real stdio request can inspect a saved project but a valid rename receives `out_of_scope`.
/// The archive remains byte-identical after both operations, not just equivalent on decode.
#[test]
fn read_only_cli_leaves_the_archive_unchanged() -> Result<(), Box<dyn std::error::Error>> {
    let dir = scratch()?;
    let project = dir.0.join("--review.pc");
    let mut editor = pcbcraft_engine::Session::default();
    editor.apply("project.new", &Default::default())?;
    let bytes = pcbcraft_engine::persistence::encode_packed(editor.snapshot().ok_or("missing synthetic project")?)?;
    fs::write(&project, &bytes)?;

    let requests = dir.0.join("requests.jsonl");
    fs::write(
        &requests,
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"inspect_project\",\"arguments\":{}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"execute\",\"arguments\":{\"command\":\"project.rename\",\"params\":{\"name\":\"Must not appear in the project\"}}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"inspect_project\",\"arguments\":{}}}\n",
        ),
    )?;
    let editable = Command::new(env!("CARGO_BIN_EXE_pcbcraft-cli")).current_dir(&dir.0).args(["mcp", "--project", "--review.pc"]).output()?;
    assert!(editable.status.success(), "{}", String::from_utf8_lossy(&editable.stderr));
    let reply = Command::new(env!("CARGO_BIN_EXE_pcbcraft-cli"))
        .current_dir(&dir.0)
        .args(["mcp", "--read-only", "--project", "--review.pc"])
        .stdin(File::open(requests)?)
        .output()?;
    assert!(reply.status.success(), "{}", String::from_utf8_lossy(&reply.stderr));
    let stdout = String::from_utf8(reply.stdout)?;
    let [_, before, refusal, after] = stdout.lines().collect::<Vec<_>>()[..] else {
        return Err(format!("unexpected MCP replies: {stdout}").into());
    };
    assert!(before.contains(r#"\"scope\":\"read_only\""#) && before.contains(r#"\"name\":\"Untitled\""#), "{before}");
    assert!(refusal.contains(r#""code":"out_of_scope""#), "{refusal}");
    assert_eq!(before.replacen(r#""id":2"#, r#""id":4"#, 1), after);
    let readback = fs::read(project)?;
    assert_eq!(fingerprint(&readback), fingerprint(&bytes));
    assert_eq!(readback, bytes);
    Ok(())
}

/// Create a private test directory without depending on platform-specific file APIs.
fn scratch() -> std::io::Result<Scratch> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(std::io::Error::other)?.as_nanos();
    let dir = std::env::temp_dir().join(format!("pcbcraft-read-only-{}-{nonce}", std::process::id()));
    fs::create_dir(&dir)?;
    Ok(Scratch(dir))
}

/// Include empty directories, every file name, and the bytes of every file (including locks).
fn inventory(root: &Path) -> std::io::Result<BTreeMap<PathBuf, Option<Vec<u8>>>> {
    let mut entries = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            entries.insert(path.clone(), None);
            entries.extend(inventory(&path)?);
        } else {
            entries.insert(path.clone(), Some(fs::read(path)?));
        }
    }
    Ok(entries)
}

/// A lock-free split project stays lock-free, and startup refuses a real interrupted save
/// without changing its journal, staged documents, or any other file.
#[test]
fn read_only_cli_does_not_lock_or_recover_a_split_project() -> Result<(), Box<dyn std::error::Error>> {
    use pcbcraft_engine::persistence::encode_documents;

    let dir = scratch()?;
    let root = dir.0.join("split");
    let mut editor = pcbcraft_engine::Session::default();
    editor.apply("project.new", &Default::default())?;
    let original = encode_documents(editor.snapshot().ok_or("missing synthetic project")?)?;
    for (name, bytes) in &original {
        let file = root.join(name);
        fs::create_dir_all(file.parent().ok_or("missing document directory")?)?;
        fs::write(file, bytes)?;
    }
    let run = |path: &Path| Command::new(env!("CARGO_BIN_EXE_pcbcraft-cli")).args(["mcp", "--read-only", "--project"]).arg(path).output();
    let before = inventory(&dir.0)?;
    assert!(!dir.0.join(".split.pcbcraft-lock").exists());
    let reply = run(&root.join("project.json"))?;
    assert!(reply.status.success(), "{}", String::from_utf8_lossy(&reply.stderr));
    assert_eq!(inventory(&dir.0)?, before);

    let changed = std::sync::Arc::new(editor.project().ok_or("missing synthetic project")?.renamed("After recovery")?);
    let changed = encode_documents(changed)?;
    let interrupted = pcbcraft_storage::save(&root, Some(&original), &changed, &mut |phase| {
        if phase == "journal" { Err("simulated interruption".to_owned()) } else { Ok(()) }
    });
    assert!(matches!(interrupted, Err(ref message) if message.contains("simulated interruption")), "{interrupted:?}");
    let pending = inventory(&dir.0)?;
    for path in [&root, &root.join("project.json")] {
        let refusal = run(path)?;
        assert!(!refusal.status.success());
        assert!(String::from_utf8_lossy(&refusal.stderr).contains("needs recovery"));
        assert_eq!(inventory(&dir.0)?, pending);
    }
    assert_eq!(pcbcraft_storage::load(&root)?, changed);
    Ok(())
}
