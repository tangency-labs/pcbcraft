use super::*;
use pcbcraft_engine::{
    net_labels::{self as engine, Style},
    persistence::encode_documents,
    schematic::Point,
};

/// Exercise the real editor with room for labels; panel state changes only test presentation.
fn editor(width: f32) -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(width, 800.));
    new_project(&mut h);
    h.state_mut().parts.open = false;
    h.state_mut().browser.open = false;
    h.run();
    h
}
/// Replace the focused name through actual text input.
fn name(h: &mut Harness<'_, PcbApp>, name: &str) {
    let label = if h.state().views.active.schematic.text_edit.is_some() { "Edit Net name" } else { "Net name" };
    h.get_by_label(label).click();
    h.run();
    clear_name(h);
    h.get_by_label(label).type_text(name);
    h.run();
}
/// Pick a model-derived label anchor so canvas and toolbar dimensions may evolve independently.
fn at(h: &Harness<'_, PcbApp>, p: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(p, h.get_by_label("Schematic canvas").rect())
}

/// Selecting ground/power ink exposes the same captured properties in the inspector and form,
/// including an editable drawing style and physical net-name appearance.
#[test]
fn power_properties_are_editable_from_the_selection_inspector() {
    for style in ["ground", "power", "power_bar"] {
        let mut h = editor(1200.);
        let revision = h.state().session.revision();
        h.state_mut().run(engine::ADD, &json!({"name":"GND","style":style,"x_nm":0,"y_nm":0,"expected_revision":revision}));
        h.state_mut().properties_open = true;
        h.run();
        let point = h
            .output()
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(text) if text.galley.job.text == "GND" => Some(text.visual_bounding_rect().center()),
                _ => None,
            })
            .unwrap();
        click_canvas_at(&mut h, point);
        assert!(h.state().views.active.schematic.selection.single_net_label().is_some());
        name(&mut h, "GNDA");
        click(&mut h, "Symbol");
        click(&mut h, "Power bar");
        click(&mut h, "More properties ↗");
        assert_eq!(h.get_by_label("Net name").value().as_deref(), Some("GNDA"));
        click(&mut h, "Text appearance");
        click(&mut h, "Automatic position");
        for (label, value) in [("Text X", "100 mil"), ("Text Y", "150 mil"), ("Text height", "1.5 mm")] {
            h.get_by_label(label).scroll_to_me();
            h.run();
            h.get_by_label(label).click();
            h.run();
            h.key_press_modifiers(Modifiers::COMMAND, Key::A);
            h.get_by_label(label).type_text(value);
            h.run();
        }
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        click(&mut h, "Apply");
        assert_eq!(h.state().session.revision(), revision + 1);
        let project = h.state().session.snapshot().unwrap();
        let label = &project.net_labels()[0];
        assert_eq!(label.name, "GNDA");
        assert_eq!(label.style, Style::PowerBar);
        assert_eq!(label.at, Point { x_nm: 0, y_nm: 0 });
        assert_eq!(label.text.as_ref().unwrap().height_nm, 1_500_000);
        assert_eq!(label.text.as_ref().unwrap().offset, Some(Point { x_nm: 2_540_000, y_nm: 3_810_000 }));
        let docs = encode_documents(project.clone()).unwrap();
        let reopened = pcbcraft_engine::persistence::decode_documents(&docs).unwrap();
        assert_eq!(encode_documents(reopened).unwrap(), docs);
        click(&mut h, "Undo");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click(&mut h, "Redo");
        assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
        name(&mut h, "discarded");
        click(&mut h, "More properties ↗");
        click(&mut h, "Cancel");
        assert_eq!(h.get_by_label("Net name").value().as_deref(), Some("GNDA"));
        assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
    }
}

/// Opening a power form from the second pane retains that pane as its command source even
/// when the shared project dialog is painted by the main host.
#[test]
fn power_properties_keep_their_originating_split_view() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.state_mut().run(engine::ADD, &json!({"name":"VCC","style":"power_bar","x_nm":0,"y_nm":0,"expected_revision":revision}));
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let canvas = h.get_by_label("Schematic canvas 2").rect();
    let point = h
        .output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == "VCC" && canvas.contains(t.pos) => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap();
    click_canvas_at(&mut h, point);
    h.hover_at(point + egui::vec2(200., 100.));
    h.run();
    h.key_press(Key::E);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::NetLabel(_)));
    name(&mut h, "+3V3");
    click(&mut h, "Apply");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].name, "+3V3");
}

/// Power placement accepts a preset, previews rotation without editing, and commits once.
/// Hover hotkeys edit/move/delete the label instead of any component underneath its ink.
#[test]
fn power_placement_edit_move_delete_and_undo() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.key_press(Key::P);
    h.run();
    assert!(h.state().parts.open);
    assert_eq!(h.get_by_label("Net name").value().as_deref(), Some("GND"));
    click(&mut h, "Place");
    assert_eq!(h.state().session.revision(), revision);
    h.key_press(Key::R);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool,Some(Tool::NetLabel {ref label,..}) if label.rotation==90));
    let point = canvas_center(&h);
    click_canvas_at(&mut h, point);
    assert_eq!(h.state().session.revision(), revision + 1);
    let label = h.state().session.project().unwrap().net_labels()[0].clone();
    assert_eq!(label.style, Style::Ground);
    let hover = at(&h, label.at);
    h.hover_at(hover);
    h.run();
    h.key_press(Key::E);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::NetLabel(_)));
    name(&mut h, "GNDA");
    click(&mut h, "Apply");
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].name, "GNDA");
    h.hover_at(hover);
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabel { .. })));
    click_canvas_at(&mut h, hover + egui::vec2(80., 40.));
    let moved = h.state().session.project().unwrap().net_labels()[0].clone();
    assert_ne!(moved.at, label.at);
    let hover = at(&h, moved.at);
    h.hover_at(hover);
    h.run();
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().session.project().unwrap().net_labels().is_empty());
    click(&mut h, "Undo");
    assert_eq!(h.state().session.project().unwrap().net_labels()[0], moved);
}

/// Ground starts immediately from both ribbon layouts, with no catalog/form or history edit.
/// Rotation and cancellation are previews; placing and undoing use the shared named-net command.
#[test]
fn ground_ribbon_places_directly_and_cancels_without_edits() {
    for width in [1400., 420.] {
        let mut h = editor(width);
        let original = h.state().session.inspect();
        let revision = h.state().session.revision();
        h.key_press(Key::P);
        h.run();
        click(&mut h, "Dock Parts Catalog");
        assert!(h.state().parts.open);
        for cancel in [true, false] {
            if width < 1000. {
                click(&mut h, "Place menu");
                click(&mut h, "Ground");
            } else {
                click(&mut h, "Place ground");
            }
            assert!(!h.state().parts.open);
            assert!(matches!(h.state().dialog, Dialog::None));
            assert!(
                matches!(&h.state().views.active.schematic.tool,Some(Tool::NetLabel {label,..}) if label.style==Style::Ground && label.name=="GND" && label.rotation==0)
            );
            assert_eq!(h.state().session.inspect(), original);
            h.key_press(Key::R);
            h.run();
            if cancel {
                h.key_press(Key::Escape);
                h.run();
                assert!(h.state().views.active.schematic.tool.is_none());
                assert_eq!(h.state().session.inspect(), original);
            } else {
                let point = canvas_center(&h);
                click_canvas_at(&mut h, point);
                assert_eq!(h.state().session.revision(), revision + 1);
                let label = &h.state().session.project().unwrap().net_labels()[0];
                assert_eq!((&*label.name, label.style, label.rotation), ("GND", Style::Ground, 90));
                click(&mut h, "Undo");
                assert!(h.state().session.project().unwrap().net_labels().is_empty());
                click(&mut h, "Redo");
                assert_eq!(h.state().session.project().unwrap().net_labels()[0].style, Style::Ground);
            }
        }
    }
}

/// Local/global shortcuts and compact caption-menu controls reach the same form. Cancelled
/// properties and cancelled placement leave both encoded bytes and undo history unchanged.
#[test]
fn label_tools_and_cancel_at_both_widths() {
    for width in [1400., 600.] {
        let mut h = editor(width);
        let original = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        h.key_press(Key::L);
        h.run();
        name(&mut h, "RESET");
        click(&mut h, "Place");
        assert!(matches!(h.state().views.active.schematic.tool,Some(Tool::NetLabel {ref label,..}) if label.style==Style::Local));
        h.key_press(Key::Escape);
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::L);
        h.run();
        click(&mut h, "Place");
        assert!(matches!(h.state().views.active.schematic.tool,Some(Tool::NetLabel {ref label,..}) if label.style==Style::Global));
        h.key_press(Key::Escape);
        h.run();
        h.get_by_label("Place menu").scroll_to_me();
        h.run();
        click(&mut h, "Place menu");
        click(&mut h, "Ground");
        assert!(!h.state().parts.open);
        assert!(
            matches!(h.state().views.active.schematic.tool,Some(Tool::NetLabel {ref label,..}) if label.style==Style::Ground && label.name=="GND")
        );
        h.key_press(Key::Escape);
        h.run();
        assert_eq!(h.state().session.revision(), revision);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), original);
    }
}

/// The live adapter discovers and executes named-anchor commands; GUI history and both
/// inspectors use the same names. A form captured before a concurrent edit cannot overwrite it.
#[test]
fn live_commands_inspection_names_and_stale_form() {
    use crate::live_control::LiveEditor;
    use pcbcraft_mcp::live::call_editor;
    let mut h = editor(1400.);
    let ctx = h.ctx.clone();
    let revision = h.state().session.revision();
    let result = call_editor(
        &mut LiveEditor { app: h.state_mut(), ctx: &ctx },
        &json!({"method":"engine.execute","params":{"command":engine::ADD,"params":{"name":"+3V3","style":"power_bar","x_nm":0,"y_nm":0,"expected_revision":revision}}}),
    );
    assert!(result.is_ok(), "{result:?}");
    h.run();
    click(&mut h, "Toggle Net Inspector");
    h.get_by_label("+3V3");
    let label = h.state().session.project().unwrap().net_labels()[0].clone();
    let p = at(&h, label.at);
    h.hover_at(p);
    h.run();
    h.key_press(Key::E);
    h.run();
    h.state_mut().session.apply("project.rename", &json!({"name":"Concurrent"})).unwrap();
    name(&mut h, "+5V");
    click(&mut h, "Apply");
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].name, "+3V3");
    assert!(h.state().message.as_deref().unwrap_or_default().contains("changed"));
}

/// Text clicks select a label without opening its form. Selection supports properties and
/// toolbar actions away from the pointer, while clicking empty canvas releases it normally.
#[test]
fn label_selection_routes_toolbar_and_keyboard_without_mutating_data() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.state_mut().run(engine::ADD, &json!({"name":"RESET","style":"local","x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    let point = at(&h, Point { x_nm: 0, y_nm: 0 });
    click_canvas_at(&mut h, point + egui::vec2(25., -10.));
    assert!(h.state().views.active.schematic.selection.single_net_label().is_some());
    assert!(matches!(h.state().dialog, Dialog::None));
    let revision = h.state().session.revision();
    h.hover_at(point + egui::vec2(200., 100.));
    h.run();
    h.key_press(Key::E);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::NetLabel(_)));
    click(&mut h, "Cancel");
    assert_eq!(h.state().session.revision(), revision);
    click(&mut h, "Rotate selected object");
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].rotation, 90);
    h.get_by_label("Modify menu").scroll_to_me();
    h.run();
    click(&mut h, "Modify menu");
    click(&mut h, "Delete");
    assert!(h.state().session.project().unwrap().net_labels().is_empty());
    click(&mut h, "Undo");
    let point = at(&h, Point { x_nm: 0, y_nm: 0 });
    click_canvas_at(&mut h, point + egui::vec2(25., -10.));
    click_canvas_at(&mut h, point + egui::vec2(200., 100.));
    assert!(h.state().views.active.schematic.selection.single_net_label().is_none());
}

/// Marker placement uses magnetic contact coordinates even when the focused view's snap
/// spacing would otherwise pull an existing pin off its electrical anchor.
#[test]
fn power_placement_reaches_pins_between_grid_points() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":1_270_000,"y_nm":0,"expected_revision":revision}));
    h.state_mut().views.active.camera.set_snap_nm(2_540_000);
    h.run();
    let pin = h.state().session.project().unwrap().symbols()[0].terminals()[0];
    let point = h.state().session.project().unwrap().terminal_position(pin).unwrap();
    let screen = at(&h, point);
    assert_ne!(h.state().views.active.camera.snap(screen, h.get_by_label("Schematic canvas").rect()), point);
    h.key_press(Key::P);
    h.run();
    click(&mut h, "Place");
    click_canvas_at(&mut h, screen);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.net_labels()[0].at, point);
    let index = project.connectivity().net_index_for(pin).unwrap();
    assert_eq!(project.connectivity().net_name(index), "GND");
}

/// Direct dragging starts from the original press, keeps the grabbed ink under the pointer,
/// previews attached routes without edits, and commits the marker and wire as one undo step.
#[test]
fn dragging_power_ink_and_anchor_moves_once_and_preserves_attached_wire() {
    for rotation in [0, 90, 180, 270] {
        for grab in [egui::vec2(0., 0.), egui::vec2(5., 17.)] {
            let mut h = editor(1400.);
            let rev = h.state().session.revision();
            h.state_mut().run(engine::ADD, &json!({"name":"GND","style":"ground","x_nm":0,"y_nm":0,"rotation":rotation,"expected_revision":rev}));
            let label = h.state().session.project().unwrap().net_labels()[0].clone();
            let rev = h.state().session.revision();
            h.state_mut().run(
                pcbcraft_engine::DRAW_WIRE,
                &json!({"from":{"x_nm":-25_400_000,"y_nm":0},"to":{"net_label":label.object_id()},"bends":[],"expected_revision":rev}),
            );
            h.run();
            h.state_mut().parts.set_repeat(true);
            let original = h.state().session.inspect();
            let rev = h.state().session.revision();
            let delta = egui::vec2(81.28, 60.96);
            let grab = match rotation {
                90 => egui::vec2(grab.y, -grab.x),
                180 => -grab,
                270 => egui::vec2(-grab.y, grab.x),
                _ => grab,
            };
            let from = at(&h, label.at) + grab * (h.state().views.active.camera.unit_step() / 10.);
            let to = from + delta;
            let expected = h.state().views.active.camera.snap(at(&h, label.at) + delta, h.get_by_label("Schematic canvas").rect());
            h.drag_at(from);
            h.run();
            h.hover_at(from + egui::vec2(8., 0.));
            h.run();
            assert!(
                matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabel { drag: Some(_), .. })),
                "rotation {rotation}, grab {grab:?}"
            );
            h.hover_at(to);
            h.run();
            assert_eq!(h.state().session.inspect(), original);
            h.drop_at(to);
            h.run();
            assert_eq!(h.state().session.revision(), rev + 1);
            let project = h.state().session.project().unwrap();
            assert_eq!(project.net_labels()[0].at, expected);
            assert_eq!(project.net_labels()[0].rotation, rotation);
            assert_eq!(project.wire_path(&project.wires()[0]).last(), Some(&expected));
            assert!(h.state().views.active.schematic.tool.is_none());
            click(&mut h, "Undo");
            assert_eq!(h.state().session.project().unwrap().net_labels()[0], label);
        }
    }
}

/// Escape, lost focus, a release outside the canvas and a concurrent edit discard a held drag.
#[test]
fn interrupted_power_drags_never_commit_or_repeat() {
    for interruption in ["escape", "focus", "outside", "stale"] {
        let mut h = editor(1400.);
        let rev = h.state().session.revision();
        h.state_mut().run(engine::ADD, &json!({"name":"GND","style":"ground","x_nm":0,"y_nm":0,"expected_revision":rev}));
        h.state_mut().parts.set_repeat(true);
        h.run();
        let start = at(&h, Point { x_nm: 0, y_nm: 0 }) + egui::vec2(0., 17.) * (h.state().views.active.camera.unit_step() / 10.);
        let to = start + egui::vec2(100., 60.);
        h.drag_at(start);
        h.run();
        h.hover_at(to);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabel { drag: Some(_), .. })));
        let mut release = to;
        match interruption {
            "escape" => h.key_press(Key::Escape),
            "focus" => h.event(egui::Event::WindowFocused(false)),
            "outside" => release = egui::pos2(3., 3.),
            _ => h.state_mut().run("project.rename", &json!({"name":"Concurrent"})),
        }
        h.run();
        let before = h.state().session.inspect();
        h.drop_at(release);
        h.run();
        assert_eq!(h.state().session.inspect(), before, "{interruption}");
        assert_eq!(h.state().session.project().unwrap().net_labels()[0].at, Point { x_nm: 0, y_nm: 0 });
    }
}

/// Contact indicators follow exact local geometry: a named anchor on a free wire end removes
/// both open markers, moving it away restores them, and matching remote names do not hide pins.
#[test]
fn power_contacts_hide_open_markers_and_restore_them_when_detached() {
    let mut h = editor(1400.);
    let origin = Point { x_nm: 0, y_nm: 0 };
    let rev = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::DRAW_WIRE, &json!({"from":{"x_nm":-25_400_000,"y_nm":0},"to":origin,"bends":[],"expected_revision":rev}));
    h.hover_at(at(&h, origin) + egui::vec2(150., 100.));
    h.run();
    let square = |h: &Harness<'_, PcbApp>, p: Point| {
        let center = at(h, p);
        h.output().shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Rect(r) if r.rect.center().distance(center)<0.1 && (r.rect.width()-5.).abs()<0.1 && r.stroke.color==pcbcraft_chrome::Tokens::get().schematic_wire))
    };
    let ring = |h: &Harness<'_, PcbApp>, p: Point| {
        let center = at(h, p);
        h.output()
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape,egui::Shape::Circle(c) if c.center.distance(center)<0.1 && (c.radius-2.).abs()<0.1))
    };
    assert!(square(&h, origin));
    for x in [0, 12_700_000] {
        let rev = h.state().session.revision();
        h.state_mut().run(engine::ADD, &json!({"name":"GND","style":"ground","x_nm":x,"y_nm":0,"expected_revision":rev}));
    }
    h.step();
    h.run();
    let project = h.state().session.project().unwrap();
    assert_eq!(crate::net_labels::attached(project), [project.net_labels()[0].id].into_iter().collect());
    assert!(!square(&h, origin));
    assert!(!ring(&h, origin));
    assert!(ring(&h, Point { x_nm: 12_700_000, y_nm: 0 }));
    let label = h.state().session.project().unwrap().net_labels()[0].clone();
    let rev = h.state().session.revision();
    h.state_mut().run(engine::SET, &json!({"id":label.object_id(),"name":"GND","style":"ground","x_nm":0,"y_nm":12_700_000,"expected_revision":rev}));
    h.step();
    h.run();
    assert!(square(&h, origin));
    assert!(ring(&h, Point { x_nm: 0, y_nm: 12_700_000 }));
    click(&mut h, "Undo");
    assert!(!square(&h, origin));
    assert!(!ring(&h, origin));
}

/// A wired power marker exercises the same shared commands used by GUI and MCP clients.
fn wired_power(style: &str, rotation: u16) -> Harness<'static, PcbApp> {
    let mut h = editor(1400.);
    let rev = h.state().session.revision();
    h.state_mut()
        .session
        .apply(engine::ADD, &json!({"name":"GND","style":style,"x_nm":0,"y_nm":0,"rotation":rotation,"expected_revision":rev}))
        .unwrap();
    let rev = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":25_400_000,"y_nm":0,"expected_revision":rev})).unwrap();
    let project = h.state().session.project().unwrap();
    let label = project.net_labels()[0].object_id();
    let symbol = project.symbols()[0].id();
    let rev = h.state().session.revision();
    h.state_mut()
        .session
        .apply(
            pcbcraft_engine::DRAW_WIRE,
            &json!({"from":{"symbol":symbol,"terminal":"1"},"to":{"net_label":label},"bends":[],"expected_revision":rev}),
        )
        .unwrap();
    h.run();
    h
}

/// Locate actual painted name ink in the requested view, including turned/manual text.
fn power_name(h: &Harness<'_, PcbApp>, canvas: &str, name: &str) -> egui::Pos2 {
    let rect = h.get_by_label(canvas).rect();
    h.output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == name && rect.contains(text.visual_bounding_rect().center()) => {
                Some(text.visual_bounding_rect().center())
            }
            _ => None,
        })
        .unwrap()
}

/// Two complete clicks use a realistic interval rather than the harness's slow default frames.
fn double_click_name(h: &mut Harness<'_, PcbApp>, point: egui::Pos2) {
    h.hover_at(point);
    h.run();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.input_mut().time = Some(h.ctx.input(|i| i.time) + 0.03);
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
    }
    h.input_mut().time = None;
    h.run();
}

/// Compare actual terminal membership and net names rather than an internal graph-cache pointer.
fn assert_power_connectivity(before: &pcbcraft_engine::Project, after: &pcbcraft_engine::Project) {
    let old = before.connectivity();
    let new = after.connectivity();
    for terminal in before.symbols().iter().flat_map(|symbol| symbol.terminals()) {
        assert_eq!(old.net_for(terminal), new.net_for(terminal));
        assert_eq!(old.net_index_for(terminal).map(|net| old.net_name(net)), new.net_index_for(terminal).map(|net| new.net_name(net)));
    }
}

/// Dragging a power name moves only its text at every symbol orientation and zoom. Attached
/// terminals, wire paths, net identity and the glyph stay fixed; the layout survives save/undo.
#[test]
fn power_names_drag_independently_of_their_electrical_anchors() {
    for style in ["ground", "power", "power_bar"] {
        for rotation in [0, 90, 180, 270] {
            for zoom in [1., 3.] {
                let mut h = wired_power(style, rotation);
                h.state_mut().views.active.camera.set_zoom(zoom);
                h.run();
                let before = h.state().session.snapshot().unwrap();
                let label = before.net_labels()[0].clone();
                let rev = h.state().session.revision();
                let canvas = h.get_by_label("Schematic canvas").rect();
                let ink = pcbcraft_chrome::net_label::name_ink(
                    &h.ctx.debug_painter(),
                    at(&h, label.at),
                    &label.name,
                    crate::net_labels::visual(label.style),
                    label.rotation,
                    h.state().views.active.camera.unit_step(),
                    None,
                )
                .unwrap();
                let start = power_name(&h, "Schematic canvas", "GND");
                let delta = egui::vec2(81.28, 60.96);
                let expected = h.state().views.active.camera.snap(ink.anchor + delta, canvas);
                h.drag_at(start);
                h.run();
                h.hover_at(start + delta);
                h.run();
                assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabelText { drag: Some(_), .. })), "{style} {rotation} {zoom}");
                assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
                h.drop_at(start + delta);
                h.run();
                let after = h.state().session.snapshot().unwrap();
                let moved = &after.net_labels()[0];
                assert_eq!(h.state().session.revision(), rev + 1);
                assert_eq!((&moved.name, moved.style, moved.at, moved.rotation), (&label.name, label.style, label.at, label.rotation));
                let offset = moved.text.as_ref().unwrap().offset.unwrap();
                let world = match rotation {
                    90 => Point { x_nm: offset.y_nm, y_nm: -offset.x_nm },
                    180 => Point { x_nm: -offset.x_nm, y_nm: -offset.y_nm },
                    270 => Point { x_nm: -offset.y_nm, y_nm: offset.x_nm },
                    _ => offset,
                };
                assert_eq!(Point { x_nm: label.at.x_nm + world.x_nm, y_nm: label.at.y_nm + world.y_nm }, expected);
                assert_eq!(after.symbols()[0].pins(), before.symbols()[0].pins());
                assert_eq!(after.wire_path(&after.wires()[0]), before.wire_path(&before.wires()[0]));
                assert_power_connectivity(&before, &after);
                let saved = encode_documents(after.clone()).unwrap();
                assert_eq!(encode_documents(pcbcraft_engine::persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
                click(&mut h, "Undo");
                assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
                click(&mut h, "Redo");
                assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
            }
        }
    }
}

/// A double click on GND opens an inline text box on the owning canvas. Renaming changes
/// the real net name, while cancelling leaves the complete snapshot and connection unchanged.
#[test]
fn double_clicking_power_text_edits_its_name_in_place() {
    for style in ["ground", "power", "power_bar"] {
        let mut h = wired_power(style, 0);
        let before = h.state().session.snapshot().unwrap();
        let point = power_name(&h, "Schematic canvas", "GND");
        double_click_name(&mut h, point);
        assert!(matches!(h.state().dialog, Dialog::None));
        h.get_by_label("Edit Net name");
        name(&mut h, "GNDA");
        h.key_press(Key::Escape);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        double_click_name(&mut h, point);
        name(&mut h, "GNDA");
        h.key_press(Key::Enter);
        h.run();
        let project = h.state().session.project().unwrap();
        let label = &project.net_labels()[0];
        assert_eq!(label.name, "GNDA");
        assert_eq!(label.at, before.net_labels()[0].at);
        assert_eq!(project.wire_path(&project.wires()[0]), before.wire_path(&before.wires()[0]));
        let terminal = project.symbols()[0].terminals()[0];
        let net = project.connectivity().net_index_for(terminal).unwrap();
        assert_eq!(project.connectivity().net_name(net), "GNDA");
        click(&mut h, "Undo");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Hover R and keyboard pickup address the name; rotating during a drag remains a preview and
/// commits with its final offset in one history entry. Glyph orientation never follows text R.
#[test]
fn power_name_rotation_and_keyboard_move_preserve_the_glyph() {
    let mut h = wired_power("ground", 90);
    let before = h.state().session.snapshot().unwrap();
    let point = power_name(&h, "Schematic canvas", "GND");
    h.hover_at(point);
    h.run();
    h.key_press(Key::R);
    h.run();
    let rotated = h.state().session.snapshot().unwrap();
    assert_eq!(rotated.net_labels()[0].rotation, 90);
    assert_eq!(rotated.net_labels()[0].text.as_ref().unwrap().rotation, 90);
    assert_eq!(rotated.net_labels()[0].at, before.net_labels()[0].at);
    click(&mut h, "Undo");
    let start = power_name(&h, "Schematic canvas", "GND");
    h.drag_at(start);
    h.run();
    let end = start + egui::vec2(70., 50.);
    h.hover_at(end);
    h.run();
    h.key_press(Key::R);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.drop_at(end);
    h.run();
    let moved = h.state().session.snapshot().unwrap();
    assert_eq!(moved.net_labels()[0].rotation, 90);
    assert_eq!(moved.net_labels()[0].text.as_ref().unwrap().rotation, 90);
    assert_eq!(moved.net_labels()[0].at, before.net_labels()[0].at);
    assert_power_connectivity(&before, &moved);
    click(&mut h, "Undo");
    let point = power_name(&h, "Schematic canvas", "GND");
    h.hover_at(point);
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabelText { drag: None, .. })));
    let end = point + egui::vec2(80., -50.);
    click_canvas_at(&mut h, end);
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].at, before.net_labels()[0].at);
    click(&mut h, "Undo");
    let point = power_name(&h, "Schematic canvas", "GND");
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    click(&mut h, "Move text");
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabelText { drag: None, .. })));
    h.key_press(Key::Escape);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Name drags and text-property actions follow the addressed split pane's camera and source.
#[test]
fn power_name_drag_belongs_to_the_focused_split_pane() {
    let mut h = wired_power("ground", 0);
    h.set_size(egui::vec2(1600., 1000.));
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let before = h.state().session.snapshot().unwrap();
    let start = power_name(&h, "Schematic canvas 2", "GND");
    h.drag_at(start);
    h.run();
    h.hover_at(start + egui::vec2(60., 40.));
    h.run();
    assert_eq!(h.state().views.active.id(), ViewId(1));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.drop_at(start + egui::vec2(60., 40.));
    h.run();
    let after = h.state().session.snapshot().unwrap();
    assert_eq!(after.net_labels()[0].at, before.net_labels()[0].at);
    assert!(after.net_labels()[0].text.as_ref().unwrap().offset.is_some());
    assert_power_connectivity(&before, &after);
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let point = power_name(&h, "Schematic canvas 2", "GND");
    double_click_name(&mut h, point);
    name(&mut h, "GNDA");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].name, "GNDA");
}

/// Power-name previews belong to their source view/revision and are discarded on cancellation,
/// focus loss, out-of-canvas release, disabled power picking or a concurrent edit.
#[test]
fn interrupted_power_name_drags_leave_the_document_unchanged() {
    for mode in ["escape", "focus", "outside", "filter", "stale"] {
        let mut h = wired_power("ground", 0);
        let start = power_name(&h, "Schematic canvas", "GND");
        let end = start + egui::vec2(80., 50.);
        h.drag_at(start);
        h.run();
        h.hover_at(end);
        h.run();
        h.key_press(Key::R);
        h.run();
        let mut release = end;
        match mode {
            "escape" => h.key_press(Key::Escape),
            "focus" => h.event(egui::Event::WindowFocused(false)),
            "outside" => release = egui::pos2(2., 2.),
            "filter" => h.state_mut().views.active.filters.symbols = false,
            _ => h.state_mut().run("project.rename", &json!({"name":"Concurrent"})),
        }
        h.run();
        let before = h.state().session.snapshot().unwrap();
        h.drop_at(release);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "{mode}");
        let label = &h.state().session.project().unwrap().net_labels()[0];
        assert_eq!(label.at, Point { x_nm: 0, y_nm: 0 });
        assert!(label.text.is_none());
    }
}
