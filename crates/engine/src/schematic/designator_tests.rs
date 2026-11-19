//! Designator locks are instance intent, independent of drawing geometry and physical ownership.
use crate::{CommandError, PLACE_SYMBOL, SET_SYMBOL_FIELDS, Session, clipboard, persistence};
use serde_json::{Value, json};
use std::sync::Arc;

mod edges;

/// Build independent resistor instances through the ordinary placement command.
fn fixture(board: bool) -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":board})).unwrap();
    for x in [0, 25_400_000] {
        edit(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":x,"y_nm":0}));
    }
    session
}

/// Submit one edit against the session's current revision.
fn edit(session: &mut Session, command: &str, mut params: Value) {
    if params.is_object() {
        params["expected_revision"] = json!(session.revision());
    }
    session.apply(command, &params).unwrap();
}

/// Change only the first instance's lock, retaining its visible fields.
fn lock(session: &mut Session, locked: bool) {
    edit(session, SET_SYMBOL_FIELDS, json!({"symbol":"symbol-2","reference":"R1","value":"R","designator_locked":locked}));
}

/// Use the production folder codec for byte and document-ownership comparisons.
fn documents(session: &Session) -> persistence::Documents {
    persistence::encode_documents(session.snapshot().unwrap()).unwrap()
}

/// Corrupt only a command-generated entry point to exercise reader refusal paths.
fn entry_edit(documents: &mut persistence::Documents, edit: impl FnOnce(&mut Value)) {
    let mut entry: Value = serde_json::from_slice(&documents[persistence::ENTRY_POINT]).unwrap();
    edit(&mut entry);
    documents.insert(persistence::ENTRY_POINT.into(), serde_json::to_vec(&entry).unwrap());
}

/// Set/save/reopen retains the lock and its eligibility predicate, while another instance
/// stays unlocked. Undo restores exact prior bytes and enabling an existing lock is a no-op.
#[test]
fn designator_lock_roundtrip_eligibility_and_one_step_undo() {
    for board in [false, true] {
        let mut session = fixture(board);
        let before = documents(&session);
        let revision = session.revision();
        lock(&mut session, true);
        assert_eq!(session.revision(), revision + 1);
        let locked = documents(&session);
        assert_eq!(
            locked.iter().filter(|(path, bytes)| before.get(*path) != Some(*bytes)).map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
            [persistence::ENTRY_POINT]
        );
        let entry: Value = serde_json::from_slice(&locked[persistence::ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/51");
        assert_eq!(entry["components"][0]["designator_locked"], true);
        assert!(entry["components"][1].get("designator_locked").is_none());
        let reopened = persistence::decode_documents(&locked).unwrap();
        assert_eq!(persistence::encode_documents(reopened.clone()).unwrap(), locked);
        let flags: Vec<_> = reopened.components().map(|part| (part.designator_locked(), part.can_annotate_designator())).collect();
        assert_eq!(flags, [(true, false), (false, true)]);
        let packed = persistence::encode_packed(reopened).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_packed(&packed).unwrap().project).unwrap(), locked);
        assert_eq!(session.inspect()["project"]["components"][0]["designator_locked"], true);
        assert_eq!(session.inspect()["project"]["components"][1]["designator_locked"], false);
        let revision = session.revision();
        lock(&mut session, true);
        assert_eq!(session.revision(), revision);
        edit(&mut session, "edit.undo", Value::Null);
        assert_eq!(documents(&session), before);
        edit(&mut session, "edit.redo", Value::Null);
        assert_eq!(documents(&session), locked);
        lock(&mut session, false);
        assert_eq!(documents(&session), before);
    }
}

/// Omitting a lock retains it, and locking does not forbid deliberate reference edits.
#[test]
fn designator_lock_does_not_block_manual_reference_edits() {
    let mut session = fixture(false);
    lock(&mut session, true);
    edit(&mut session, SET_SYMBOL_FIELDS, json!({"symbol":"symbol-2","reference":"R7","value":"10k"}));
    let part = session.project().unwrap().components().next().unwrap();
    assert_eq!(part.reference(), "R7");
    assert!(part.designator_locked());
    assert!(!part.can_annotate_designator());
}

/// Lock metadata preparation is entered only when an edit adds intent, never for unrelated
/// edits. Count entries rather than comparing elapsed time or inferring work from saved bytes.
#[test]
fn designator_lock_unrelated_edits_skip_metadata_preparation() {
    let mut session = fixture(false);
    let other = session.project().unwrap().symbols()[1].id();
    crate::DESIGNATOR_METADATA_PREPARATIONS.with(|calls| {
        calls.set(0);
        edit(&mut session, SET_SYMBOL_FIELDS, json!({"symbol":other,"reference":"R2","value":"1k"}));
        assert_eq!(calls.get(), 0, "lock-free field edit must not inspect any lock flags");
        lock(&mut session, true);
        assert_eq!(calls.get(), 1, "only the addressed part introduced a lock");
        let before = session.snapshot().unwrap();
        edit(&mut session, crate::MOVE_SYMBOL, json!({"symbol":other,"x_nm":50_800_000,"y_nm":0}));
        assert!(Arc::ptr_eq(&before.components, &session.project().unwrap().components));
        assert_eq!(calls.get(), 1, "geometry edit must not enter lock preparation");
        edit(&mut session, SET_SYMBOL_FIELDS, json!({"symbol":other,"reference":"R8","value":"1k"}));
        assert_eq!(calls.get(), 1, "field edit on another part must not enter lock preparation");
        let project = session.project().unwrap();
        assert!(project.components().next().unwrap().designator_locked());
        assert_eq!(project.components().last().unwrap().reference(), "R8");
        lock(&mut session, false);
        assert_eq!(calls.get(), 1, "clearing intent does not adopt metadata");
    });
}

/// A lock-only preview allocates no net identities and its proposed bytes match Apply.
/// No preview publishes a snapshot, revision, observer event or undo entry, including a no-op.
#[test]
fn designator_lock_dry_run_matches_apply_without_mutation() {
    #[derive(Debug, Default)]
    struct Observer(std::sync::Mutex<Vec<(String, u64)>>);

    impl crate::edits::EditObserver for Observer {
        /// Keep root notifications separate from the commands committed afterward.
        fn opened(&self, _: Arc<crate::Project>) {
            self.0.lock().unwrap().push(("opened".into(), 0));
        }

        /// Record each published command and its session revision.
        fn committed(&self, edit: crate::edits::CommittedEdit) {
            self.0.lock().unwrap().push((edit.command, edit.revision));
        }
    }

    let mut session = fixture(false);
    let observer = Arc::new(Observer::default());
    session.observe_edits(Some(observer.clone()));
    assert_eq!(*observer.0.lock().unwrap(), [("opened".into(), 0)]);
    observer.0.lock().unwrap().clear();
    let before = documents(&session);
    let snapshot = session.snapshot().unwrap();
    let revision = session.revision();
    let params = json!({"symbol":"symbol-2","reference":"R1","value":"R","designator_locked":true,"dry_run":true,"expected_revision":revision});
    let preview = session.execute(SET_SYMBOL_FIELDS, &params).unwrap();
    assert_eq!(preview["revision"], revision);
    assert_eq!(preview["provisional"], true);
    assert_eq!(preview["changes"].as_array().unwrap().len(), 1);
    assert_eq!(preview["changes"][0]["path"], persistence::ENTRY_POINT);
    assert_eq!(preview["changes"][0]["before"], std::str::from_utf8(&before[persistence::ENTRY_POINT]).unwrap());
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(session.apply(SET_SYMBOL_FIELDS, &params).unwrap(), revision);
    assert!(observer.0.lock().unwrap().is_empty());
    lock(&mut session, true);
    assert_eq!(*observer.0.lock().unwrap(), [(SET_SYMBOL_FIELDS.into(), revision + 1)]);
    assert_eq!(preview["changes"][0]["after"], std::str::from_utf8(&documents(&session)[persistence::ENTRY_POINT]).unwrap());
    let mut params = params;
    params["expected_revision"] = json!(session.revision());
    assert_eq!(session.execute(SET_SYMBOL_FIELDS, &params).unwrap()["changes"], json!([]));
    assert_eq!(session.apply(SET_SYMBOL_FIELDS, &params).unwrap(), revision + 1);
    lock(&mut session, true);
    assert_eq!(*observer.0.lock().unwrap(), [(SET_SYMBOL_FIELDS.into(), revision + 1)]);
    edit(&mut session, "edit.undo", Value::Null);
    assert_eq!(documents(&session), before);
    assert_eq!(*observer.0.lock().unwrap(), [(SET_SYMBOL_FIELDS.into(), revision + 1), ("edit.undo".into(), revision + 2)]);
}

/// A legal property move inserts a part into a wire and proposes a complete valid split.
/// Its net/capture UUIDs are provisional, while authored intent matches a later Apply.
/// Apply adds one history step even when allocated identities advance the revision further.
#[test]
fn designator_lock_topology_preview_has_provisional_identities() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let g = crate::schematic::GRID_NM;
    for (x, y) in [(0, 0), (40, 0), (20, 20)] {
        edit(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":x*g,"y_nm":y*g}));
    }
    let symbols = session.project().unwrap().symbols();
    let from = symbols[0].terminals()[1].inspect();
    let to = symbols[1].terminals()[0].inspect();
    let symbol = symbols[2].id();
    edit(&mut session, crate::DRAW_WIRE, json!({"from":from,"to":to,"bends":[]}));
    let before = documents(&session);
    let snapshot = session.snapshot().unwrap();
    assert_eq!(snapshot.net_registry.records.len(), 1);
    let history = (session.undo.len(), session.redo.len());
    let revision = session.revision();
    let mut params = json!({"symbol":symbol,"reference":"R3","value":"R","designator_locked":true,
        "x_nm":20*g,"y_nm":0,"dry_run":true,"expected_revision":revision});
    let preview = session.execute(SET_SYMBOL_FIELDS, &params).unwrap();
    assert_eq!(preview["revision"], revision);
    assert_eq!(preview["provisional"], true);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert_eq!(documents(&session), before);
    let mut proposed_documents = before.clone();
    for change in preview["changes"].as_array().unwrap() {
        let path = change["path"].as_str().unwrap();
        assert_eq!(change["before"], std::str::from_utf8(&before[path]).unwrap());
        proposed_documents.insert(path.into(), change["after"].as_str().unwrap().as_bytes().to_vec());
    }
    let proposed = persistence::decode_documents(&proposed_documents).unwrap();
    assert_eq!(persistence::encode_documents(proposed.clone()).unwrap(), proposed_documents);
    assert_eq!(proposed.wires().len(), 2);
    assert_eq!(proposed.connectivity().nets.len(), 2);
    assert!(proposed.components().last().unwrap().designator_locked());
    // Compound wire edits advance the revision past every numeric identity they allocate.
    let expected_revision = (revision + 1).max(proposed.highest_id());
    assert!(expected_revision > revision + 1);
    params["dry_run"] = json!(false);
    edit(&mut session, SET_SYMBOL_FIELDS, params.clone());
    let applied = session.snapshot().unwrap();
    let actual_documents = documents(&session);
    assert_eq!(session.revision(), expected_revision);
    assert_eq!((session.undo.len(), session.redo.len()), (history.0 + 1, 0));
    let proposal_branch = proposed.net_registry.records.iter().find(|record| !record.lineage.is_empty()).unwrap();
    let applied_branch = applied.net_registry.records.iter().find(|record| record.primary == proposal_branch.primary).unwrap();
    assert_ne!(proposal_branch.id, applied_branch.id);
    assert_ne!(proposal_branch.lineage[0].snapshot.capture_id, applied_branch.lineage[0].snapshot.capture_id);
    assert_eq!(proposal_branch.lineage[0].snapshot.content_digest, applied_branch.lineage[0].snapshot.content_digest);
    assert_eq!(proposal_branch.lineage[0].net, applied_branch.lineage[0].net);
    assert_eq!(proposal_branch.lineage[0].anchor, applied_branch.lineage[0].anchor);
    assert_eq!(proposed.net_registry.records.len(), applied.net_registry.records.len());
    // Only the provisional identity registry differs; all other native authored bytes agree.
    let mut normalized = crate::Project::clone(&proposed);
    normalized.net_registry = applied.net_registry.clone();
    assert_eq!(persistence::encode_documents(Arc::new(normalized)).unwrap(), actual_documents);
    let mut missing_number = actual_documents.clone();
    entry_edit(&mut missing_number, |entry| {
        entry["schematic_instances"]["net_registry"]["records"][0].as_object_mut().unwrap().remove("generated_number");
    });
    let error = persistence::decode_documents(&missing_number).unwrap_err();
    assert!(error.contains("Generated net numbers require project format 23, 24, 36, 39 or 40") && error.contains("every record"), "{error}");
    params["dry_run"] = json!(true);
    let error = session.execute(SET_SYMBOL_FIELDS, &params).unwrap_err();
    assert_eq!(error, CommandError::StaleRevision { current: session.revision() });
    assert!(Arc::ptr_eq(&applied, &session.snapshot().unwrap()));
    assert_eq!((session.undo.len(), session.redo.len()), (history.0 + 1, 0));
    edit(&mut session, "edit.undo", Value::Null);
    assert_eq!((session.undo.len(), session.redo.len()), (history.0, 1));
    crate::assert_same_content(&snapshot, &session.snapshot().unwrap());
    edit(&mut session, "edit.redo", Value::Null);
    assert_eq!((session.undo.len(), session.redo.len()), (history.0 + 1, 0));
    assert_eq!(documents(&session), actual_documents);
}

/// Boolean-only input, unknown fields, unknown targets and stale authority refuse the whole edit.
#[test]
fn designator_lock_invalid_commands_preserve_snapshot() {
    let mut session = fixture(false);
    let snapshot = session.snapshot().unwrap();
    for preview in [false, true] {
        for field in ["designator_locked", "dry_run"] {
            for value in [Value::Null, json!("true"), json!(0), json!(-1), json!(1.0), json!([]), json!({})] {
                let mut params = json!({"symbol":"symbol-2","reference":"R7","value":"10k","designator_locked":true,
                    "dry_run":preview,"expected_revision":session.revision()});
                params[field] = value;
                let error = session.apply(SET_SYMBOL_FIELDS, &params).unwrap_err();
                assert!(
                    matches!(&error, CommandError::InvalidParams(message) if message.contains(field) && message.contains("boolean")),
                    "{field}: {error}"
                );
                assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
            }
        }
        use crate::error_codes::ErrorCode;
        for (extra, reason, code) in [
            (json!({"unexpected":true}), "unexpected", ErrorCode::InvalidParams),
            (json!({"symbol":"symbol-999"}), "symbol does not exist", ErrorCode::NotFound),
            (json!({"expected_revision":0,"unexpected":true}), "project changed", ErrorCode::StaleRevision),
        ] {
            let mut params = json!({"symbol":"symbol-2","reference":"R7","value":"10k","designator_locked":true,
                "dry_run":preview,"expected_revision":session.revision()});
            params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let error = session.execute(SET_SYMBOL_FIELDS, &params).unwrap_err();
            assert_eq!(error.code(), code, "{error}");
            assert!(error.to_string().contains(reason), "{error}");
            if code == ErrorCode::StaleRevision {
                assert_eq!(error.details(), json!({"current_revision":session.revision()}));
            }
            assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
        }
    }
}

/// Lock fields are sparse true-only records in project/51; older and unsupported headers,
/// missing flags, explicit false/null, wrong types, duplicate keys and unknown fields are refused.
#[test]
fn designator_lock_codec_rejects_noncanonical_and_older_records() {
    let mut session = fixture(false);
    let unlocked = documents(&session);
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&unlocked).unwrap()).unwrap(), unlocked);
    lock(&mut session, true);
    let locked = documents(&session);
    for (value, reason) in
        [(json!(false), "omit unlocked"), (Value::Null, "null"), (json!(1), "boolean"), (json!("true"), "boolean"), (json!([]), "boolean")]
    {
        let mut bad = locked.clone();
        entry_edit(&mut bad, |entry| entry["components"][0]["designator_locked"] = value);
        let error = persistence::decode_documents(&bad).unwrap_err();
        assert!(error.contains(reason), "{error}");
    }
    for header in
        ["pcbcraft-project/16", "pcbcraft-project/23", "pcbcraft-project/24", "pcbcraft-project/30", "pcbcraft-project/39", "pcbcraft-project/999"]
    {
        let mut bad = locked.clone();
        entry_edit(&mut bad, |entry| entry["format"] = json!(header));
        let error = persistence::decode_documents(&bad).unwrap_err();
        assert!(error.contains("Designator locks require project format 51"), "{error}");
    }
    let mut missing = unlocked.clone();
    entry_edit(&mut missing, |entry| entry["format"] = json!("pcbcraft-project/51"));
    assert!(persistence::decode_documents(&missing).unwrap_err().contains("at least one true"));
    let mut unknown = locked.clone();
    entry_edit(&mut unknown, |entry| entry["components"][0]["unit_locked"] = json!(true));
    assert!(persistence::decode_documents(&unknown).unwrap_err().contains("unknown field `unit_locked`"));
    let text = std::str::from_utf8(&locked[persistence::ENTRY_POINT]).unwrap();
    let mut duplicate = locked.clone();
    duplicate.insert(
        persistence::ENTRY_POINT.into(),
        text.replace("\"designator_locked\":true", "\"designator_locked\":true,\"designator_locked\":true").into_bytes(),
    );
    assert!(persistence::decode_documents(&duplicate).unwrap_err().contains("duplicate field `designator_locked`"));
    assert!(persistence::encode(session.snapshot().unwrap()).unwrap_err().contains("Designator locks require split project format 51"));
}

/// Copy preserves intent while duplication allocates a fresh reference; preserve-reference
/// paste into a project without occurrence metadata retains the text and adopts storage.
/// Unlocked fragments still use format 1.
#[test]
fn designator_lock_clipboard_preserves_intent_and_lazy_envelopes() {
    let mut source = fixture(false);
    let params = json!({"items":{"symbols":["symbol-2"],"wires":[]},"expected_revision":source.revision()});
    let plain = source.execute(clipboard::COPY, &params).unwrap()["clipboard"].as_str().unwrap().to_owned();
    assert_eq!(serde_json::from_str::<Value>(&plain).unwrap()["format"], "pcbcraft-clipboard/1");
    lock(&mut source, true);
    let text = source.execute(clipboard::COPY, &json!({"items":{"symbols":["symbol-2"],"wires":[]},"expected_revision":source.revision()})).unwrap()
        ["clipboard"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["format"], "pcbcraft-clipboard/3");
    edit(&mut source, clipboard::PASTE, json!({"text":text,"x_nm":50_800_000,"y_nm":0}));
    let copied = source.project().unwrap().components().last().unwrap();
    assert_eq!(copied.reference(), "R3");
    assert!(copied.designator_locked());
    let mut destination = Session::default();
    destination.apply("project.new", &json!({"create_board":false})).unwrap();
    let mut historical = crate::Project::clone(&persistence::diagnostic_snapshot(destination.snapshot().unwrap()));
    historical.layout = None;
    destination.open_snapshot(Arc::new(historical));
    assert!(!destination.project().unwrap().has_sheet_occurrences());
    edit(&mut destination, clipboard::PASTE, json!({"text":text,"x_nm":0,"y_nm":0,"references":"preserve"}));
    assert!(destination.project().unwrap().has_sheet_occurrences());
    let saved = documents(&destination);
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
    edit(&mut destination, "edit.undo", Value::Null);
    assert!(destination.project().unwrap().layout.is_none());
    assert!(!destination.project().unwrap().has_sheet_occurrences());
    edit(&mut destination, "edit.redo", Value::Null);
    assert_eq!(documents(&destination), saved);
    let part = destination.project().unwrap().components().next().unwrap();
    assert_eq!(part.reference(), "R1");
    assert!(part.designator_locked());
    let mut envelope: Value = serde_json::from_str(&text).unwrap();
    for format in ["pcbcraft-clipboard/1", "pcbcraft-clipboard/2"] {
        envelope["format"] = json!(format);
        let error = clipboard::Fragment::from_text(&envelope.to_string()).unwrap_err();
        assert!(matches!(&error, CommandError::InvalidParams(message) if message.contains("Designator locks require clipboard format 3")), "{error}");
    }
    let mut envelope: Value = serde_json::from_str(&plain).unwrap();
    envelope["format"] = json!("pcbcraft-clipboard/3");
    let error = clipboard::Fragment::from_text(&envelope.to_string()).unwrap_err();
    assert!(matches!(&error, CommandError::InvalidParams(message) if message.contains("at least one locked")), "{error}");
}

/// Project parameters, class bindings and generated net numbers coexist with locks in either
/// instance storage home, retaining their existing instance-document vocabulary.
#[test]
fn designator_locks_compose_with_project_parameters_and_numbered_nets() {
    for board in [false, true] {
        let mut session = fixture(board);
        edit(&mut session, crate::parameters::SET, json!({"scope":"project","name":"Revision","value":"B"}));
        edit(&mut session, crate::net_classes::CREATE, json!({"name":"Signal","rules":{"clearance_nm":200_000}}));
        let terminal = session.project().unwrap().symbols()[0].terminals()[0];
        let target = session.project().unwrap().no_connect_target(terminal).unwrap();
        edit(&mut session, crate::no_connect::SET, json!({"target":target,"marked":true}));
        let symbols = session.project().unwrap().symbols();
        let from = symbols[0].terminals()[1];
        let to = symbols[1].terminals()[0];
        edit(&mut session, crate::DRAW_WIRE, json!({"from":from.inspect(),"to":to.inspect(),"bends":[]}));
        let project = session.project().unwrap();
        let net = project.net_registry.records[0].id;
        let target = crate::nets::ClassTarget::Persistent { project: project.id, scope: project.net_scope(), net };
        let class = project.net_classes().definitions[0].id;
        edit(&mut session, crate::nets::SET_CLASSES, json!({"target":target,"classes":[class]}));
        lock(&mut session, true);
        let saved = documents(&session);
        let reopened = persistence::decode_documents(&saved).unwrap();
        assert_eq!(persistence::encode_documents(reopened.clone()).unwrap(), saved);
        assert_eq!(reopened.parameters().get("Revision").map(String::as_str), Some("B"));
        assert!(reopened.net_registry.records.iter().any(|record| !record.class_bindings.is_empty()));
        assert!(reopened.components().next().unwrap().designator_locked());
        for version in 1..=4 {
            let mut historical = crate::Project::clone(&reopened);
            if version == 1 {
                historical.net_registry = Arc::default();
            } else {
                for record in &mut Arc::make_mut(&mut historical.net_registry).records {
                    if version < 4 {
                        record.generated_number = None;
                    }
                    if version == 2 {
                        record.class_bindings.clear();
                    }
                }
            }
            let variants = persistence::encode_documents(Arc::new(historical)).unwrap();
            let entry: Value = serde_json::from_slice(&variants[persistence::ENTRY_POINT]).unwrap();
            assert_eq!(entry["format"], "pcbcraft-project/51");
            let decoded = persistence::decode_documents(&variants).unwrap();
            if let Some(path) = decoded.instances_document_path() {
                let instance: Value = serde_json::from_slice(&variants[&path]).unwrap();
                assert_eq!(instance["format"], format!("pcbcraft-instances/{version}"));
            }
            assert_eq!(decoded.net_registry.records.is_empty(), version == 1);
            assert_eq!(decoded.net_registry.records.iter().any(|record| !record.class_bindings.is_empty()), version >= 3);
            assert_eq!(decoded.net_registry.records.iter().any(|record| record.generated_number.is_some()), version == 4);
            assert_eq!(persistence::encode_documents(decoded).unwrap(), variants);
        }
        let mut missing_catalog = saved.clone();
        entry_edit(&mut missing_catalog, |entry| {
            entry.as_object_mut().unwrap().remove("net_classes");
        });
        let error = persistence::decode_documents(&missing_catalog).unwrap_err();
        assert!(error.contains("Net class assignments require project format 21") && error.contains("net-class catalog"), "{error}");
        let mut empty_choices = saved.clone();
        entry_edit(&mut empty_choices, |entry| entry["footprint_choices"] = json!([]));
        let error = persistence::decode_documents(&empty_choices).unwrap_err();
        assert!(error.contains("project footprint choices require format") && error.contains("39 or 40") && error.contains("nonempty"), "{error}");
        lock(&mut session, false);
        let after = documents(&session);
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&after).unwrap()).unwrap(), after);
    }
}

/// Locks compose with group catalogs, expanded/compact memberships and pinned profiles.
/// No-board cases load a retained, command-authored catalog; authoring still requires a board.
/// Unlocking restores the lower canonical envelope without changing those records or board bytes.
#[test]
fn designator_locks_preserve_group_representations_and_assembly_profiles() {
    for board in [false, true] {
        for group_mode in 0..=3 {
            for profiles in [false, true] {
                let mut session = fixture(board);
                if group_mode != 0 {
                    edit(&mut session, crate::part_groups::CREATE, json!({"name":"Signal"}));
                    let group = session.project().unwrap().part_groups().groups[0].id;
                    if group_mode >= 2 {
                        let part_instance = session.project().unwrap().part_instance_addresses().next().unwrap();
                        edit(&mut session, crate::part_groups::SET_GROUPS, json!({"part_instance":part_instance,"groups":[group]}));
                    }
                    if group_mode == 3 {
                        let mut project = crate::Project::clone(&session.snapshot().unwrap());
                        project.prefer_compact_part_groups = true;
                        session.open_snapshot(Arc::new(project));
                    }
                }
                if profiles {
                    let content = json!({"expected_revision":session.revision(),"content":{"revision":1,"name":"Assembly","provenance":{"kind":"authored"},"bottom_side":"viewed_from_top"}});
                    if board {
                        edit(&mut session, crate::board::assembly_profile::SET_ASSEMBLY_PROFILE, content);
                    } else {
                        let snapshot = session.snapshot().unwrap();
                        let error = session.apply(crate::board::assembly_profile::SET_ASSEMBLY_PROFILE, &content).unwrap_err();
                        assert!(matches!(error, CommandError::Unavailable(message) if message.contains("can't run now")));
                        assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
                        // Compose a retained catalog from shared commands and the production codec,
                        // rather than invoking an unavailable authoring command or writing project JSON.
                        let mut donor = fixture(true);
                        edit(&mut donor, crate::board::assembly_profile::SET_ASSEMBLY_PROFILE, content);
                        let donor = persistence::decode_documents(&documents(&donor)).unwrap();
                        let mut project = crate::Project::clone(&snapshot);
                        project.assembly_profiles = donor.assembly_profiles.clone();
                        let saved = persistence::encode_documents(Arc::new(project)).unwrap();
                        session.open_snapshot(persistence::decode_documents(&saved).unwrap());
                        assert!(session.project().unwrap().board_target().is_none());
                    }
                    if let Some(target) = session.project().unwrap().board_target() {
                        let profile = session.project().unwrap().assembly_profiles.heads[0].profile;
                        let mut params = json!({"board":target.board,"profile":profile,"revision":1,"expected_revision":session.revision()});
                        let preview = session.execute(crate::board::assembly_profile::PREVIEW_ASSEMBLY_PROFILE, &params).unwrap();
                        params["acceptance"] = preview["acceptance"].clone();
                        edit(&mut session, crate::board::assembly_profile::ADOPT_ASSEMBLY_PROFILE, params);
                    }
                }
                let before = documents(&session);
                let snapshot = session.snapshot().unwrap();
                let original: Value = serde_json::from_slice(&before[persistence::ENTRY_POINT]).unwrap();
                if profiles || group_mode != 0 {
                    let expected = if profiles {
                        36
                    } else if group_mode == 3 {
                        39
                    } else {
                        24
                    };
                    assert_eq!(original["format"], format!("pcbcraft-project/{expected}"));
                }
                lock(&mut session, true);
                let locked = documents(&session);
                let entry: Value = serde_json::from_slice(&locked[persistence::ENTRY_POINT]).unwrap();
                assert_eq!(entry["format"], "pcbcraft-project/51");
                for field in ["part_groups", "assembly_profiles", "schematic_instances"] {
                    assert_eq!(entry.get(field), original.get(field), "{field}");
                }
                for (path, bytes) in &before {
                    if path != persistence::ENTRY_POINT {
                        assert_eq!(&locked[path], bytes, "{path}");
                    }
                }
                let reopened = persistence::decode_documents(&locked).unwrap();
                assert_eq!(reopened.part_groups(), snapshot.part_groups());
                assert_eq!(reopened.part_group_memberships(), snapshot.part_group_memberships());
                assert_eq!(reopened.assembly_profiles, snapshot.assembly_profiles);
                assert_eq!(reopened.prefer_compact_part_groups, snapshot.prefer_compact_part_groups);
                assert_eq!(persistence::encode_documents(reopened.clone()).unwrap(), locked);
                let packed = persistence::encode_packed(reopened).unwrap();
                assert_eq!(persistence::decode_packed(&packed).unwrap().documents, locked);
                if group_mode == 3 {
                    let mut conflicting = locked.clone();
                    let path = snapshot.instances_document_path().unwrap_or_else(|| persistence::ENTRY_POINT.into());
                    let mut record: Value = serde_json::from_slice(&conflicting[&path]).unwrap();
                    let home = if board { &mut record } else { &mut record["schematic_instances"] };
                    home["part_group_memberships"] = serde_json::to_value(snapshot.part_group_memberships().iter().collect::<Vec<_>>()).unwrap();
                    conflicting.insert(path, serde_json::to_vec(&record).unwrap());
                    let error = persistence::decode_documents(&conflicting).unwrap_err();
                    assert!(error.contains("project format 39, 36 or 40") && error.contains("exactly one membership encoding"), "{error}");
                }
                if profiles {
                    let mut downgraded = locked.clone();
                    entry_edit(&mut downgraded, |entry| entry["format"] = json!("pcbcraft-project/36"));
                    assert!(persistence::decode_documents(&downgraded).unwrap_err().contains("Designator locks require project format 51"));
                    let mut empty = locked.clone();
                    entry_edit(&mut empty, |entry| entry["assembly_profiles"] = json!({"heads":[],"revisions":[]}));
                    assert!(persistence::decode_documents(&empty).unwrap_err().contains("nonempty identity catalog"));
                }
                if group_mode != 0 {
                    let mut empty = locked.clone();
                    entry_edit(&mut empty, |entry| entry["part_groups"]["groups"] = json!([]));
                    assert!(persistence::decode_documents(&empty).unwrap_err().contains("Empty part-group catalogs"));
                }
                edit(&mut session, "edit.undo", Value::Null);
                assert_eq!(documents(&session), before);
                edit(&mut session, "edit.redo", Value::Null);
                assert_eq!(documents(&session), locked);
                lock(&mut session, false);
                assert_eq!(documents(&session), before);
            }
        }
    }
}

/// A first lock adopts missing historical occurrence metadata in the same transaction.
/// For these already-watermarked snapshots, Undo recovers the unadopted original bytes.
#[test]
fn designator_lock_adopts_legacy_metadata_without_changing_an_unlocked_save() {
    for board in [false, true] {
        let mut session = fixture(board);
        let historical = persistence::diagnostic_snapshot(session.snapshot().unwrap());
        session.open_snapshot(historical);
        let before = documents(&session);
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&before).unwrap()).unwrap(), before);
        let revision = session.revision();
        let preview = session
            .execute(
                SET_SYMBOL_FIELDS,
                &json!({"symbol":"symbol-2","reference":"R1","value":"R","designator_locked":true,
            "dry_run":true,"expected_revision":revision}),
            )
            .unwrap();
        assert_eq!(documents(&session), before);
        lock(&mut session, true);
        let after = documents(&session);
        for delta in preview["changes"].as_array().unwrap() {
            let path = delta["path"].as_str().unwrap();
            assert_eq!(delta["after"], std::str::from_utf8(&after[path]).unwrap());
        }
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&after).unwrap()).unwrap(), after);
        edit(&mut session, "edit.undo", Value::Null);
        assert_eq!(documents(&session), before);
    }
}

/// A genuinely pre-watermark project gains allocation metadata on first lock. Undo restores
/// authored content and the earlier storage home, but must retain that monotonic watermark.
#[test]
fn designator_lock_legacy_undo_retains_adopted_allocation_watermark() {
    let mut session = fixture(true);
    let mut historical = crate::Project::clone(&persistence::diagnostic_snapshot(session.snapshot().unwrap()));
    historical.layout = None;
    historical.numeric_id_high_water = None;
    Arc::make_mut(&mut historical.board).ownership = crate::board::identity::Ownership::Legacy;
    let historical_documents = persistence::encode_documents(Arc::new(historical)).unwrap();
    let entry: Value = serde_json::from_slice(&historical_documents[persistence::ENTRY_POINT]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/6");
    for field in ["layout_root", "numeric_id_high_water", "schematic_instances"] {
        assert!(entry.get(field).is_none(), "{field}");
    }
    session = Session { libraries: session.libraries.clone(), ..Default::default() };
    session.open_snapshot(persistence::decode_documents(&historical_documents).unwrap());
    assert_eq!(documents(&session), historical_documents);
    let before = session.snapshot().unwrap();
    assert!(before.layout.is_none() && before.numeric_id_high_water.is_none() && !before.has_sheet_occurrences());
    lock(&mut session, true);
    let locked = documents(&session);
    assert_eq!(serde_json::from_slice::<Value>(&locked[persistence::ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/51");
    let watermark = session.project().unwrap().numeric_id_high_water.unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&locked).unwrap()).unwrap(), locked);
    edit(&mut session, "edit.undo", Value::Null);
    let restored = session.snapshot().unwrap();
    assert!(restored.layout.is_none() && !restored.has_sheet_occurrences());
    assert_eq!(restored.numeric_id_high_water, Some(watermark));
    assert!(restored.components().all(|part| !part.designator_locked()));
    crate::assert_same_content(&before, &restored);
    let restored_documents = documents(&session);
    assert_eq!(serde_json::from_slice::<Value>(&restored_documents[persistence::ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/7");
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&restored_documents).unwrap()).unwrap(), restored_documents);
    edit(&mut session, "edit.redo", Value::Null);
    assert_eq!(documents(&session), locked);
}

/// A descendant's lock selects the hierarchical envelope without flattening the tree.
/// Copy duplicates intent; explicit same-project Cut relocation retains original identity.
#[test]
fn designator_lock_hierarchy_copy_and_cut_relocation() {
    let mut source = fixture(false);
    let root = source.project().unwrap().sheet_target();
    edit(&mut source, crate::hierarchy::CREATE_CHILD, json!({"sheet":root.id(),"name":"Child","x_nm":0,"y_nm":0}));
    let child = source.project().unwrap().sheets().find(|sheet| sheet.name == "Child").unwrap().target;
    edit(&mut source, PLACE_SYMBOL, json!({"sheet":child.id(),"definition":"resistor","x_nm":0,"y_nm":0}));
    let symbol = source.project().unwrap().for_sheet(child).unwrap().symbols()[0].id();
    let block = format!("sheet-symbol-{}", source.project().unwrap().subsheets[0].id);
    let selection = |revision| json!({"sheet":root.id(),"items":{"symbols":[],"wires":[]},"objects":[block],"expected_revision":revision});
    let plain = source.execute(clipboard::COPY, &selection(source.revision())).unwrap()["clipboard"].as_str().unwrap().to_owned();
    assert_eq!(serde_json::from_str::<Value>(&plain).unwrap()["format"], "pcbcraft-clipboard/2");
    edit(&mut source, SET_SYMBOL_FIELDS, json!({"sheet":child.id(),"symbol":symbol,"reference":"R3","value":"R","designator_locked":true}));
    let before = documents(&source);
    let text = source.execute(clipboard::COPY, &selection(source.revision())).unwrap()["clipboard"].as_str().unwrap().to_owned();
    let envelope: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(envelope["format"], "pcbcraft-clipboard/3");
    assert!(envelope["root_sheet"].is_string());
    assert_eq!(documents(&source), before);
    let mut destination = Session::default();
    destination.apply("project.new", &json!({})).unwrap();
    destination.open_snapshot(persistence::diagnostic_snapshot(destination.snapshot().unwrap()));
    assert!(!destination.project().unwrap().has_sheet_occurrences());
    edit(&mut destination, clipboard::PASTE, json!({"text":text,"x_nm":50_800_000,"y_nm":0}));
    assert!(destination.project().unwrap().has_sheet_occurrences());
    assert_eq!(destination.project().unwrap().sheets().count(), 2);
    assert!(destination.project().unwrap().components().next().unwrap().designator_locked());
    let copied = documents(&destination);
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&copied).unwrap()).unwrap(), copied);
    let cut = source.execute(clipboard::CUT, &selection(source.revision())).unwrap()["clipboard"].as_str().unwrap().to_owned();
    assert!(source.project().unwrap().parent_sheet(child).is_none());
    edit(&mut source, clipboard::PASTE, json!({"text":cut,"sheet":root.id(),"x_nm":0,"y_nm":0,"reuse_sheets":true}));
    assert_eq!(source.project().unwrap().parent_sheet(child), Some(root));
    assert_eq!(source.project().unwrap().for_sheet(child).unwrap().symbols()[0].id(), symbol);
    assert!(source.project().unwrap().components().last().unwrap().designator_locked());
    edit(&mut source, "edit.undo", Value::Null);
    edit(&mut source, "edit.undo", Value::Null);
    let restored = documents(&source);
    let current: Value = serde_json::from_slice(&restored[persistence::ENTRY_POINT]).unwrap();
    let original: Value = serde_json::from_slice(&before[persistence::ENTRY_POINT]).unwrap();
    let current_watermark = current["numeric_id_high_water"].as_u64().unwrap();
    let original_watermark = original["numeric_id_high_water"].as_u64().unwrap();
    assert!(current_watermark >= original_watermark);
    // Paste reserves allocation identity across Undo; all other authored bytes must return.
    let text = std::str::from_utf8(&before[persistence::ENTRY_POINT]).unwrap();
    let old = format!("\"numeric_id_high_water\": {original_watermark}");
    assert_eq!(text.matches(&old).count(), 1);
    let mut expected = before.clone();
    expected.insert(persistence::ENTRY_POINT.into(), text.replace(&old, &format!("\"numeric_id_high_water\": {current_watermark}")).into_bytes());
    assert_eq!(restored, expected);
}
