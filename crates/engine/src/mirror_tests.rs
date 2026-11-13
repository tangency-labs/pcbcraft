//! Placed-symbol reflections compose with quarter turns without changing identities or wiring.
use super::*;
use crate::{
    clipboard::{Fragment, PasteOptions},
    persistence::{decode, decode_documents, decode_packed, encode, encode_documents, encode_packed},
    schematic::{GRID_NM, MirrorAxis, Point},
    selection::Selection,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Keep commands on the same validated GUI/MCP path while supplying the current revision.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Place the starter two-pin resistor at the origin and return its stable placement ID.
fn resistor(session: &mut Session) -> String {
    apply(session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    session.project().unwrap().symbols()[0].id()
}

/// Both axes compose with every quarter turn, publish the expected pins, and restore the exact
/// authored placement when applied twice. Identities, fields, undo and redo stay authoritative.
#[test]
fn both_axes_compose_with_every_turn_and_restore_on_repeat() {
    // The starter resistor's pins sit at -4 and +4 grid steps on the local X axis.
    let pin = |x: i64, y: i64| Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM };
    for turn in 0..4 {
        for axis in [MirrorAxis::Horizontal, MirrorAxis::Vertical] {
            let mut session = Session::default();
            session.apply("project.new", &json!({"name": "Mirror"})).unwrap();
            let symbol = resistor(&mut session);
            apply(
                &mut session,
                SET_SYMBOL_FIELDS,
                json!({"symbol":symbol,"reference":"R7","value":"4.7k","field_layout":{
                    "reference":{"offset":{"x_nm":-2_540_000,"y_nm":-6_350_000},"visible":true,"show_name":false,"rotation":90,"align":"start","height_nm":1_270_000},
                    "value":{"offset":{"x_nm":2_540_000,"y_nm":6_350_000},"visible":true,"show_name":true,"rotation":0,"align":"end","height_nm":1_270_000},
                }}),
            );
            if turn > 0 {
                apply(&mut session, MOVE_SYMBOL, json!({"symbol":symbol,"x_nm":0,"y_nm":0,"rotation":turn as u16 * 90}));
            }
            let before = session.snapshot().unwrap();
            let (expected_turns, expected_mirrored) = schematic::mirror_pose(turn, false, axis);

            apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":axis.as_str()}));
            let after = session.snapshot().unwrap();
            let placement = &after.symbols()[0];
            assert_eq!((placement.turns(), placement.is_mirrored()), (expected_turns, expected_mirrored), "{axis:?} at {turn} turns");
            assert_eq!(placement.id(), before.symbols()[0].id());
            assert_eq!(placement.component_id(), before.symbols()[0].component_id());
            assert_eq!(placement.terminals(), before.symbols()[0].terminals());
            // Literal expectations reflect the local pins about the symbol center, then apply the
            // stored post-mirror turn. Rows are indexed by the pre-mirror turn.
            let expected_pins = match axis {
                MirrorAxis::Horizontal => {
                    [[pin(4, 0), pin(-4, 0)], [pin(0, 4), pin(0, -4)], [pin(-4, 0), pin(4, 0)], [pin(0, -4), pin(0, 4)]][usize::from(turn)]
                }
                MirrorAxis::Vertical => {
                    [[pin(-4, 0), pin(4, 0)], [pin(0, -4), pin(0, 4)], [pin(4, 0), pin(-4, 0)], [pin(0, 4), pin(0, -4)]][usize::from(turn)]
                }
            };
            assert_eq!(placement.pins(), expected_pins, "{axis:?} at {turn} turns");
            assert_eq!(after.reference(placement), "R7");
            assert_eq!(after.component(placement).unwrap().display_value(), "4.7k");
            assert_eq!(placement.field_layout, before.symbols()[0].field_layout, "explicit text intent stays authored");
            let inspected = &session.inspect()["project"]["schematic"]["symbols"][0];
            assert_eq!(inspected["mirrored"], json!(true));
            assert_eq!(inspected["rotation"], json!(u16::from(expected_turns) * 90));

            // Rotation remains a counterclockwise turn of the mirrored drawing rather than a reset.
            apply(&mut session, ROTATE_SYMBOL, json!({"symbol":symbol}));
            let rotated_snapshot = session.snapshot().unwrap();
            let rotated = &rotated_snapshot.symbols()[0];
            assert_eq!((rotated.turns(), rotated.is_mirrored()), ((expected_turns + 1) % 4, true));
            session.apply("edit.undo", &Value::Null).unwrap();

            // The same axis twice restores the complete original snapshot, including fields.
            apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":axis.as_str()}));
            crate::assert_same_content(&before, &session.snapshot().unwrap());
            session.apply("edit.undo", &Value::Null).unwrap();
            crate::assert_same_content(&after, &session.snapshot().unwrap());
            session.apply("edit.undo", &Value::Null).unwrap();
            crate::assert_same_content(&before, &session.snapshot().unwrap());
            session.apply("edit.redo", &Value::Null).unwrap();
            crate::assert_same_content(&after, &session.snapshot().unwrap());
        }
    }
}

/// Attached wires follow the reflected terminal positions through the ordinary identity-preserving
/// edit path for both axes at every supported rotation.
#[test]
fn attached_wires_follow_mirrored_pins_without_changing_identities() {
    for turn in 0..4 {
        for axis in [MirrorAxis::Horizontal, MirrorAxis::Vertical] {
            let mut session = Session::default();
            session.apply("project.new", &json!({"name": "Mirror wires"})).unwrap();
            let symbol = resistor(&mut session);
            let placed = session.snapshot().unwrap();
            let symbol_view = &placed.symbols()[0];
            let routes = symbol_view
                .terminals()
                .iter()
                .zip(symbol_view.pins())
                .enumerate()
                .map(|(index, (terminal, anchor))| {
                    let to = Point { x_nm: anchor.x_nm + (index as i64 * 2 - 1) * 12 * GRID_NM, y_nm: anchor.y_nm };
                    (terminal.inspect(), to)
                })
                .collect::<Vec<_>>();
            for (from, to) in routes {
                apply(&mut session, DRAW_WIRE, json!({"from":from,"to":to,"bends":[]}));
            }
            if turn > 0 {
                apply(&mut session, MOVE_SYMBOL, json!({"symbol":symbol,"x_nm":0,"y_nm":0,"rotation":turn as u16 * 90}));
            }
            let before = session.snapshot().unwrap();
            apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":axis.as_str()}));
            let after = session.snapshot().unwrap();
            assert!(before.same_connectivity(&after), "{axis:?} at {turn} turns");
            assert_eq!(
                before.wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>(),
                after.wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>()
            );
            let pins = after.symbols()[0].pins();
            for wire in after.wires() {
                let path = after.wire_path(wire);
                for (endpoint, at) in wire.endpoints().into_iter().zip([path.first().copied(), path.last().copied()]) {
                    if let schematic::WireEndpoint::Terminal(pin) = endpoint {
                        assert_eq!(at, after.terminal_position(pin), "{axis:?} at {turn} turns");
                        let index = after.symbols()[0].unit_definition().pin_index(pin.number()).unwrap();
                        assert_eq!(at, Some(pins[index]), "wire reaches the mirrored pin anchor");
                    }
                }
            }
        }
    }
}

/// Stale revisions, unknown symbols and malformed axes cannot publish a partial pose.
#[test]
fn invalid_mirror_requests_leave_the_snapshot_and_history_untouched() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Mirror checks"})).unwrap();
    let symbol = resistor(&mut session);
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    for params in [
        json!({"symbol":symbol,"axis":"horizontal","expected_revision":revision-1}),
        json!({"symbol":symbol,"axis":"horizontal","expected_revision":revision+1}),
        json!({"symbol":symbol,"axis":"diagonal","expected_revision":revision}),
        json!({"symbol":symbol,"expected_revision":revision}),
        json!({"symbol":"symbol-999","axis":"vertical","expected_revision":revision}),
    ] {
        assert!(session.apply(MIRROR_SYMBOL, &params).is_err(), "{params}");
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()), "{params}");
    }
}

/// The clipboard retains the mirror flag and keeps the existing paste-rotation composition.
#[test]
fn clipboard_preserves_mirroring_through_paste_rotation() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Mirror clipboard"})).unwrap();
    let symbol = resistor(&mut session);
    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"vertical"}));
    apply(&mut session, ROTATE_SYMBOL, json!({"symbol":symbol}));
    let source = session.snapshot().unwrap();
    let turns = source.symbols()[0].turns();
    assert!(source.symbols()[0].is_mirrored());
    let text = Fragment::capture(&source, &Selection::one_symbol(symbol)).unwrap().to_text().unwrap();
    let fragment = Fragment::from_text(&text).unwrap();
    for quarter_turns in [0, 1, 3] {
        let (pasted, selected) = fragment
            .placed(
                &source,
                Point { x_nm: 30 * GRID_NM, y_nm: 0 },
                source.allocation_high_water() + 1,
                &PasteOptions { quarter_turns, ..PasteOptions::default() },
            )
            .unwrap();
        let id = selected.single_symbol().unwrap();
        let copy = pasted.symbols().iter().find(|copy| copy.id() == id).unwrap();
        assert!(copy.is_mirrored());
        assert_eq!(copy.turns(), (turns + quarter_turns) % 4);
        let numbers =
            |symbol: &schematic::SymbolPlacement| symbol.terminals().iter().map(|terminal| terminal.number().to_owned()).collect::<Vec<_>>();
        assert_eq!(numbers(copy), numbers(&source.symbols()[0]));
    }
}

/// Mirrored placements use sheet/6, keep only the true flag on disk, reopen from folders and
/// archives deterministically, and downgrade exactly when the last mirror is removed.
#[test]
fn mirrored_symbols_persist_as_sheet_six_and_downgrade_deterministically() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Mirror storage"})).unwrap();
    let symbol = resistor(&mut session);
    let before = session.snapshot().unwrap();
    let before_documents = encode_documents(before.clone()).unwrap();
    let sheet_path = before_documents.keys().find(|path| path.starts_with("sheets/")).unwrap().clone();

    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"horizontal"}));
    let mirrored = session.snapshot().unwrap();
    let documents = encode_documents(mirrored.clone()).unwrap();
    let sheet = std::str::from_utf8(&documents[&sheet_path]).unwrap();
    assert!(sheet.contains("\"format\": \"pcbcraft-sheet/6\""));
    assert!(sheet.contains("\"mirrored\":true"), "{sheet}");
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    let archive = encode_packed(mirrored.clone()).unwrap();
    assert_eq!(encode_documents(decode_packed(&archive).unwrap().project).unwrap(), documents);

    let mut older = documents.clone();
    older.insert(sheet_path.clone(), sheet.replace("pcbcraft-sheet/6", "pcbcraft-sheet/5").into_bytes());
    assert!(decode_documents(&older).unwrap_err().contains("sheet format 6"));

    // The diagnostic single-document codec refuses mirrors instead of silently dropping them.
    let legacy = include_bytes!("../../app/tests/fixtures/two-resistors.pc");
    let mut plain = Session::default();
    plain.open_snapshot(decode(legacy).unwrap());
    let mut single: Value = serde_json::from_slice(legacy).unwrap();
    single["sheet"]["symbols"][0]["mirrored"] = json!(true);
    assert!(decode(&serde_json::to_vec(&single).unwrap()).unwrap_err().contains("mirrored"));
    let plain_symbol = plain.project().unwrap().symbols()[0].id();
    apply(&mut plain, MIRROR_SYMBOL, json!({"symbol":plain_symbol,"axis":"horizontal"}));
    assert!(encode(plain.snapshot().unwrap()).unwrap_err().contains("mirrored"));

    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"horizontal"}));
    let restored = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(restored[&sheet_path], before_documents[&sheet_path]);
    assert_eq!(encode_documents(decode_documents(&restored).unwrap()).unwrap(), restored);
}

/// Place a resistor and an offset power terminal whose pin starts on the resistor's left pin.
fn offset_power_fixture() -> (Session, String, Arc<Project>) {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Mirror named"})).unwrap();
    let (_, mut docs) = crate::library::bundled_documents().find(|(id, _)| *id == "power").unwrap();
    let mut manifest: Value = serde_json::from_str(&docs["library.json"]).unwrap();
    manifest["id"] = json!("custom-rails");
    docs.insert("library.json".into(), manifest.to_string());
    let mut definition: Value = serde_json::from_str(&docs["symbols/gnd.json"]).unwrap();
    definition["units"][0]["pins"][0]["at_nm"] = json!([2 * GRID_NM, 0]);
    docs.retain(|path, _| path == "library.json");
    docs.insert("symbols/gnd.json".into(), definition.to_string());
    session.execute(crate::library::sources::LOAD, &json!({"source":"custom","documents":docs})).unwrap();

    apply(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":6*GRID_NM,"y_nm":0}));
    let definition = session.libraries().latest_symbol("custom-rails", "gnd").unwrap();
    session
        .apply(
            PLACE_SYMBOL,
            &json!({
                "library":"custom-rails","definition":"gnd","definition_revision":definition.revision(),
                "definition_digest":crate::library::sources::symbol_digest(&definition).unwrap(),
                "x_nm":0,"y_nm":0,"expected_revision":session.revision()
            }),
        )
        .unwrap();
    let symbol = session.project().unwrap().symbols()[1].id();
    let before = session.snapshot().unwrap();
    assert!(before.wires().is_empty());
    (session, symbol, before)
}

/// A double mirror removes only the connecting legs that retained named contacts created,
/// restoring the authored wire set even when a power terminal's pin is off center.
#[test]
fn double_mirror_removes_generated_named_contact_legs() {
    let (mut session, symbol, before) = offset_power_fixture();
    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"horizontal"}));
    assert_eq!(session.project().unwrap().wires().len(), 1, "the moved named pin gains a connecting leg");

    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"horizontal"}));
    crate::assert_same_content(&before, &session.snapshot().unwrap());
}

/// Generated-leg provenance survives a folder reopen, so a later mirror removes only that leg
/// and leaves authored routes untouched.
#[test]
fn reopened_project_removes_generated_named_contact_leg_after_mirror() {
    let (mut session, symbol, _) = offset_power_fixture();
    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"horizontal"}));
    assert_eq!(session.project().unwrap().wires().len(), 1);

    apply(&mut session, DRAW_WIRE, json!({"from":{"x_nm":20*GRID_NM,"y_nm":20*GRID_NM},"to":{"x_nm":24*GRID_NM,"y_nm":20*GRID_NM},"bends":[]}));
    let authored = session.project().unwrap().wires()[1].id();
    assert_eq!(session.project().unwrap().wires().len(), 2);

    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let sheet_path = documents.keys().find(|path| path.starts_with("sheets/")).unwrap().clone();
    let sheet = std::str::from_utf8(&documents[&sheet_path]).unwrap();
    assert!(sheet.contains("\"format\": \"pcbcraft-sheet/7\""), "{sheet}");
    assert!(sheet.contains("\"generated_legs\": ["), "{sheet}");
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    assert!(encode(session.snapshot().unwrap()).unwrap_err().contains("generated named-contact legs"));

    session.open_snapshot(decode_documents(&documents).unwrap());
    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"horizontal"}));
    let after = session.project().unwrap();
    assert_eq!(after.wires().len(), 1, "only the authored route remains");
    assert_eq!(after.wires()[0].id(), authored);
    assert!(!after.symbols()[1].is_mirrored());
    let restored = encode_documents(session.snapshot().unwrap()).unwrap();
    let restored_sheet = std::str::from_utf8(&restored[&sheet_path]).unwrap();
    assert!(!restored_sheet.contains("generated_legs"), "{restored_sheet}");
    assert!(!restored_sheet.contains("pcbcraft-sheet/7"), "{restored_sheet}");
}

/// Editing a generated leg's route makes it authored again: its stale provenance is dropped
/// on save and a later mirror preserves the edited route.
#[test]
fn edited_generated_leg_becomes_authored_across_reopen() {
    let (mut session, symbol, _) = offset_power_fixture();
    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"horizontal"}));
    let leg = session.project().unwrap().wires()[0].id();

    apply(&mut session, MOVE_WIRE_SEGMENT, json!({"wire":leg,"at":{"x_nm":0,"y_nm":0},"to":{"x_nm":0,"y_nm":2*GRID_NM}}));
    let edited = session.project().unwrap().wires()[0].clone();

    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let sheet_path = documents.keys().find(|path| path.starts_with("sheets/")).unwrap().clone();
    let sheet = std::str::from_utf8(&documents[&sheet_path]).unwrap();
    assert!(!sheet.contains("generated_legs"), "{sheet}");
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);

    session.open_snapshot(decode_documents(&documents).unwrap());
    apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":"horizontal"}));
    let after = session.project().unwrap();
    assert_eq!(after.wires(), &[edited], "the edited route is authored and must survive");
    assert!(!after.symbols()[1].is_mirrored());
}
