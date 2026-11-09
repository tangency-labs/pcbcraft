use super::*;
use crate::tests::{click, click_canvas_at, harness};
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use std::sync::Arc;

/// External text requires an explicit electrical interpretation before starting a native preview.
#[test]
fn plain_lines_offer_explicit_net_label_conversion_and_cancel_without_history() {
    let mut h = harness();
    h.state_mut().run("project.new", &json!({"name":"Pasted names"}));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Paste("VCC\nDATA\nGND".into()));
    h.run();
    assert!(matches!(h.state().dialog, Dialog::ExternalPasteOptions(_)));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Cancel");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.event(Event::Paste("VCC\nDATA\nGND".into()));
    h.run();
    click(&mut h, "Paste labels");
    assert!(h.state().views.active.schematic.clipboard.active());
    let rect = h.get_by_label("Schematic canvas").rect();
    click_canvas_at(&mut h, rect.center());
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.project().unwrap().net_labels().len(), 3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
}

/// Spreadsheet coordinates can retain exact positions, units and closure without touching setup.
#[test]
fn spreadsheet_coordinates_become_one_outline_after_explicit_conversion() {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({"create_board":true}));
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Paste("x_mil\ty_mil\n100\t200\n1100\t200\n1100\t700\n100\t200".into()));
    h.run();
    assert!(matches!(h.state().dialog, Dialog::ExternalPasteOptions(_)));
    h.get_by_label("Close outline").click();
    h.get_by_label("Keep source coordinates").click();
    h.run();
    click(&mut h, "Paste outline");
    assert_eq!(h.state().message, None);
    let b = h.state().session.project().unwrap().boards().next().unwrap();
    assert!(b.setup().is_none());
    assert_eq!(b.drawings().len(), 1);
    assert_eq!(b.drawings()[0].points[0], pcbcraft_engine::schematic::Point { x_nm: 2_540_000, y_nm: 5_080_000 });
    assert!(b.drawings()[0].closed);
    assert_eq!(b.drawings()[0].points.len(), 3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
}

/// A native version error cannot be reinterpreted as electrical text, and a stale external form
/// cannot move its conversion into a newer design revision.
#[test]
fn native_and_stale_conversion_failures_preserve_destination() {
    let mut h = harness();
    h.state_mut().run("project.new", &json!({}));
    h.run();
    h.event(Event::Paste("{\"format\":\"pcbcraft-clipboard/99\"}".into()));
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    h.event(Event::Paste("A\nB".into()));
    h.run();
    h.state_mut().session.apply("project.rename", &json!({"name":"New revision"})).unwrap();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Paste labels");
    assert!(matches!(&h.state().dialog,Dialog::ExternalPasteOptions(d) if d.error.as_ref().is_some_and(|s| s.contains("destination changed"))));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Invalid conversion remains editable, and text fields keep ordinary clipboard editing while
/// the form owns input; neither path can publish an unintended design change.
#[test]
fn invalid_and_text_field_pastes_remain_in_the_conversion_draft() {
    let mut h = harness();
    h.state_mut().run("project.new", &json!({}));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Paste("A\n\nB".into()));
    h.run();
    click(&mut h, "Paste labels");
    assert!(matches!(&h.state().dialog,Dialog::ExternalPasteOptions(d) if d.error.is_some()));
    h.get_by_label("Clipboard text").click();
    h.run();
    h.event(Event::Paste("MORE".into()));
    h.run();
    assert!(matches!(&h.state().dialog,Dialog::ExternalPasteOptions(d) if d.text.contains("MORE")));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Cancel");
    h.event(Event::Paste("X".repeat(engine::MAX_TEXT_BYTES + 1)));
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().message.as_ref().is_some_and(|s| s.contains("64 KiB")));
}

/// External conversion forms use the same host ownership as native paste options.
#[test]
fn external_conversion_form_stays_in_its_initiating_host() {
    let mut h = harness();
    h.state_mut().run("project.new", &json!({}));
    h.run();
    let source = h.state().views.active.id();
    let host = h.state_mut().views.open_window(source).unwrap();
    h.state_mut().bind_editor_host(host);
    h.state_mut().begin_external_clipboard("SIGNAL".into());
    assert_eq!(h.state().external_paste_options_host(), Some(host));
    let before = h.state().session.snapshot().unwrap();
    h.run();
    click(&mut h, "Cancel");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
