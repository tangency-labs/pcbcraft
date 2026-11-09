//! Coordinate drafts are edited through real controls and committed through the shared command.

use super::*;
use crate::tests::{click, harness, resistor_at_origin};
use egui::{Key, Modifiers};
use egui_kittest::{Harness, kittest::Queryable};
use pcbcraft_engine::{CONNECT_PINS, MOVE_SYMBOL, PLACE_SYMBOL, persistence::encode_documents, selection::Selection};

/// A wired symbol is selected in the actual schematic inspector.
fn fixture() -> Harness<'static, PcbApp> {
    let mut h = harness();
    resistor_at_origin(&mut h);
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":25_400_000,"y_nm":0,"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(
        CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":revision}),
    );
    h.state_mut().properties_open = true;
    h.state_mut().select(Selection::one_symbol("symbol-2".into()));
    h.set_size(egui::vec2(1200., 1000.));
    h.run();
    h
}

/// Replace a field through its accessible input rather than mutating the property draft.
fn type_field(h: &mut Harness<'_, PcbApp>, label: &str, value: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run();
    h.get_by_label(label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.get_by_label(label).type_text(value);
    h.run();
}

/// A mixed-unit coordinate edit and other fields form one revision and undo step. The wire
/// remains attached, and compact/expanded controls retain the same physical draft.
#[test]
fn coordinates_and_fields_apply_once_with_wires_and_round_trip() {
    let mut h = fixture();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    type_field(&mut h, "X", "100 mil");
    type_field(&mut h, "Y", "-1.270001 mm");
    type_field(&mut h, "Value", "4.7k");
    click(&mut h, "More properties ↗");
    assert_eq!(h.get_by_label("Y").value().as_deref(), Some("-1.270001 mm"));
    click(&mut h, "Rotation");
    click(&mut h, "90°");
    click(&mut h, "OK");
    assert_eq!(h.state().session.revision(), revision + 1);
    let changed = h.state().session.snapshot().unwrap();
    assert_eq!(changed.symbols()[0].position, Point { x_nm: 2_540_000, y_nm: -1_270_001 });
    assert_eq!(changed.symbols()[0].turns(), 1);
    assert_eq!(changed.component(&changed.symbols()[0]).unwrap().value(), "4.7k");
    assert!(before.same_connectivity(&changed));
    let documents = encode_documents(changed.clone()).unwrap();
    let reopened = pcbcraft_engine::persistence::decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened).unwrap(), documents);
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Redo");
    assert!(Arc::ptr_eq(&changed, &h.state().session.snapshot().unwrap()));
}

/// Invalid or out-of-bounds coordinates cannot partially apply other fields. Reset and modal
/// Cancel discard both valid and invalid draft coordinates without changing saved bytes/history.
#[test]
fn invalid_coordinates_reset_and_cancel_leave_the_project_alone() {
    let mut h = fixture();
    let before = h.state().session.inspect();
    let documents = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    for input in ["2 cm", "", "1000 mm"] {
        type_field(&mut h, "Value", "not applied");
        type_field(&mut h, "X", input);
        click(&mut h, "Apply changes");
        assert_eq!(h.state().session.inspect(), before, "{input}");
        assert_eq!(h.get_by_label("X").value().as_deref(), Some(input));
        click(&mut h, "Reset changes");
        assert_eq!(h.get_by_label("X").value().as_deref(), Some("0 mil"));
        assert_eq!(h.get_by_label("Value").value().as_deref(), Some("R"));
    }
    type_field(&mut h, "X", "100 mil");
    click(&mut h, "More properties ↗");
    type_field(&mut h, "Y", "bad");
    click(&mut h, "OK");
    assert!(matches!(h.state().dialog, Dialog::EditFields(_)));
    assert_eq!(h.state().session.inspect(), before);
    click(&mut h, "Cancel");
    assert_eq!(h.get_by_label("X").value().as_deref(), Some("0 mil"));
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), documents);
}

/// Unit changes preserve exact physical drafts, geometry and history, even for a one-nanometre
/// placement. Applying only another field must not round those original coordinates away.
#[test]
fn unit_switches_preserve_exact_coordinates_and_unapplied_drafts() {
    let mut h = fixture();
    let revision = h.state().session.revision();
    h.state_mut().run(MOVE_SYMBOL, &json!({"symbol":"symbol-2","x_nm":1,"y_nm":-1,"expected_revision":revision}));
    h.run();
    let before = h.state().session.inspect();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    for _ in 0..4 {
        let label = format!("Units: {}", h.state().preferences.units.symbol());
        click(&mut h, &label);
    }
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    assert!(h.query_by_label("Apply changes").is_none());
    type_field(&mut h, "Value", "2k");
    click(&mut h, "Apply changes");
    assert_eq!(h.state().session.project().unwrap().symbols()[0].position, Point { x_nm: 1, y_nm: -1 });
    let revision = h.state().session.revision();
    type_field(&mut h, "X", "100");
    click(&mut h, "Units: mil");
    assert_eq!(h.get_by_label("X").value().as_deref(), Some("2.54 mm"));
    assert_eq!(h.state().session.revision(), revision);
    type_field(&mut h, "Y", "-0.0254");
    click(&mut h, "Apply changes");
    assert_eq!(h.state().session.project().unwrap().symbols()[0].position, Point { x_nm: 2_540_000, y_nm: -25_400 });
    assert_eq!(h.state().session.revision(), revision + 1);
}

/// A changed selection or external revision expires the draft rather than applying its text
/// to the new object. Captured actions from another pane are refused even at the same revision.
#[test]
fn property_coordinates_reject_changed_selection_revision_and_pane() {
    let mut h = fixture();
    type_field(&mut h, "X", "100 mil");
    let stale = h.state().read_property_edit().unwrap();
    h.state_mut().select(Selection::one_symbol("symbol-3".into()));
    let before = h.state().session.inspect();
    assert!(h.state_mut().apply_property_edit(&stale).is_err());
    h.run();
    assert_eq!(h.get_by_label("X").value().as_deref(), Some("1000 mil"));
    type_field(&mut h, "X", "30 mm");
    h.state_mut().run("project.rename", &json!({"name":"External edit"}));
    h.run();
    assert_eq!(h.get_by_label("X").value().as_deref(), Some("1000 mil"));
    let mut captured = h.state().read_property_edit().unwrap();
    captured.x = "40 mm".into();
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let pane = h.state_mut().views.other_mut().id();
    let ctx = h.ctx.clone();
    h.state_mut().activate_editor(&ctx, pane);
    assert!(h.state_mut().apply_property_edit(&captured).is_err());
    assert_eq!(h.state().session.inspect()["project"]["schematic"]["symbols"], before["project"]["schematic"]["symbols"]);
}

/// A draft captured in a detached view cannot edit the main host's selection, even at the same
/// revision. Rebinding its original view changes only its selected symbol, with shared undo.
#[test]
fn detached_coordinate_drafts_keep_their_view_and_selection() {
    let mut h = fixture();
    let main = h.state().views.active.id();
    let host = h.state_mut().views.open_window(main).unwrap();
    let child = h.state().views.hosts[&host].focused;
    h.state_mut().views.view_mut(child).unwrap().schematic.selection = Selection::one_symbol("symbol-3".into());
    let before = h.state().session.snapshot().unwrap();
    h.state_mut().views.bind(child);
    let mut edit = h.state().read_property_edit().unwrap();
    edit.x = "30 mm".into();
    h.state_mut().views.bind(main);
    assert!(h.state_mut().apply_property_edit(&edit).is_err());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.state_mut().views.bind(child);
    h.state_mut().apply_property_edit(&edit).unwrap();
    h.state_mut().views.bind(main);
    h.run();
    let project = h.state().session.project().unwrap();
    assert_eq!(project.symbols()[0].position, before.symbols()[0].position);
    assert_eq!(project.symbols()[1].position.x_nm, 30_000_000);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// The normal filter card leaves the taller inspector scrollable at desktop and compact
/// heights. Coordinates, Apply and Reset remain reachable without overlapping the filters.
#[test]
fn coordinates_remain_reachable_with_selection_filters() {
    for size in [egui::vec2(1280., 800.), egui::vec2(420., 700.)] {
        let mut h = fixture();
        h.state_mut().views.active.filters_open = true;
        h.set_size(size);
        h.run();
        h.get_by_label("X").scroll_to_me();
        h.run();
        type_field(&mut h, "X", "100 mil");
        h.get_by_label("Apply changes").scroll_to_me();
        h.run();
        let apply = h.get_by_label("Apply changes").rect();
        let filters = h.get_by_label("Collapse Selection filters").rect();
        assert!(apply.bottom() < filters.top(), "{size:?}: {apply:?}, {filters:?}");
        click(&mut h, "Apply changes");
        assert_eq!(h.state().session.project().unwrap().symbols()[0].position.x_nm, 2_540_000);
        h.get_by_label("X").scroll_to_me();
        h.run();
        type_field(&mut h, "X", "200 mil");
        h.get_by_label("Reset changes").scroll_to_me();
        h.run();
        click(&mut h, "Reset changes");
        assert_eq!(h.state().session.project().unwrap().symbols()[0].position.x_nm, 2_540_000);
    }
}
