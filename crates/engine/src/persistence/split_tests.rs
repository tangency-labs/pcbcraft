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

/// Resolve the physical document from its registered path.
fn board_path(d: &Documents) -> String {
    super::project_board_path(&d[ENTRY_POINT]).unwrap().unwrap()
}

/// Split and legacy forms retain identities, pin mappings, wire geometry and sheet names.
#[test]
fn round_trip_and_legacy_migrations_have_identical_documents() {
    let expected = documents(&session());
    let fixtures = [
        (ENTRY_POINT, include_bytes!("../../../app/tests/fixtures/split/two-resistors/project.json").as_slice()),
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
    let old_archive = decode_packed(include_bytes!("../../../app/tests/fixtures/split/two-resistors.pc")).unwrap();
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

/// Nonallocating edits change only their owning document; allocating edits also reserve IDs
/// in the project entry point. Existing authored records retain one-line diffs.
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
        let path = match owner {
            "sheet" => sheet(&before),
            "board.json" => board_path(&before),
            _ => owner.to_owned(),
        };
        let mut expected = BTreeSet::from([path.clone()]);
        if command == PLACE_FOOTPRINT {
            // Placement can advance the allocation watermark.
            expected.insert(ENTRY_POINT.to_owned());
        }
        assert_eq!(changed(&before, &after), expected);
        if command != PLACE_FOOTPRINT {
            let a = std::str::from_utf8(&before[&path]).unwrap();
            let b = std::str::from_utf8(&after[&path]).unwrap();
            assert_eq!(a.lines().count(), b.lines().count());
            assert_eq!(a.lines().zip(b.lines()).filter(|(a, b)| a != b).count(), 1);
        }
        assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    }
}

/// Wire/component allocations update their owning documents and retain allocation metadata
/// after deletion, while unused pinned definitions are removed.
#[test]
fn wire_and_definition_changes_remain_local() {
    let mut s = session();
    let before = documents(&s);
    let path = sheet(&before);
    apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":1270000},"to":{"x_nm":2540000,"y_nm":1270000},"bends":[]}));
    let wired = documents(&s);
    assert_eq!(changed(&before, &wired), BTreeSet::from([ENTRY_POINT.to_owned(), path.clone()]));
    let wire = s.inspect()["project"]["schematic"]["wires"].as_array().unwrap().last().unwrap()["id"].clone();
    apply(&mut s, DELETE_WIRE, json!({"wire":wire}));
    crate::assert_same_content(&decode_documents(&before).unwrap(), &s.snapshot().unwrap());
    apply(&mut s, PLACE_SYMBOL, json!({"definition":"led","x_nm":0,"y_nm":0}));
    let added = documents(&s);
    let new_file = added.keys().find(|path| !before.contains_key(*path)).unwrap().clone();
    assert_eq!(changed(&before, &added), BTreeSet::from([ENTRY_POINT.to_owned(), path, new_file]));
    let id = s.inspect()["project"]["schematic"]["symbols"].as_array().unwrap().last().unwrap()["id"].clone();
    apply(&mut s, DELETE_SYMBOL, json!({"symbol":id}));
    crate::assert_same_content(&decode_documents(&before).unwrap(), &s.snapshot().unwrap());
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
        (board_path(&original).as_str(), "footprints", Value::Null),
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
    d.insert("../project.json".to_owned(), b"{}".to_vec());
    assert!(decode_documents(&d).is_err());
    let mut s = session();
    apply(&mut s, PLACE_FOOTPRINT, json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0}));
    let mut d = documents(&s);
    let path = board_path(&d);
    edit(&mut d, &path, |v| v["footprints"][0]["id"] = json!(2));
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
        assert_eq!(encode_documents(opened.project.clone()).unwrap(), expected);
        let packed = encode_packed(opened.project.clone()).unwrap();
        assert!(!decode_packed(&packed).unwrap().needs_manifest);
        assert_eq!(packed, encode_packed(opened.project).unwrap());
    }
}

/// The new inventory names the fixed JSON entry point, while earlier manifested containers
/// and hand-zipped old layouts fail even when the outer archive is renamed to `.pc`.
#[test]
fn container_three_inventory_and_breaking_layout_boundary() {
    let expected = documents(&session());
    let packed = encode_packed_documents(expected.clone()).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(&packed)).unwrap();
    let mut manifest = String::new();
    zip.by_name("manifest.json").unwrap().read_to_string(&mut manifest).unwrap();
    let manifest: Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest["format"], "pcbcraft-container/3");
    let entry = manifest["parts"].as_array().unwrap().iter().find(|part| part["path"] == "project.json").unwrap();
    assert_eq!(entry["size_bytes"], expected[ENTRY_POINT].len());
    assert!(zip.by_name("project.pc").is_err());
    assert_eq!(decode_packed(&packed).unwrap().documents, expected);

    for version in ["pcbcraft-container/1", "pcbcraft-container/2"] {
        let mut old = expected.clone();
        let mut header = manifest.clone();
        header["format"] = json!(version);
        old.insert("manifest.json".to_owned(), serde_json::to_vec(&header).unwrap());
        let error = decode_packed(&zip_files(&old, "", false)).err().unwrap();
        assert!(error.contains("unsupported container format") && error.contains(version), "{error}");
    }
    let mut old = expected;
    let entry = old.remove(ENTRY_POINT).unwrap();
    old.insert("project.pc".to_owned(), entry);
    for prefix in ["", "myproject/"] {
        assert!(decode_packed(&zip_files(&old, prefix, true)).unwrap().needs_manifest);
    }
}

/// Every `.pc` entry records Unix as its creator with a regular 0644 mode, whichever platform
/// built PCBCraft, so archives match byte for byte across desktops and the browser and readers
/// apply the stored mode instead of deriving one from MS-DOS attributes.
#[test]
fn packed_entries_are_unix_regular_files_on_every_platform() {
    let packed = encode_packed_documents(documents(&session())).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(&packed)).unwrap();
    for index in 0..zip.len() {
        let entry = zip.by_index(index).unwrap();
        assert_eq!(entry.system(), zip::System::Unix, "{:?}", entry.name());
        assert_eq!(entry.unix_mode(), Some(0o100644), "{:?}", entry.name());
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
            vec!["__MACOSX/._my-board".to_owned(), "__MACOSX/project.json".to_owned(), "__MACOSX/my-board/sheets/._sheet.json".to_owned()];
        for name in [".DS_Store", "Thumbs.db", "desktop.ini"] {
            metadata.extend([name.to_owned(), format!("{prefix}sheets/{name}"), format!("{prefix}library/symbols/starter/{name}")]);
        }
        for path in metadata {
            writer.start_file(path, options).unwrap();
            writer.write_all(b"OS metadata").unwrap();
        }
        let opened = decode_packed(&writer.finish().unwrap().into_inner()).unwrap();
        assert!(opened.needs_manifest);
        assert_eq!(&opened.documents, &expected);
        let saved = encode_packed_documents(opened.documents).unwrap();
        let reopened = decode_packed(&saved).unwrap();
        assert!(!reopened.needs_manifest);
        assert_eq!(&reopened.documents, &expected);
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
        for path in [".DS_Store", "sheets/Thumbs.db", "library/desktop.ini", "__MACOSX/._project.json"] {
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
    for path in ["notes.txt", "__MACOSX", "../evil", "/evil", "C:/evil", "library/symbols/starter/unused.r1.json", "second/project.json"] {
        let mut bad = documents(&session());
        bad.insert(path.to_owned(), b"{}".to_vec());
        assert!(decode_packed(&zip_files(&bad, "", false)).is_err());
    }
    let mut missing = d.clone();
    missing.remove(&board_path(&d));
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
    for path in ["project.json", "__MACOSX/._project.json", "sheets/.DS_Store"] {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.add_symlink(path, "/tmp/elsewhere", SimpleFileOptions::default()).unwrap();
        assert!(decode_packed(&writer.finish().unwrap().into_inner()).err().unwrap().contains("unsupported ZIP entry"));
    }
    for path in ["board.json", "__MACOSX/._project.json", "sheets/.DS_Store"] {
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
    let mut entries = documents(&session());
    entries.insert("other.json".to_owned(), b"{}".to_vec());
    let mut duplicate = zip_files(&entries, "", false);
    for index in (0..duplicate.len() - 10).rev() {
        if &duplicate[index..index + 10] == b"other.json" {
            duplicate[index..index + 10].copy_from_slice(b"board.json");
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

/// Explicit conversion checks old manifest hashes before renaming paths and preserves every
/// payload byte; normal Open accepts the same validated previous split archives.
#[test]
fn explicit_conversion_preserves_old_split_packages_and_validates_integrity() {
    use sha2::{Digest, Sha256};
    let expected = documents(&session());
    let mut old = expected.clone();
    let expected = migrate_board_layout(expected, ".").unwrap();
    let entry = old.remove(ENTRY_POINT).unwrap();
    old.insert("project.pc".to_owned(), entry);
    let parts: Vec<_> =
        old.iter().map(|(path, bytes)| json!({"path":path,"size_bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes))})).collect();
    old.insert("manifest.json".to_owned(), serde_json::to_vec(&json!({"format":"pcbcraft-container/2","parts":parts,"assets":[]})).unwrap());
    for prefix in ["", "wrapped/"] {
        let archive = zip_files(&old, prefix, false);
        let opened = decode_packed(&archive).unwrap();
        assert_eq!(opened.documents, expected);
        assert!(opened.needs_manifest);
        let converted = decode_legacy_packed(&archive).unwrap();
        assert_eq!(converted.documents, expected);
        assert_eq!(decode_packed(&encode_packed_documents(converted.documents).unwrap()).unwrap().documents, expected);
    }
    let payload = old.get_mut("project.pc").unwrap();
    payload.push(b' ');
    assert!(decode_legacy_packed(&zip_files(&old, "", false)).err().unwrap().contains("digest mismatch"));
    old.remove("manifest.json");
    assert!(decode_legacy_packed(&zip_files(&old, "", false)).is_ok());
    old.insert("project.json".to_owned(), old["project.pc"].clone());
    assert!(decode_legacy_packed(&zip_files(&old, "", false)).is_err());
    assert!(decode_legacy_packed(&encode_packed_documents(expected).unwrap()).is_err());
}

/// Generated-leg matching indexes sheet wires and scans legs within the step batch budget,
/// cancels cleanly mid-leg, and matches full encode and decode round-trip byte for byte.
#[test]
fn generated_leg_matching_is_bounded_cancellable_and_byte_exact() {
    let mut s = session();
    for index in 0..64_i64 {
        let y_nm = (index + 100) * 2_540_000;
        apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":y_nm},"to":{"x_nm":2_540_000,"y_nm":y_nm},"bends":[]}));
    }
    let mut snapshot = s.snapshot().unwrap();
    let project = Arc::make_mut(&mut snapshot);
    let sheet_data = &mut Arc::make_mut(&mut project.sheets)[0];
    let mut legs: Vec<crate::wire::GeneratedLeg> = sheet_data
        .wires
        .iter()
        .filter(|wire| wire.bends().is_some())
        .map(|wire| {
            let [other, end] = wire.endpoints();
            crate::wire::GeneratedLeg { wire: wire.ordinal(), end, other, bends: wire.bends().map(|bends| bends.to_vec()) }
        })
        .collect();
    let live_count = legs.len();
    assert!(live_count >= 64);
    let wire_count = sheet_data.wires.len();
    let sample = legs[0].clone();
    legs.push(crate::wire::GeneratedLeg { wire: 999_999, end: sample.end, other: sample.other, bends: sample.bends.clone() });
    legs.push(crate::wire::GeneratedLeg { wire: sample.wire, end: sample.other, other: sample.end, bends: sample.bends });
    let leg_count = legs.len();
    sheet_data.generated_legs = Arc::new(legs);

    let expected = encode_documents(snapshot.clone()).unwrap();
    let sheet_path = sheet(&expected);
    let parsed_sheet: Value = serde_json::from_slice(&expected[&sheet_path]).unwrap();
    assert_eq!(parsed_sheet["generated_legs"].as_array().unwrap().len(), live_count);

    let batch = 8;
    let mut encoder = DocumentsEncoder::new(snapshot.clone());
    let mut wire_index_steps = 0_usize;
    let mut leg_match_steps = 0_usize;
    let mut prev_wire_cursor = 0_usize;
    let mut prev_leg_cursor = 0_usize;
    // Phase 25 drains the wire index one entry per record, so it runs with a one-record budget:
    // each step must remove exactly one entry, and the phase ends only after the index is empty.
    let mut budget = batch;
    let mut draining: Option<usize> = None;
    let mut drain_steps = 0_usize;
    let mut drains_finished = 0_usize;
    let incremental = loop {
        let encoded = encoder.step(budget).unwrap();
        budget = batch;
        let progress = encoder.generated_leg_scan_progress();
        if let Some(remaining) = draining.take() {
            match progress {
                Some((25, _, indexed, _)) => {
                    assert!(remaining > 0, "an empty wire index must end the drain");
                    assert_eq!(indexed, remaining - 1, "one record drains exactly one wire index entry");
                }
                _ => {
                    assert_eq!(remaining, 0, "the drain ended with wire index entries left");
                    drains_finished += 1;
                }
            }
        }
        if let Some(encoded) = encoded {
            break encoded;
        }
        if let Some((phase, cursor, indexed, matched)) = progress {
            match phase {
                23 => {
                    wire_index_steps += 1;
                    assert!(cursor >= prev_wire_cursor);
                    assert!(cursor - prev_wire_cursor <= batch);
                    assert_eq!(indexed, cursor, "each scanned wire adds one index entry");
                    prev_wire_cursor = cursor;
                }
                24 => {
                    leg_match_steps += 1;
                    assert!(cursor >= prev_leg_cursor);
                    assert!(cursor - prev_leg_cursor <= batch);
                    assert!(matched <= cursor);
                    assert_eq!(indexed, wire_count, "leg matching keeps the whole wire index");
                    prev_leg_cursor = cursor;
                    // Stop at the last leg so the step that enters phase 25 drains nothing.
                    budget = (leg_count - cursor).clamp(1, batch);
                }
                25 => {
                    if drain_steps == 0 {
                        assert_eq!(indexed, wire_count, "entering the drain must not remove entries");
                    }
                    drain_steps += 1;
                    assert!(drain_steps <= wire_count + 1, "the wire index drain must end within one step per entry");
                    draining = Some(indexed);
                    budget = 1;
                }
                phase => unreachable!("generated-leg scan reported phase {phase}"),
            }
        }
    };
    assert!(wire_index_steps >= 6, "wire indexing should span multiple bounded steps");
    assert!(leg_match_steps >= 6, "generated-leg matching should span multiple bounded steps");
    assert_eq!(drains_finished, 1, "the wire index drain should finish exactly once");
    assert_eq!(drain_steps, wire_count + 1, "the drain should take one step per wire index entry");
    assert_eq!(incremental, expected);

    // Cancel strictly mid-leg during phase 24 and verify a subsequent save is unaffected.
    let mut cancelled = DocumentsEncoder::new(snapshot.clone());
    let mut paused_mid_leg = false;
    for _ in 0..512 {
        assert!(cancelled.step(4).unwrap().is_none());
        if let Some((24, cursor, _indexed, matched)) = cancelled.generated_leg_scan_progress()
            && cursor > 0
            && cursor < live_count
            && matched < live_count
        {
            paused_mid_leg = true;
            break;
        }
    }
    assert!(paused_mid_leg, "encoder should yield mid-leg before finishing phase 24");
    drop(cancelled);
    assert_eq!(encode_documents(snapshot).unwrap(), expected);

    // Decode uses the indexed wire lookup and round-trips byte-for-byte.
    let reopened = decode_documents(&expected).unwrap();
    assert_eq!(reopened.sheets[0].generated_legs.len(), live_count);
    assert_eq!(encode_documents(reopened).unwrap(), expected);
}

/// Native board copper serialization (tracks and vias) streams incrementally within the caller's
/// step budget, can be cancelled mid-tracks or mid-vias without affecting the snapshot, and
/// produces byte-exact `board.json` output matching the canonical array framing.
#[test]
fn board_copper_encoding_is_step_bounded_cancellable_and_byte_exact() {
    use crate::{ADD_TRACKS, ADD_VIA, INITIALIZE_BOARD_SETUP, board::setup::LayerPurpose};

    const TRACK_COUNT: usize = 64;
    const VIA_COUNT: usize = 48;
    const BUDGET: usize = 5;

    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Copper streaming","create_board":true})).unwrap();
    apply(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    let front = s.project().unwrap().board_setup().unwrap().layers().iter().find(|layer| layer.purpose == LayerPurpose::Copper).unwrap().id;
    for i in 0..TRACK_COUNT {
        let y = (i as i64) * 2_000_000;
        apply(&mut s, ADD_TRACKS, json!({"layer":front,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":y},{"x_nm":5_000_000,"y_nm":y}]}));
    }
    for i in 0..VIA_COUNT {
        let x = 10_000_000 + (i as i64) * 2_000_000;
        apply(&mut s, ADD_VIA, json!({"at":{"x_nm":x,"y_nm":0},"via_diameter_nm":600_000,"via_drill_nm":300_000}));
    }
    let snapshot = s.snapshot().unwrap();
    assert_eq!(snapshot.board.tracks.len(), TRACK_COUNT);
    assert_eq!(snapshot.board.vias.len(), VIA_COUNT);

    let mut encoder = DocumentsEncoder::new(snapshot.clone());
    let mut track_steps = 0usize;
    let mut via_steps = 0usize;
    let mut mid_tracks_seen = false;
    let mut mid_vias_seen = false;
    let mut prev_tracks: Option<usize> = None;
    let mut prev_vias: Option<usize> = None;
    let incremental = loop {
        if let Some(docs) = encoder.step(BUDGET).unwrap() {
            break docs;
        }
        match encoder.board_copper_progress() {
            Some(("tracks", written)) => {
                track_steps += 1;
                if let Some(prev) = prev_tracks {
                    assert!(written >= prev && written - prev <= BUDGET, "track step exceeded budget: {prev} -> {written}");
                } else {
                    assert!(written <= BUDGET, "initial track step exceeded budget: {written}");
                }
                if written > 0 && written < TRACK_COUNT {
                    mid_tracks_seen = true;
                }
                prev_tracks = Some(written);
            }
            Some(("vias", written)) => {
                via_steps += 1;
                if let Some(prev) = prev_vias {
                    assert!(written >= prev && written - prev <= BUDGET, "via step exceeded budget: {prev} -> {written}");
                } else {
                    assert!(written <= BUDGET, "initial via step exceeded budget: {written}");
                }
                if written > 0 && written < VIA_COUNT {
                    mid_vias_seen = true;
                }
                prev_vias = Some(written);
            }
            _ => {}
        }
    };
    assert!(mid_tracks_seen, "encoder must yield mid-tracks");
    assert!(mid_vias_seen, "encoder must yield mid-vias");
    assert!(track_steps >= TRACK_COUNT / BUDGET, "too few track steps: {track_steps}");
    assert!(via_steps >= VIA_COUNT / BUDGET, "too few via steps: {via_steps}");

    for (target_kind, target_total) in [("tracks", TRACK_COUNT), ("vias", VIA_COUNT)] {
        let mut cancelled = DocumentsEncoder::new(snapshot.clone());
        let mut cancelled_mid = false;
        for _ in 0..512 {
            assert!(cancelled.step(BUDGET).unwrap().is_none(), "finished before mid-{target_kind} pause");
            if let Some((kind, written)) = cancelled.board_copper_progress()
                && kind == target_kind
                && written > 0
                && written < target_total
            {
                cancelled_mid = true;
                break;
            }
        }
        assert!(cancelled_mid, "did not reach mid-{target_kind} state to cancel");
        drop(cancelled);
    }

    let reference = encode_documents(snapshot.clone()).unwrap();
    assert_eq!(incremental, reference);
    let mut one_by_one = DocumentsEncoder::new(snapshot.clone());
    let single_step = loop {
        if let Some(docs) = one_by_one.step(1).unwrap() {
            break docs;
        }
    };
    assert_eq!(single_step, reference);
    check_board_document_size(&snapshot).unwrap();

    let board_path = snapshot.board_document_path().unwrap();
    let board_text = std::str::from_utf8(&incremental[&board_path]).unwrap();
    let mut expected_copper = String::from("  \"tracks\": [\n");
    for (index, track) in snapshot.board.tracks.iter().enumerate() {
        if index > 0 {
            expected_copper.push_str("\n    ,\n");
        }
        expected_copper.push_str("    ");
        expected_copper.push_str(&serde_json::to_string(track).unwrap());
        expected_copper.push('\n');
    }
    expected_copper.push_str("  ],\n  \"vias\": [\n");
    for (index, via) in snapshot.board.vias.iter().enumerate() {
        if index > 0 {
            expected_copper.push_str("\n    ,\n");
        }
        expected_copper.push_str("    ");
        expected_copper.push_str(&serde_json::to_string(via).unwrap());
        expected_copper.push('\n');
    }
    expected_copper.push_str("  ],\n  \"footprints\": [\n");
    assert!(board_text.contains(&expected_copper), "board.json copper formatting drifted");
    assert_eq!(encode_documents(decode_documents(&incremental).unwrap()).unwrap(), incremental);
}

/// Native document encoding meters net-registry graph rebuild, record/lineage validation,
/// compact 8 MiB byte metering, and Phase 16 `RecordEncoding` emission across budgeted steps,
/// tolerates mid-validation and mid-record cancellation without mutating the snapshot, and
/// matches Serde byte-for-byte.
#[test]
fn documents_encoder_steps_large_net_lineage_incrementally_and_matches_serde() {
    const LINEAGE_LEN: usize = 64;
    for board in [false, true] {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board": board})).unwrap();
        let mut pins = Vec::new();
        for index in 0..4 {
            apply(&mut s, crate::PLACE_RESISTOR, json!({"x_nm": index as i64 * 25_400_000, "y_nm": 0}));
            pins.push(json!({"symbol": s.project().unwrap().symbols().last().unwrap().id(), "terminal": "1"}));
        }
        apply(&mut s, crate::CONNECT_PINS, json!({"from": pins[0], "to": pins[1]}));
        apply(&mut s, crate::CONNECT_PINS, json!({"from": pins[2], "to": pins[3]}));
        apply(&mut s, crate::nets::PROMOTE, json!({"anchor": pins[0]}));
        apply(&mut s, crate::nets::PROMOTE, json!({"anchor": pins[2]}));

        let small = s.snapshot().unwrap();
        let mut large_project = (*small).clone();
        let registry = Arc::make_mut(&mut large_project.net_registry);
        assert_eq!(registry.records.len(), 2);
        let second_id = registry.records[1].id;
        let anchor = registry.records[0].primary.clone();
        registry.records[0].lineage = (0..LINEAGE_LEN)
            .map(|index| crate::nets::Lineage {
                snapshot: crate::board::update::SourceCapture { capture_id: Uuid::from_u128(100 + index as u128), content_digest: "a".repeat(64) },
                net: (index % 2 == 0).then_some(second_id),
                anchor: anchor.clone(),
            })
            .collect();
        let large = Arc::new(large_project);

        let mut small_encoder = DocumentsEncoder::new(small);
        let mut small_steps = 0usize;
        while small_encoder.step(1).unwrap().is_none() {
            small_steps += 1;
        }

        let mut large_encoder = DocumentsEncoder::new(large.clone());
        let mut large_steps = 0usize;
        let mut build_graph_steps = 0usize;
        let mut validate_record_steps = 0usize;
        let mut meter_record_steps = 0usize;
        let incremental_1 = loop {
            if let Some(docs) = large_encoder.step(1).unwrap() {
                break docs;
            }
            match large_encoder.registry_validation_stage() {
                Some("build_graph") => build_graph_steps += 1,
                Some("validate_records") => validate_record_steps += 1,
                Some("meter_records") => meter_record_steps += 1,
                _ => {}
            }
            large_steps += 1;
        };
        assert!(build_graph_steps > 1);
        assert!(validate_record_steps >= 2 * LINEAGE_LEN);
        assert!(meter_record_steps >= LINEAGE_LEN);
        assert_eq!(large_steps, small_steps + 4 * LINEAGE_LEN + 4);

        for target_stage in ["validate_records", "meter_records"] {
            let mut cancelled = DocumentsEncoder::new(large.clone());
            while cancelled.registry_validation_stage() != Some(target_stage) {
                assert!(cancelled.step(1).unwrap().is_none());
            }
            for _ in 0..10 {
                assert!(cancelled.step(1).unwrap().is_none());
                assert_eq!(cancelled.registry_validation_stage(), Some(target_stage));
            }
            drop(cancelled);
        }

        let mut cancelled = DocumentsEncoder::new(large.clone());
        for _ in 0..large_steps / 2 {
            assert!(cancelled.step(1).unwrap().is_none());
        }
        drop(cancelled);

        let mut resumed = DocumentsEncoder::new(large.clone());
        let incremental_7 = loop {
            if let Some(docs) = resumed.step(7).unwrap() {
                break docs;
            }
        };

        let one_shot = encode_documents(large.clone()).unwrap();
        assert_eq!(incremental_1, one_shot);
        assert_eq!(incremental_7, one_shot);

        let path = large.instances_document_path().unwrap_or_else(|| ENTRY_POINT.to_owned());
        let text = std::str::from_utf8(&one_shot[&path]).unwrap();
        let first_row = serde_json::to_string(&large.net_registry.records[0]).unwrap();
        let second_row = serde_json::to_string(&large.net_registry.records[1]).unwrap();
        let expected_pair = format!("    {first_row}\n\n    ,\n    {second_row}\n");
        assert!(text.contains(&expected_pair));

        let reopened = decode_documents(&incremental_1).unwrap();
        assert_eq!(reopened.net_registry, large.net_registry);
        assert_eq!(encode_documents(reopened).unwrap(), one_shot);
    }
}
