//! Moving a selection to a new sheet through its form: opening it from the ribbon and the
//! canvas menu, a typed name, a refused name, an empty name and keyboard use.
use super::*;
use pcbcraft_engine::{
    CONNECT_PINS, PLACE_RESISTOR,
    persistence::{Documents, encode_documents},
    selection::Selection,
};

/// Two resistors joined by one wire, with the first one selected.
fn split_harness() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Split"}));
    for x_nm in [0, 50_800_000] {
        app.run(PLACE_RESISTOR, &json!({"x_nm":x_nm,"y_nm":0,"expected_revision":app.session.revision()}));
    }
    let symbols: Vec<_> = app.session.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    let (from, to) = (json!({"symbol":symbols[0],"terminal":"2"}), json!({"symbol":symbols[1],"terminal":"1"}));
    app.run(CONNECT_PINS, &json!({"from":from,"to":to,"expected_revision":app.session.revision()}));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(40).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h.state_mut().select(Selection { symbols: [symbols[0].clone()].into(), ..Selection::default() });
    h.run();
    h
}

/// Open the form through the Sheets ribbon group's menu.
fn open(h: &mut Harness<'_, PcbApp>) {
    click_tab(h, "Sheets");
    click(h, "Sheets menu");
    click(h, "Move to new sheet…");
    assert!(matches!(h.state().dialog, Dialog::SplitSheet(_)));
}

/// The project as saved, to show that a step changed nothing.
fn documents(h: &Harness<'_, PcbApp>) -> Documents {
    encode_documents(h.state().session.snapshot().unwrap()).unwrap()
}

/// Open state: the form suggests a free sheet name, shows what would move and the one port
/// the boundary wire needs, and changes nothing.
#[test]
fn split_form_opens_with_the_preview_for_the_suggested_name() {
    let mut h = split_harness();
    let before = documents(&h);
    let revision = h.state().session.revision();
    open(&mut h);
    assert_eq!(h.get_by_label("Name").value().as_deref(), Some("Sheet 2"));
    h.get_by_label("Moves 1 symbol to a new sheet placed here as a sheet symbol.");
    h.get_by_label("Ports (1)");
    assert!(!h.get_by_label("Move to Sheet").accesskit_node().is_disabled());
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(documents(&h), before);
}

/// Filled state: a typed name is previewed again and can be moved.
#[test]
fn split_form_previews_a_typed_name() {
    let mut h = split_harness();
    open(&mut h);
    clear_name(&mut h);
    h.get_by_label("Name").type_text("Filter");
    h.run();
    assert_eq!(h.get_by_label("Name").value().as_deref(), Some("Filter"));
    h.get_by_label("Ports (1)");
    assert!(h.query_by_label_contains("already exists").is_none());
    assert!(!h.get_by_label("Move to Sheet").accesskit_node().is_disabled());
    assert!(matches!(h.state().dialog, Dialog::SplitSheet(_)));
}

/// Move to Sheet makes the named child sheet with the selected resistor as one undo step;
/// Cancel on a reopened form changes nothing.
#[test]
fn split_form_moves_the_selection_and_cancel_changes_nothing() {
    let mut h = split_harness();
    let before = h.state().session.snapshot().unwrap();
    let selected = h.state().views.active.schematic.selection.single_symbol().unwrap().to_owned();
    open(&mut h);
    clear_name(&mut h);
    h.get_by_label("Name").type_text("Filter");
    h.run();
    click(&mut h, "Move to Sheet");
    assert!(matches!(h.state().dialog, Dialog::None));
    let project = h.state().session.project().unwrap();
    let child = project.sheets().find(|sheet| sheet.name == "Filter").unwrap().target;
    let moved: Vec<_> = project.for_sheet(child).unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    assert_eq!(moved, std::slice::from_ref(&selected));
    assert_eq!(project.for_sheet(child).unwrap().hierarchical_labels().len(), 1);
    assert!(h.state().views.active.schematic.selection.is_empty());
    click(&mut h, "Undo");
    assert_same_content(&before, &h.state().session.snapshot().unwrap());

    h.state_mut().select(Selection { symbols: [selected].into(), ..Selection::default() });
    h.run();
    let unchanged = documents(&h);
    open(&mut h);
    click(&mut h, "Cancel");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(documents(&h), unchanged);
}

/// Error state: a name another sheet already has, in any case, shows the engine's reason,
/// keeps the form open and offers no Move to Sheet; Enter does nothing.
#[test]
fn split_form_shows_why_a_taken_name_is_refused() {
    let mut h = split_harness();
    let taken = h.state().session.project().unwrap().sheets().next().unwrap().name.to_owned();
    let before = documents(&h);
    open(&mut h);
    assert!(h.query_by_label_contains("already exists").is_none());
    clear_name(&mut h);
    h.get_by_label("Name").type_text(&taken.to_lowercase());
    h.run();
    assert!(h.query_by_label_contains("already exists").is_some());
    assert!(h.get_by_label("Move to Sheet").accesskit_node().is_disabled());
    h.key_press(Key::Enter);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::SplitSheet(_)));
    assert!(h.query_by_label_contains("already exists").is_some());
    assert_eq!(documents(&h), before);
}

/// Empty state: with no name the form asks for one and Move to Sheet is unavailable.
#[test]
fn split_form_asks_for_a_name_when_it_is_empty() {
    let mut h = split_harness();
    let before = documents(&h);
    open(&mut h);
    assert!(h.query_by_label("Type a name for the new sheet.").is_none());
    clear_name(&mut h);
    h.get_by_label("Type a name for the new sheet.");
    assert!(h.get_by_label("Move to Sheet").accesskit_node().is_disabled());
    h.key_press(Key::Enter);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::SplitSheet(_)));
    assert_eq!(documents(&h), before);
}

/// Opened from the canvas menu, the Name field has focus, so a name and Enter move the
/// selection without the pointer.
#[test]
fn split_form_applies_a_typed_name_on_enter() {
    let mut h = split_harness();
    open_from_canvas(&mut h);
    clear_name(&mut h);
    h.get_by_label("Name").type_text("Keys");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().session.project().unwrap().sheets().any(|sheet| sheet.name == "Keys"));
}

/// Keyboard focus state: the form opened from the selected symbol's context menu has the
/// Name field focused with the suggested name, ready to type over.
#[test]
fn split_form_opens_from_the_canvas_menu_with_the_name_focused() {
    let mut h = split_harness();
    open_from_canvas(&mut h);
    assert!(h.get_by_label("Name").is_focused());
    assert_eq!(h.get_by_label("Name").value().as_deref(), Some("Sheet 2"));
    h.get_by_label("Ports (1)");
}

/// Open the form from the selected symbol's canvas context menu.
fn open_from_canvas(h: &mut Harness<'_, PcbApp>) {
    let position = h.state().session.project().unwrap().symbols().first().unwrap().position;
    let canvas = h.get_by_label("Schematic canvas").rect();
    let at = h.state().views.active.camera.screen(position, canvas);
    h.hover_at(at);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    click(h, "Move to New Sheet…");
    assert!(matches!(h.state().dialog, Dialog::SplitSheet(_)));
}
