use super::*;

/// Malformed records and nested Points pair each input with its serde diagnostic.
fn malformed_records(arc: &ArcTrack) -> Vec<(Value, String)> {
    let record = json!(arc);
    let mut cases = Vec::new();
    for field in ["id", "layer", "width_nm", "center", "start", "end", "direction"] {
        let mut omitted = record.clone();
        omitted.as_object_mut().unwrap().remove(field);
        cases.push((omitted, format!("missing field `{field}`")));
        let mut null = record.clone();
        null[field] = Value::Null;
        cases.push((null, "invalid type".into()));
    }
    for (value, reason) in [
        (json!("left"), "unknown variant `left`"),
        (json!(true), "invalid type"),
        (json!(0), "invalid type"),
        (json!({"clockwise":null}), "expected a string"),
        (json!({"counterclockwise":null}), "expected a string"),
        (json!({}), "expected a string"),
        (json!({"clockwise":null,"counterclockwise":null}), "expected a string"),
        (json!({"clockwise":{}}), "expected a string"),
        (json!({"left":null}), "expected a string"),
    ] {
        let mut invalid = record.clone();
        invalid["direction"] = value;
        cases.push((invalid, reason.into()));
    }
    let mut extra = record.clone();
    extra["radius_nm"] = json!(5000000);
    cases.push((extra, "unknown field `radius_nm`".into()));
    for point in ["center", "start", "end"] {
        for coordinate in ["x_nm", "y_nm"] {
            let mut omitted = record.clone();
            omitted[point].as_object_mut().unwrap().remove(coordinate);
            cases.push((omitted, format!("missing field `{coordinate}`")));
            let mut null = record.clone();
            null[point][coordinate] = Value::Null;
            cases.push((null, "invalid type".into()));
        }
        let mut extra = record.clone();
        extra[point]["z_nm"] = json!(0);
        cases.push((extra, "unknown field `z_nm`".into()));
    }
    cases
}

/// The final allowed count succeeds; one more refuses without replacing current authored state.
#[test]
fn arc_budget_and_uuid_order_are_checked_before_publication() {
    let (mut s, arc) = fixture();
    let arcs: Vec<_> = (1..=MAX_ARCS).rev().map(|id| ArcTrack { id: Uuid::from_u128(id as u128), ..arc.clone() }).collect();
    s.apply(SET_ARCS, &params(&s, &arcs)).unwrap();
    assert_eq!(view(&s).arc_tracks().len(), MAX_ARCS);
    assert!(view(&s).arc_tracks().windows(2).all(|pair| pair[0].id < pair[1].id));
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let mut too_many = arcs;
    too_many.push(arc.clone());
    assert_invalid_params(s.apply(SET_ARCS, &params(&s, &too_many)).unwrap_err(), "at most 4096");
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let reopened = decode_documents(&before).unwrap();
    assert_eq!(encode_documents(reopened).unwrap(), before);
    let path = s.project().unwrap().board_document_path().unwrap();
    let mut board: Value = serde_json::from_slice(&before[&path]).unwrap();
    board["arcs"].as_array_mut().unwrap().push(json!(arc));
    let mut excess = before.clone();
    excess.insert(path, serde_json::to_vec(&board).unwrap());
    assert!(decode_documents(&excess).unwrap_err().contains("at most 4096"));
}

/// Mixed straight/via/arc identities share one namespace, including saved codec validation.
#[test]
fn arc_ids_cannot_alias_existing_tracks_or_vias() {
    let (mut s, mut arc) = fixture();
    let layer = arc.layer;
    s.apply(
        crate::ADD_TRACKS,
        &json!({"layer":layer,"points":[{"x_nm":-50000000,"y_nm":-50000000},{"x_nm":-45000000,"y_nm":-50000000}],"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(crate::ADD_VIA, &json!({"at":{"x_nm":-50000000,"y_nm":-45000000},"expected_revision":s.revision()})).unwrap();
    let collisions = [view(&s).tracks()[0].id, view(&s).vias()[0].id];
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    for id in collisions {
        arc.id = id;
        assert_invalid_params(s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap_err(), "duplicate arc, track or via identity");
        assert_eq!(s.revision(), revision);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    arc.id = Uuid::from_u128(0xa11c);
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    let path = s.project().unwrap().board_document_path().unwrap();
    let board: Value = serde_json::from_slice(&documents[&path]).unwrap();
    for id in [arc.id, collisions[0], collisions[1]] {
        let mut invalid = board.clone();
        if id == arc.id {
            invalid["arcs"].as_array_mut().unwrap().push(json!(arc));
        } else {
            invalid["arcs"][0]["id"] = json!(id);
        }
        let mut malformed = documents.clone();
        malformed.insert(path.clone(), serde_json::to_vec(&invalid).unwrap());
        assert!(decode_documents(&malformed).unwrap_err().contains("duplicate arc, track or via identity"), "{id}");
    }
}

/// Native reopening accepts boundary records and refuses the shared invalid-record cases.
#[test]
fn native_arc_geometry_and_layers_are_validated_on_reopen() {
    let (mut s, arc) = fixture();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let initial_revision = s.revision();
    let mut accepted = Vec::new();
    for direction in [Direction::Clockwise, Direction::Counterclockwise] {
        for end in [Point { x_nm: 0, y_nm: 4_999_999 }, arc.end, Point { x_nm: 0, y_nm: 5_000_001 }, Point { x_nm: -5_000_000, y_nm: 0 }] {
            accepted.push(ArcTrack { end, direction, ..arc.clone() });
        }
    }
    let (min, max) = (super::super::super::track::MIN_WIDTH_NM, super::super::super::track::MAX_WIDTH_NM);
    for width_nm in [min, min + 1, max - 1, max] {
        accepted.push(ArcTrack { width_nm, ..arc.clone() });
    }
    for bound in [crate::schematic::MAX_CENTER_NM - 1, crate::schematic::MAX_CENTER_NM] {
        accepted.push(ArcTrack {
            center: Point { x_nm: -bound, y_nm: -bound },
            start: Point { x_nm: bound, y_nm: -bound },
            end: Point { x_nm: -bound, y_nm: bound },
            ..arc.clone()
        });
    }
    for end in [Point { x_nm: -1, y_nm: 1 }, Point { x_nm: -2, y_nm: 1 }] {
        accepted.push(ArcTrack { start: Point { x_nm: 1, y_nm: 1 }, end, ..arc.clone() });
    }
    for record in accepted {
        s.apply(SET_ARCS, &params(&s, &[record])).unwrap();
        let documents = encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    }
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let path = s.project().unwrap().board_document_path().unwrap();
    let board: Value = serde_json::from_slice(&documents[&path]).unwrap();
    for (record, reason) in invalid_records(&s, &arc) {
        let mut invalid = board.clone();
        invalid["arcs"] = json!([record]);
        let mut malformed = documents.clone();
        malformed.insert(path.clone(), serde_json::to_vec(&invalid).unwrap());
        let error = decode_documents(&malformed).unwrap_err();
        assert!(error.contains(reason), "{reason}: {error}");
        assert_eq!(s.revision(), revision);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), documents);
    }
    for _ in 0..revision - initial_revision {
        s.apply("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
}

/// Arc storage retains zones, origin and rule authority, restoring their format after arc removal.
#[test]
fn arc_format_retains_zones_rules_and_placement_origin() {
    let (mut s, arc) = fixture();
    let board = view(&s).target.board;
    let nets = s.project().unwrap().inspect_net_registry().unwrap();
    s.apply(
        super::super::super::zones::ADD_ZONE,
        &json!({"board":board,"layer":arc.layer,"target":{"kind":"derived","anchor":nets["nets"][0]["anchor"]},
            "points":[{"x_nm":0,"y_nm":0},{"x_nm":10000000,"y_nm":0},{"x_nm":10000000,"y_nm":10000000},{"x_nm":0,"y_nm":10000000}],
            "expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(crate::SET_RULES, &json!({"board":board,"rules":super::super::super::rules::Values::default(),"expected_revision":s.revision()}))
        .unwrap();
    s.apply(crate::SET_PLACEMENT_ORIGIN, &json!({"board":board,"origin":{"kind":"point","x_nm":100,"y_nm":200},"expected_revision":s.revision()}))
        .unwrap();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = s.project().unwrap().board_document_path().unwrap();
    let previous: Value = serde_json::from_slice(&before[&path]).unwrap();
    assert_eq!(previous["format"], "pcbcraft-board/41");
    assert_eq!(previous["zones"].as_array().unwrap().len(), 1);
    assert!(previous["placement_origin"].is_object());
    assert!(previous["rules"].is_object());
    s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let current: Value = serde_json::from_slice(&saved[&path]).unwrap();
    assert_eq!(current["format"], "pcbcraft-board/41");
    assert_eq!(current["zones"], previous["zones"]);
    assert_eq!(current["placement_origin"], previous["placement_origin"]);
    assert_eq!(current["rules"], previous["rules"]);
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    s.apply(SET_ARCS, &params(&s, &[])).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Commands and native documents retain the shared integer-token parser, not schema math semantics.
#[test]
fn arc_integer_tokens_are_required_in_commands_and_native_documents() {
    let (mut s, arc) = fixture();
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = s.project().unwrap().board_document_path().unwrap();
    let board = String::from_utf8(documents[&path].clone()).unwrap();
    let record = serde_json::to_string(&json!(arc)).unwrap();
    let board: Value = serde_json::from_str(&board).unwrap();
    let compact = serde_json::to_string(&board).unwrap();
    for (canonical, spellings) in [("\"width_nm\":250000", ["250000.0", "2.5e5", "250000.5"]), ("\"x_nm\":0", ["0.0", "0e0", "-0"])] {
        for spelling in spellings {
            let field = canonical.split(':').next().unwrap();
            let malformed = record.replacen(canonical, &format!("{field}:{spelling}"), 1);
            assert_ne!(malformed, record);
            let mut request = params(&s, &[]);
            request["arcs"] = serde_json::from_str(&format!("[{malformed}]")).unwrap();
            assert_invalid_params(s.execute(SET_ARCS, &request).unwrap_err(), "exact circular arc records");
            let mut invalid = documents.clone();
            let body = compact.replace(&record, &malformed);
            assert_ne!(body, compact);
            invalid.insert(path.clone(), body.into_bytes());
            assert!(decode_documents(&invalid).unwrap_err().contains("invalid type"), "{malformed}");
            assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), documents);
        }
    }
}

/// Command and raw-native shapes reject missing, null, extra and object-direction fields, including nested Points.
#[test]
fn arc_command_rejects_malformed_record_shapes() {
    let (mut s, arc) = fixture();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let snapshot = s.snapshot().unwrap();
    let state = s.inspect();
    let path = s.project().unwrap().board_document_path().unwrap();
    let board: Value = serde_json::from_slice(&documents[&path]).unwrap();
    let request = params(&s, std::slice::from_ref(&arc));
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    for (record, reason) in malformed_records(&arc) {
        let error = serde_json::from_value::<ArcTrack>(record.clone()).unwrap_err();
        assert!(error.to_string().contains(&reason), "{record}: {error}");
        let mut malformed = request.clone();
        malformed["arcs"] = json!([record]);
        assert_invalid_params(s.execute(SET_ARCS, &malformed).unwrap_err(), "exact circular arc records");
        let mut invalid = board.clone();
        invalid["arcs"] = malformed["arcs"].clone();
        let mut native = documents.clone();
        native.insert(path.clone(), serde_json::to_vec(&invalid).unwrap());
        let error = decode_documents(&native).unwrap_err();
        assert!(error.contains(&reason), "{record}: {error}");
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
        assert_eq!(s.inspect(), state);
        assert_eq!(s.revision(), revision);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), documents);
    }
    for spelling in ["1.0", "1e0", "-0", "9223372036854775807", "9223372036854775808"] {
        let mut malformed = request.clone();
        malformed["arcs"][0]["width_nm"] = serde_json::from_str(spelling).unwrap();
        let error = s.execute(SET_ARCS, &malformed).unwrap_err();
        assert!(matches!(&error, CommandError::InvalidParams(_)), "{spelling}: {error:?}");
        let message = error.to_string();
        assert!(message.contains("arc records") || message.contains("invalid circular arc geometry"), "{spelling}: {message}");
        assert_eq!(s.revision(), revision);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), documents);
    }
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), documents);
}
