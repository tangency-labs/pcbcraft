//! Moving a selection to a new sheet through its form: opening it from the ribbon and the
//! canvas menu, a typed name, a refused name, an empty name, keyboard use, a richer plan
//! and a refusal when moving.
use super::*;
use pcbcraft_engine::{
    CONNECT_PINS, PLACE_RESISTOR,
    library::sources,
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
    click(h, "Move to New Sheet…");
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
    h.get_by_label("Moves 1 symbol to a new sheet; a sheet symbol takes its place here.");
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

/// Right-clicking a label targets that label, even when a different object was selected.
#[test]
fn split_form_opens_for_the_net_label_under_the_context_menu() {
    let mut h = split_harness();
    let app = h.state_mut();
    let previous = app.views.active.schematic.selection.clone();
    app.run(
        pcbcraft_engine::net_labels::ADD,
        &json!({"name":"MOVE_ME","style":"local","x_nm":0,"y_nm":25_400_000,"expected_revision":app.session.revision()}),
    );
    app.select(previous);
    let Some(label) = app.session.project().and_then(|project| project.net_labels().last()) else { panic!("expected a net label") };
    let (id, position) = (label.id, label.at);
    h.run();
    let canvas = h.get_by_label("Schematic canvas").rect();
    let at = h.state().views.active.camera.screen(position, canvas);
    h.hover_at(at);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    click(&mut h, "Move to New Sheet…");
    let Dialog::SplitSheet(edit) = &h.state().dialog else { panic!("expected the split form") };
    assert_eq!(edit.items, Selection::one_net_label(id));
    h.get_by_label("Moves 1 net label to a new sheet; a sheet symbol takes its place here.");
}

/// Show the marker notice even when no selected wire stays, and omit it otherwise.
#[test]
fn split_form_counts_a_selected_no_connect_marker_that_stays() {
    let mut h = split_harness();
    const NOTICE: &str = "1 selected no-connect marker stays on this sheet.";
    open(&mut h);
    assert!(h.query_by_label(NOTICE).is_none());
    click(&mut h, "Cancel");

    let app = h.state_mut();
    let Some(other) = app.session.project().and_then(|project| project.symbols().get(1)) else { panic!("expected the second resistor") };
    let target = pcbcraft_engine::no_connect::Target { part: other.component_id(), terminal: "2".into() };
    let mut items = app.views.active.schematic.selection.clone();
    app.run(pcbcraft_engine::no_connect::SET, &json!({"target":target,"marked":true,"expected_revision":app.session.revision()}));
    items.no_connects.insert(target.into());
    app.select(items);
    h.run();
    open(&mut h);
    h.get_by_label(NOTICE);
    assert!(h.query_by_label_contains("selected wire stays").is_none());
}

/// Wires and markers follow what they attach to, so with only a wire selected, or nothing,
/// Move to New Sheet… is unavailable in the Sheets menu and the form does not open.
#[test]
fn move_to_new_sheet_needs_a_symbol_or_net_label() {
    let mut h = split_harness();
    let wire = h.state().session.project().unwrap().wires().first().unwrap().id();
    for selection in [Selection { wires: [wire].into(), ..Selection::default() }, Selection::default()] {
        h.state_mut().select(selection);
        h.run();
        click_tab(&mut h, "Sheets");
        click(&mut h, "Sheets menu");
        assert!(h.get_by_label("Move to New Sheet…").accesskit_node().is_disabled());
        h.key_press(Key::Escape);
        h.run();
        h.state_mut().begin_split_sheet();
        assert!(matches!(h.state().dialog, Dialog::None));
    }
}

/// Place a bundled ground symbol whose value names its global net.
fn power(app: &mut PcbApp, value: &str, x_nm: i64, y_nm: i64) -> String {
    let d = app.session.libraries().latest_symbol("power", "gnd").unwrap();
    let digest = sources::symbol_digest(&d).unwrap();
    let revision = app.session.revision();
    app.run(
        pcbcraft_engine::PLACE_SYMBOL,
        &json!({"library":"power","definition":"gnd","definition_revision":d.revision(),"definition_digest":digest,"value":value,"x_nm":x_nm,"y_nm":y_nm,"expected_revision":revision}),
    );
    app.session.project().unwrap().symbols().last().unwrap().id()
}

/// Place a resistor and return its symbol.
fn resistor(app: &mut PcbApp, x_nm: i64, y_nm: i64) -> String {
    app.run(PLACE_RESISTOR, &json!({"x_nm":x_nm,"y_nm":y_nm,"expected_revision":app.session.revision()}));
    app.session.project().unwrap().symbols().last().unwrap().id()
}

/// Join two terminals with a wire.
fn wire(app: &mut PcbApp, from: (&str, &str), to: (&str, &str)) {
    let (from, to) = (json!({"symbol":from.0,"terminal":from.1}), json!({"symbol":to.0,"terminal":to.1}));
    app.run(CONNECT_PINS, &json!({"from":from,"to":to,"expected_revision":app.session.revision()}));
    assert!(app.message.is_none(), "{:?}", app.message);
}

/// Unit A of a dual amplifier, a resistor and a ground symbol selected; unit B, a resistor,
/// another ground and an AGND symbol outside. Unit A's output crosses on an unnamed net, the
/// resistor reaches AGND outside, and the two grounds share the global GND net.
fn rich_harness() -> Harness<'static, PcbApp> {
    let grid = pcbcraft_engine::schematic::GRID_NM;
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Split"}));
    app.run(
        sources::LOAD,
        &json!({"source":"units","documents":{
            "library.json":include_str!("../../../../examples/native-parts/library.json"),
            "symbols/dual.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json"),
        }}),
    );
    let definition = app.session.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
    let revision = app.session.revision();
    app.place_catalog_symbol(&definition, json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
    let amplifier = app.session.project().unwrap().symbols().first().unwrap().clone();
    let revision = app.session.revision();
    app.run(
        pcbcraft_engine::PLACE_UNIT,
        &json!({"component":amplifier.component_id(),"unit":"b","x_nm":100 * grid,"y_nm":0,"expected_revision":revision}),
    );
    let inner = resistor(&mut app, 0, 20 * grid);
    let load = resistor(&mut app, 50 * grid, 0);
    let ground_in = power(&mut app, "GND", 0, 40 * grid);
    let ground_out = power(&mut app, "GND", 50 * grid, 40 * grid);
    let analog = power(&mut app, "AGND", 70 * grid, 20 * grid);
    let unit_a = amplifier.id();
    wire(&mut app, (&unit_a, "1"), (&load, "1"));
    wire(&mut app, (&inner, "2"), (&analog, "1"));
    wire(&mut app, (&inner, "1"), (&ground_in, "1"));
    wire(&mut app, (&load, "2"), (&ground_out, "1"));
    assert!(app.message.is_none(), "{:?}", app.message);
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(40).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h.state_mut().select(Selection { symbols: [unit_a, inner, ground_in].into(), ..Selection::default() });
    h.run();
    h
}

/// Rich plan state: several ports, one with a generated name and one named by the AGND
/// symbol outside, the ground net that keeps connecting by name, and the amplifier whose
/// other unit stays behind.
#[test]
fn split_form_lists_ports_global_nets_and_split_parts() {
    let mut h = rich_harness();
    let before = documents(&h);
    open(&mut h);
    assert!(h.query_by_label_contains("Moves 3 symbols").is_some());
    assert!(h.query_by_label_contains("Ports (").is_some());
    assert!(h.query_by_label_contains("AGND · ").is_some());
    assert!(h.query_by_label_contains("(generated name)").is_some());
    h.get_by_label("Connected by name, no port needed");
    assert!(h.query_all_by_label("GND").next().is_some());
    assert!(h.query_by_label_contains("keeps other units on this sheet").is_some());
    assert!(!h.get_by_label("Move to Sheet").accesskit_node().is_disabled());
    assert_eq!(documents(&h), before);
}

/// Apply refusal state: when the engine refuses the move itself, its reason shows under the
/// plan, the form stays open with the name kept, and nothing changes. The form's selection
/// is swapped for one wire after its preview, which the engine refuses on Move to Sheet.
#[test]
fn split_form_shows_a_refusal_from_the_move_under_the_plan() {
    let mut h = split_harness();
    let before = documents(&h);
    let wire = h.state().session.project().unwrap().wires().first().unwrap().id();
    open(&mut h);
    h.get_by_label("Ports (1)");
    if let Dialog::SplitSheet(edit) = &mut h.state_mut().dialog {
        edit.items = Selection { wires: [wire].into(), ..Selection::default() };
    }
    click(&mut h, "Move to Sheet");
    assert!(matches!(h.state().dialog, Dialog::SplitSheet(_)));
    assert!(h.query_by_label_contains("Select at least one symbol or net label to move.").is_some());
    h.get_by_label("Ports (1)");
    assert_eq!(h.get_by_label("Name").value().as_deref(), Some("Sheet 2"));
    assert_eq!(documents(&h), before);
}
