//! Acceptance tests for split-document ownership, migration and portable packaging.
use super::*;
use crate::{DELETE_SYMBOL, DELETE_WIRE, DRAW_WIRE, MOVE_SYMBOL, PLACE_FOOTPRINT, PLACE_SYMBOL, ROTATE_SYMBOL, SET_SYMBOL_FIELDS, Session};
use serde_json::{Value, json};
use std::io::Cursor;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const FIXTURE: &[u8] = include_bytes!("../../../app/tests/fixtures/two-resistors.pc");

/// Open the stable fixture through the real legacy migration, retaining its document IDs.
fn session() -> Session {
    let mut s = Session::default();
    s.open_snapshot(decode(FIXTURE).unwrap());
    s
}

/// Run an ordinary editor command against the latest revision.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Encode through the public working-format writer.
fn documents(s: &Session) -> Documents {
    encode_documents(s.snapshot().unwrap()).unwrap()
}

/// Return the exact paths changed, including new and deleted files.
fn changed(a: &Documents, b: &Documents) -> BTreeSet<String> {
    a.keys().chain(b.keys()).filter(|path| a.get(*path) != b.get(*path)).cloned().collect()
}

/// Edit one parsed test document, allowing schema and cross-file failures to be isolated.
fn edit(d: &mut Documents, path: &str, f: impl FnOnce(&mut Value)) {
    let mut value = serde_json::from_slice(&d[path]).unwrap();
    f(&mut value);
    d.insert(path.to_owned(), serde_json::to_vec(&value).unwrap());
}

/// Locate the sheet without assuming a generated filename in individual tests.
fn sheet(d: &Documents) -> String {
    d.keys().find(|path| path.starts_with("sheets/")).unwrap().clone()
}

/// Split and legacy forms retain identities, pin mappings, wire geometry and sheet names.
#[test]
fn round_trip_and_legacy_migrations_have_identical_documents() {
    let expected = documents(&session());
    let fixtures = [
        (ENTRY_POINT, include_bytes!("../../../app/tests/fixtures/split/two-resistors/project.pc").as_slice()),
        ("board.json", include_bytes!("../../../app/tests/fixtures/split/two-resistors/board.json").as_slice()),
        (
            "sheets/0bbcf5e3-dd21-47b3-ae6b-bd66c8f97989.json",
            include_bytes!("../../../app/tests/fixtures/split/two-resistors/sheets/0bbcf5e3-dd21-47b3-ae6b-bd66c8f97989.json").as_slice(),
        ),
        (
            "library/symbols/starter/resistor.r1.json",
            include_bytes!("../../../app/tests/fixtures/split/two-resistors/library/symbols/starter/resistor.r1.json").as_slice(),
        ),
    ];
    let old_documents = fixtures.into_iter().map(|(path, bytes)| (path.to_owned(), bytes.to_vec())).collect();
    assert_eq!(expected, encode_documents(decode_documents(&old_documents).unwrap()).unwrap());
    let old_archive = decode_packed(include_bytes!("../../../app/tests/fixtures/split/two-resistors.pc.zip")).unwrap();
    assert_eq!(expected, encode_documents(old_archive.project).unwrap());
    for bytes in [
        include_bytes!("../../../app/tests/fixtures/two-resistors-format-1.pc").as_slice(),
        include_bytes!("../../../app/tests/fixtures/two-resistors-format-2.pc").as_slice(),
        FIXTURE,
    ] {
        let project = decode(bytes).unwrap();
        assert_eq!(encode_documents(project).unwrap(), expected);
    }
    let format3 = std::str::from_utf8(include_bytes!("../../../app/tests/fixtures/two-resistors-format-2.pc"))
        .unwrap()
        .replace("pcbcraft-project/2", "pcbcraft-project/3");
    assert_eq!(encode_documents(decode(format3.as_bytes()).unwrap()).unwrap(), expected);
    assert_eq!(encode_documents(decode_documents(&expected).unwrap()).unwrap(), expected);
    let mut renamed = expected.clone();
    edit(&mut renamed, ENTRY_POINT, |value| value["sheets"][0]["name"] = json!("Power supply"));
    let reopened = decode_documents(&renamed).unwrap();
    let canonical = encode_documents(reopened.clone()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&canonical[ENTRY_POINT]).unwrap()["sheets"][0]["name"], "Power supply");
    let packed = encode_packed(reopened).unwrap();
    assert_eq!(encode_documents(decode_packed(&packed).unwrap().project).unwrap(), canonical);
}

/// Each common edit changes only its owning document and one editable record where applicable.
#[test]
fn edit_to_file_boundaries_and_one_record_diffs() {
    for (command, params, owner) in [
        (MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1270000,"y_nm":0}), "sheet"),
        (ROTATE_SYMBOL, json!({"symbol":"symbol-2"}), "sheet"),
        (SET_SYMBOL_FIELDS, json!({"symbol":"symbol-2","reference":"R7","value":"10k"}), ENTRY_POINT),
        (PLACE_FOOTPRINT, json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0}), "board.json"),
    ] {
        let mut s = session();
        let before = documents(&s);
        apply(&mut s, command, params);
        let after = documents(&s);
        let path = if owner == "sheet" { sheet(&before) } else { owner.to_owned() };
        assert_eq!(changed(&before, &after), BTreeSet::from([path.clone()]));
        if command != PLACE_FOOTPRINT {
            let a = std::str::from_utf8(&before[&path]).unwrap();
            let b = std::str::from_utf8(&after[&path]).unwrap();
            assert_eq!(a.lines().count(), b.lines().count());
            assert_eq!(a.lines().zip(b.lines()).filter(|(a, b)| a != b).count(), 1);
        }
        assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    }
}

/// Free-space wires and symbol-copy lifetimes affect exactly the documents that own them.
#[test]
fn wire_and_definition_changes_remain_local() {
    let mut s = session();
    let before = documents(&s);
    let path = sheet(&before);
    apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":1270000},"to":{"x_nm":2540000,"y_nm":1270000},"bends":[]}));
    let wired = documents(&s);
    assert_eq!(changed(&before, &wired), BTreeSet::from([path.clone()]));
    let wire = s.inspect()["project"]["schematic"]["wires"].as_array().unwrap().last().unwrap()["id"].clone();
    apply(&mut s, DELETE_WIRE, json!({"wire":wire}));
    assert_eq!(documents(&s), before);
    apply(&mut s, PLACE_SYMBOL, json!({"definition":"led","x_nm":0,"y_nm":0}));
    let added = documents(&s);
    let new_file = added.keys().find(|path| !before.contains_key(*path)).unwrap().clone();
    assert_eq!(changed(&before, &added), BTreeSet::from([ENTRY_POINT.to_owned(), path, new_file]));
    let id = s.inspect()["project"]["schematic"]["symbols"].as_array().unwrap().last().unwrap()["id"].clone();
    apply(&mut s, DELETE_SYMBOL, json!({"symbol":id}));
    assert_eq!(documents(&s), before);
}

/// Appending records keeps all earlier record lines intact, including their separators.
#[test]
fn appends_preserve_existing_record_lines() {
    let mut s = session();
    let before = documents(&s);
    apply(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let after = documents(&s);
    for path in [ENTRY_POINT.to_owned(), sheet(&before)] {
        let a = std::str::from_utf8(&before[&path]).unwrap();
        let b = std::str::from_utf8(&after[&path]).unwrap();
        for line in a.lines().filter(|line| line.trim_start().starts_with("{\"id\":")) {
            assert!(b.lines().any(|candidate| line == candidate));
        }
    }
}

/// Missing, extra, mistyped and cross-document records fail before any project is returned.
#[test]
fn reject_incomplete_or_inconsistent_document_sets() {
    let original = documents(&session());
    let path = sheet(&original);
    for name in original.keys() {
        let mut d = original.clone();
        d.remove(name);
        assert!(decode_documents(&d).is_err());
    }
    for (name, field, value) in [
        (ENTRY_POINT, "extra", json!(true)),
        (ENTRY_POINT, "sheets", Value::Null),
        (path.as_str(), "junctions", Value::Null),
        (path.as_str(), "id", json!(Uuid::nil())),
        ("board.json", "footprints", Value::Null),
    ] {
        let mut d = original.clone();
        edit(&mut d, name, |v| v[field] = value);
        assert!(decode_documents(&d).unwrap_err().contains(name));
    }
    let mut d = original.clone();
    edit(&mut d, &path, |v| v["symbols"][0]["component"] = json!(999));
    assert!(decode_documents(&d).is_err());
    let mut d = original.clone();
    edit(&mut d, &path, |v| v["wires"][0]["bends"] = Value::Null);
    assert!(decode_documents(&d).is_err());
    let mut d = original.clone();
    let library = d.keys().find(|p| p.starts_with("library/")).unwrap().clone();
    edit(&mut d, &library, |v| v["revision"] = json!(999));
    assert!(decode_documents(&d).unwrap_err().contains(&library));
    let mut d = original.clone();
    d.insert("../project.pc".to_owned(), b"{}".to_vec());
    assert!(decode_documents(&d).is_err());
    let mut s = session();
    apply(&mut s, PLACE_FOOTPRINT, json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0}));
    let mut d = documents(&s);
    edit(&mut d, "board.json", |v| v["footprints"][0]["id"] = json!(2));
    assert!(decode_documents(&d).unwrap_err().contains("duplicate"));
}

/// Construct ordinary OS-style archives, optionally including explicit directory entries.
fn zip_files(d: &Documents, prefix: &str, dirs: bool) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    if dirs && !prefix.is_empty() {
        writer.add_directory(prefix, options).unwrap();
    }
    if dirs {
        writer.add_directory(format!("{prefix}sheets/"), options).unwrap();
    }
    for (path, bytes) in d {
        writer.start_file(format!("{prefix}{path}"), options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Hand-zipped folders with or without a single wrapper directory open without a manifest.
#[test]
fn accepts_missing_manifest_and_adds_one_on_export() {
    let expected = documents(&session());
    for prefix in ["", "my-board/"] {
        let bytes = zip_files(&expected, prefix, true);
        let opened = decode_packed(&bytes).unwrap();
        assert!(opened.needs_manifest);
        assert!(!opened.legacy);
        assert_eq!(encode_documents(opened.project.clone()).unwrap(), expected);
        let packed = encode_packed(opened.project.clone()).unwrap();
        assert!(!decode_packed(&packed).unwrap().needs_manifest);
        assert_eq!(packed, encode_packed(opened.project).unwrap());
    }
}

/// OS-created sidecars are ignored only after ZIP validation and never become project documents.
#[test]
fn manifestless_archives_ignore_os_metadata_before_root_discovery() {
    let expected = documents(&session());
    for prefix in ["", "my-board/"] {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        if !prefix.is_empty() {
            writer.add_directory(prefix, options).unwrap();
        }
        for directory in ["__MACOSX/", "__MACOSX/my-board/", "__MACOSX/my-board/sheets/"] {
            writer.add_directory(directory, options).unwrap();
        }
        for (path, bytes) in &expected {
            writer.start_file(format!("{prefix}{path}"), options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        let mut metadata =
            vec!["__MACOSX/._my-board".to_owned(), "__MACOSX/project.pc".to_owned(), "__MACOSX/my-board/sheets/._sheet.json".to_owned()];
        for name in [".DS_Store", "Thumbs.db", "desktop.ini"] {
            metadata.extend([name.to_owned(), format!("{prefix}sheets/{name}"), format!("{prefix}library/symbols/starter/{name}")]);
        }
        for path in metadata {
            writer.start_file(path, options).unwrap();
            writer.write_all(b"OS metadata").unwrap();
        }
        let opened = decode_packed(&writer.finish().unwrap().into_inner()).unwrap();
        assert!(opened.needs_manifest);
        assert_eq!(opened.documents.as_ref(), Some(&expected));
        let saved = encode_packed_documents(opened.documents.unwrap()).unwrap();
        let reopened = decode_packed(&saved).unwrap();
        assert!(!reopened.needs_manifest);
        assert_eq!(reopened.documents.as_ref(), Some(&expected));
    }
}

/// A present manifest is authoritative: corrupted claims never fall back to hand-ZIP mode.
#[test]
fn rejects_bad_manifests_and_extra_archive_entries() {
    let packed = encode_packed(session().snapshot().unwrap()).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(&packed)).unwrap();
    let mut d = Documents::new();
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        d.insert(file.name().unwrap().into_owned(), bytes);
    }
    for (field, value) in [("sha256", json!("0".repeat(64))), ("size_bytes", json!(0)), ("path", json!("missing.json"))] {
        let mut bad = d.clone();
        edit(&mut bad, "manifest.json", |v| v["parts"][0][field] = value);
        assert!(decode_packed(&zip_files(&bad, "", false)).is_err());
    }
    for manifest in [b"null".as_slice(), b"{}", b"broken", b"{\"format\":\"pcbcraft-container/999\",\"parts\":[],\"assets\":[]}"] {
        let mut bad = d.clone();
        bad.insert("manifest.json".to_owned(), manifest.to_vec());
        assert!(decode_packed(&zip_files(&bad, "", false)).is_err());
    }
    for prefix in ["", "wrapped/"] {
        for path in [".DS_Store", "sheets/Thumbs.db", "library/desktop.ini", "__MACOSX/._project.pc"] {
            let mut bad = d.clone();
            bad.insert(path.to_owned(), b"OS metadata".to_vec());
            assert!(decode_packed(&zip_files(&bad, prefix, false)).is_err(), "manifest must reject {prefix}{path}");
        }
    }
    let mut unsupported = d.clone();
    edit(&mut unsupported, "manifest.json", |value| {
        value["assets"] = json!([{"path":format!("assets/{}", "0".repeat(64)), "size_bytes":0, "sha256":"0".repeat(64)}]);
    });
    assert!(decode_packed(&zip_files(&unsupported, "", false)).err().unwrap().contains("bundled assets are not yet supported"));
    for path in ["notes.txt", "__MACOSX", "../evil", "/evil", "C:/evil", "library/symbols/starter/unused.r1.json", "second/project.pc"] {
        let mut bad = documents(&session());
        bad.insert(path.to_owned(), b"{}".to_vec());
        assert!(decode_packed(&zip_files(&bad, "", false)).is_err());
    }
    let mut missing = d.clone();
    missing.remove("board.json");
    assert!(decode_packed(&zip_files(&missing, "", false)).is_err());
    let mut extra = d;
    extra.remove("manifest.json");
    extra.insert("outside.json".to_owned(), b"{}".to_vec());
    assert!(decode_packed(&zip_files(&extra, "wrapped/", false)).is_err());
    let mut asset = documents(&session());
    asset.insert(format!("assets/{}", "0".repeat(64)), Vec::new());
    assert!(decode_packed(&zip_files(&asset, "", false)).is_err());
}

/// ZIP links and payloads over per-part limits are refused before document adoption.
#[test]
fn rejects_links_oversized_parts_and_truncated_archives() {
    for path in ["project.pc", "__MACOSX/._project.pc", "sheets/.DS_Store"] {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.add_symlink(path, "/tmp/elsewhere", SimpleFileOptions::default()).unwrap();
        assert!(decode_packed(&writer.finish().unwrap().into_inner()).err().unwrap().contains("unsupported ZIP entry"));
    }
    for path in ["board.json", "__MACOSX/._project.pc", "sheets/.DS_Store"] {
        let mut d = documents(&session());
        d.insert(path.to_owned(), vec![b' '; MAX_DOCUMENT_BYTES + 1]);
        assert!(decode_packed(&zip_files(&d, "", false)).err().unwrap().contains("size limit"));
    }
    for path in ["__MACOSX/../outside", "sheets/../.DS_Store"] {
        let mut d = documents(&session());
        d.insert(path.to_owned(), b"metadata".to_vec());
        assert!(decode_packed(&zip_files(&d, "", false)).err().unwrap().contains("unsafe ZIP path"));
    }
    let packed = encode_packed(session().snapshot().unwrap()).unwrap();
    assert!(decode_packed(&packed[..packed.len() - 1]).is_err());
}

/// ZIP64 local headers, duplicate names and bad CRCs are rejected before manifest validation.
#[test]
fn rejects_zip64_duplicates_and_crc_corruption() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in documents(&session()) {
        writer.start_file(path, SimpleFileOptions::default().large_file(true)).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    assert!(decode_packed(&writer.finish().unwrap().into_inner()).is_err());
    let mut duplicate = encode_packed(session().snapshot().unwrap()).unwrap();
    for index in (0..duplicate.len() - 10).rev() {
        if &duplicate[index..index + 10] == b"board.json" {
            duplicate[index..index + 10].copy_from_slice(b"project.pc");
        }
    }
    assert!(decode_packed(&duplicate).is_err());
    let mut corrupt = encode_packed(session().snapshot().unwrap()).unwrap();
    let position = corrupt.windows(13).position(|bytes| bytes == b"Two resistors").unwrap();
    corrupt[position] = b'X';
    assert!(decode_packed(&corrupt).is_err());
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut d = documents(&session());
    d.insert("sheets/.DS_Store".to_owned(), b"unique metadata payload".to_vec());
    for (path, bytes) in d {
        writer.start_file(path, SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    let mut corrupt = writer.finish().unwrap().into_inner();
    let position = corrupt.windows(23).position(|bytes| bytes == b"unique metadata payload").unwrap();
    corrupt[position] = b'X';
    assert!(decode_packed(&corrupt).err().unwrap().contains(".DS_Store"));
}
