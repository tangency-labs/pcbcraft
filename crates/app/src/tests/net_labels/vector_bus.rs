//! The Bus scope in the label form, the bus drawing on the canvas and refused bus names.
use super::*;

/// The open Scope menu's option with this text. The ribbon's disabled Bus wire tool shares the
/// word, so the option is the lowest match: the menu opens below the form's Scope field.
fn option<'h>(h: &'h Harness<'_, PcbApp>, text: &str) -> egui_kittest::Node<'h> {
    h.get_all_by_label(text).max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap()
}

/// Open the label form with a typed name and choose a scope through the real combo box.
fn form_with_scope(h: &mut Harness<'_, PcbApp>, text: &str, scope: &str) {
    h.key_press(Key::L);
    h.run();
    name(h, text);
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Scope").click();
    h.run();
    option(h, scope).click();
    h.run();
}

/// The Scope menu lists Bus after Local and Global. Choosing it edits nothing until Place arms
/// the placement tool with a bus label. The harness ends on the filled form with Bus chosen.
#[test]
fn bus_scope_is_offered_in_the_label_form() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.key_press(Key::L);
    h.run();
    name(&mut h, "D[0..7]");
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Scope").click();
    h.run();
    let local = option(&h, "Local").rect().top();
    let global = option(&h, "Global").rect().top();
    let bus = option(&h, "Bus").rect().top();
    assert!(local < global && global < bus, "Bus follows Local and Global");
    option(&h, "Bus").click();
    h.run();
    assert!(h.query_by_label("Local").is_none(), "the menu closes after a choice");
    assert!(matches!(h.state().dialog, Dialog::NetLabel(_)), "the label form stays open");
    assert!(h.state().views.active.schematic.tool.is_none());
    assert_eq!(h.state().session.revision(), revision, "choosing a scope edits nothing");
    click(&mut h, "Place");
    assert!(
        matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabel { ref label, .. }) if label.style == Style::Bus && label.name == "D[0..7]")
    );
    h.key_press(Key::Escape);
    h.run();
    form_with_scope(&mut h, "D[0..7]", "Bus");
}

/// The open Scope menu lists Local, Global and Bus in that order, and
/// opening it edits nothing. The harness ends with the menu open.
#[test]
fn scope_menu_lists_local_global_and_bus() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.key_press(Key::L);
    h.run();
    name(&mut h, "D[0..7]");
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Scope").click();
    h.run();
    let tops = ["Local", "Global", "Bus"].map(|text| option(&h, text).rect().top());
    assert!(tops[0] < tops[1] && tops[1] < tops[2], "{tops:?}");
    assert!(matches!(h.state().dialog, Dialog::NetLabel(_)), "the label form stays open under the menu");
    assert_eq!(h.state().session.revision(), revision, "opening the menu edits nothing");
}

/// Placing a bus label from the form adds one bus label on the clicked point, and one Undo
/// removes it. The harness ends on the placed bus label.
#[test]
fn bus_label_placed_from_the_form_is_one_undo_step() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    form_with_scope(&mut h, "D[0..7]", "Bus");
    click(&mut h, "Place");
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabel { ref label, .. }) if label.style == Style::Bus));
    let target = Point { x_nm: 0, y_nm: 0 };
    let screen = at(&h, target);
    click_canvas_at(&mut h, screen);
    h.key_press(Key::Escape);
    h.run();
    let project = h.state().session.project().unwrap();
    let [label] = project.net_labels() else { panic!("exactly one label is placed") };
    assert_eq!((label.name.as_str(), label.style, label.at), ("D[0..7]", Style::Bus, target));
    assert_eq!(h.state().session.revision(), revision + 1);
    assert!(h.state().message.is_none());
    click(&mut h, "Undo");
    assert!(h.state().session.project().unwrap().net_labels().is_empty());
    click(&mut h, "Redo");
    assert_eq!(h.state().session.project().unwrap().net_labels().len(), 1);
}

/// A malformed bus name is refused in the form with the grammar's reason and starts no
/// placement; the same text is accepted as an ordinary local label, and the engine refuses it
/// as a bus for every caller. The harness ends on the form showing the refusal.
#[test]
fn malformed_bus_name_is_refused_with_its_reason() {
    let mut h = editor(1400.);
    let original = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(engine::ADD, &json!({"name":"D[0..64]","style":"local","x_nm":0,"y_nm":0,"expected_revision":revision}));
    assert!(h.state().message.is_none());
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].style, Style::Local);
    h.state_mut().run(engine::ADD, &json!({"name":"D[0..64]","style":"bus","x_nm":0,"y_nm":-2_540_000,"expected_revision":revision + 1}));
    h.run();
    assert!(h.state().message.as_deref().unwrap_or_default().contains("at most 64 members"));
    assert_eq!(h.state().session.project().unwrap().net_labels().len(), 1);
    click(&mut h, "Undo");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), original);

    form_with_scope(&mut h, "D[0..64]", "Bus");
    click(&mut h, "Place");
    assert!(h.query_by_label_contains("Enter a bus name such as D[0..7]: a bus carries at most 64 members").is_some());
    assert!(matches!(h.state().dialog, Dialog::NetLabel(_)), "the form stays open on the refusal");
    assert!(h.state().views.active.schematic.tool.is_none(), "no placement starts");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), original);
}

/// A wire started on a bus label's point starts on open canvas instead: the finished wire
/// exists but neither end is the label. Selecting the label then opens its name and Bus scope
/// in the properties panel, where the harness ends.
#[test]
fn bus_label_is_selectable_but_never_a_wire_end() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.state_mut().run(engine::ADD, &json!({"name":"A[3..0]","style":"bus","x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    let point = at(&h, Point { x_nm: 0, y_nm: 0 });

    h.key_press(Key::W);
    h.run();
    click_canvas_at(&mut h, point);
    h.hover_at(point + egui::vec2(120., 0.));
    h.run();
    h.key_press(Key::Enter);
    h.run();
    h.key_press(Key::Escape);
    h.run();
    let project = h.state().session.project().unwrap();
    let [wire] = project.wires() else { panic!("one wire is drawn from the label's point") };
    assert!(!wire.endpoints().iter().any(|end| matches!(end, pcbcraft_engine::WireEndpoint::NetLabel(_))), "no wire ends on the bus label");
    assert_eq!(project.net_labels()[0].style, Style::Bus);

    click_canvas_at(&mut h, point + egui::vec2(25., -10.));
    assert!(h.state().views.active.schematic.selection.single_net_label().is_some());
    h.state_mut().properties_open = true;
    h.run();
    assert_eq!(h.get_by_label("Net name").value().as_deref(), Some("A[3..0]"));
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Scope");
}
