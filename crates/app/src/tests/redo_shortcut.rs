//! Ctrl+Shift+Z is a second Redo chord, beside Ctrl+Y, in the project and library editors.

use pcbcraft_engine::authoring::Kind;

use super::*;

/// Ctrl+Shift+Z redoes the project edit Ctrl+Z undid instead of undoing a further step, and with
/// nothing to redo it leaves the project as it is.
#[test]
fn ctrl_shift_z_redoes_project_edits() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    let one = h.state().session.snapshot().unwrap();
    let revision = h.state().session().revision();
    h.state_mut()
        .session
        .apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 5_080_000, "y_nm": 0, "expected_revision": revision}))
        .unwrap();
    h.run();
    let edited = h.state().session.snapshot().unwrap();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_same_content(&one, &h.state().session.snapshot().unwrap());
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run();
    assert_same_content(&edited, &h.state().session.snapshot().unwrap());
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run();
    assert_same_content(&edited, &h.state().session.snapshot().unwrap());
}

/// The symbol and footprint editors take Ctrl+Shift+Z as Redo for their own document history.
/// Both Redo chords stay silent once the redo history is empty, as in the project editors.
#[test]
fn ctrl_shift_z_redoes_library_edits() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        let mut h = harness();
        h.state_mut().new_library_item(kind);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        let ctx = h.ctx.clone();
        for (revision, offset) in [(0, json!({"x_nm": -1_270_000, "y_nm": 0})), (1, json!({"x_nm": 0, "y_nm": 1_270_000}))] {
            let transform = json!({"pivot": {"x_nm": 0, "y_nm": 0}, "offset": offset, "quarter_turns": 0, "mirror_x": false, "mirror_y": false});
            let action = json!({"operation": "transform", "objects": [1], "transform": transform});
            h.state_mut()
                .run_window_command(&ctx, "app.library_edit", &json!({"document": id, "expected_revision": revision, "action": action}))
                .unwrap();
        }
        h.run();
        let objects = |h: &Harness<'_, PcbApp>| h.state().library_editors.inspect()["documents"][0]["state"]["item"]["objects"].clone();
        let edited = objects(&h);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_ne!(objects(&h), edited, "{kind:?}: Ctrl+Z undoes the second move");
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        h.run();
        assert_eq!(objects(&h), edited, "{kind:?}: Ctrl+Shift+Z redoes the second move");
        let notice = h.state().library_editors.inspect()["notice"].clone();
        for (modifiers, key) in [(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z), (Modifiers::COMMAND, Key::Y)] {
            h.key_press_modifiers(modifiers, key);
            h.run();
            assert_eq!(objects(&h), edited, "{kind:?}: {key:?} with nothing to redo changes nothing");
            assert_eq!(h.state().library_editors.inspect()["notice"], notice, "{kind:?}: {key:?} with nothing to redo stays silent");
        }
    }
}
