//! Review, assignment and cursor placement share the engine command and approval boundaries.
use super::*;
use pcbcraft_engine::{
    ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT, PREVIEW_UPDATE,
    persistence::{decode_documents, encode_documents},
};

/// Apply an ordinary revision-bound fixture command through the application's shared dispatcher.
fn edit(h: &mut Harness<'_, PcbApp>, command: &str, mut params: Value) {
    if !matches!(command, "edit.undo" | "edit.redo") {
        params["expected_revision"] = json!(h.state().session.revision());
    }
    h.state_mut().run(command, &params);
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h.run();
}

/// One original captured component and optional pinned package keep tests independent of libraries on disk.
fn fixture(assigned: bool) -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400.0, 900.0));
    resistor_with_board_at_origin(&mut h);
    if assigned {
        assign(&mut h, "resistor-0402", json!({"1":"1","2":"2"}));
    }
    h
}

/// Resolve a supported pinned package from the engine catalog, then assign explicit terminal identities.
fn assign(h: &mut Harness<'_, PcbApp>, definition: &str, mapping: Value) {
    let package = h.state().session.project().unwrap().footprint_catalog().into_iter().find(|package| package.id == definition).unwrap();
    let component = h.state().session.project().unwrap().components().next().unwrap().id();
    edit(
        h,
        ASSIGN_FOOTPRINT,
        json!({"component":component,"library":package.library,"definition":package.id,"definition_revision":package.revision,"mapping":mapping}),
    );
}

/// Place the fixture's actual canonical component rather than assuming allocator order.
fn place_first(h: &mut Harness<'_, PcbApp>, x: i64, y: i64) {
    let component = h.state().session.project().unwrap().components().next().unwrap().id();
    edit(h, PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":x,"y_nm":y}));
}

/// Change a visible property through its current symbol while retaining its annotation.
fn change_value(h: &mut Harness<'_, PcbApp>) {
    let project = h.state().session.project().unwrap();
    let symbol = project.symbols().first().unwrap().id();
    let reference = project.components().next().unwrap().reference().to_owned();
    edit(h, pcbcraft_engine::SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":reference,"value":"2k"}));
}

/// Open from the promoted Capture action, exercising normal availability and modal ownership.
fn open(h: &mut Harness<'_, PcbApp>) {
    h.remove_cursor();
    h.run();
    click(h, "Update PCB from schematic");
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)), "dialog {:?}; message {:?}", h.state().dialog, h.state().message);
}

/// Opening, Enter and dismissal never create a board. Explicit creation uses the primary
/// action as a separate undoable edit, with title close and Escape discarding the draft.
#[test]
fn no_board_review_cancel_and_explicit_creation_have_distinct_history() {
    let mut h = harness();
    h.state_mut().start_new_project();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    open(&mut h);
    h.key_press(Key::Enter);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Close Update PCB from Schematic");
    assert_eq!(h.state().session.revision(), revision);
    assert!(h.state().session.project().unwrap().board_target().is_none());

    open(&mut h);
    h.key_press(Key::Escape);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));

    open(&mut h);
    click(&mut h, "Create board");
    let created = h.state().session.snapshot().unwrap();
    assert!(created.board_target().is_some());
    assert_eq!(h.state().session.revision(), revision + 1);
    assert!(h.get_by_label("Update PCB").accesskit_node().is_disabled());
    click(&mut h, "Close");
    assert!(Arc::ptr_eq(&created, &h.state().session.snapshot().unwrap()));
    edit(&mut h, "edit.undo", json!({}));
    assert!(h.state().session.project().unwrap().board_target().is_none());
}

/// Update accepts changes once, opens a disposable placement and resumes unplaced work after reopen.
#[test]
fn initial_update_apply_undo_reopen_and_noop_use_the_real_dialog() {
    let mut h = fixture(true);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    open(&mut h);
    h.get_by_label("Add footprint R1: basic-components / resistor-0402 · r4.");
    assert_eq!(h.state().session.revision(), revision);
    click(&mut h, "Update PCB");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().views.active.board.placement.is_some());
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 0);
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().views.active.board.placement.is_none());
    let accepted = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    edit(&mut h, "edit.undo", json!({}));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    edit(&mut h, "edit.redo", json!({}));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), accepted);
    h.state_mut().session.open_snapshot(decode_documents(&accepted).unwrap());
    h.run();
    open(&mut h);
    h.get_by_label("PCB is up to date.");
    assert!(h.get_by_label("Update PCB").accesskit_node().is_disabled());
    h.get_by_label("1 footprint ready to place");
    click(&mut h, "Place footprints");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), accepted);
    click(&mut h, "PCB canvas");
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 1);
    edit(&mut h, "edit.undo", json!({}));
    super::assert_same_content(&decode_documents(&accepted).unwrap(), &h.state().session.snapshot().unwrap());
}

/// Missing packages explain the blocker; assignment refreshes the review without extra confirmation.
#[test]
fn package_blocker_chooser_returns_to_review_without_partial_apply() {
    let mut h = fixture(false);
    let before = h.state().session.snapshot().unwrap();
    open(&mut h);
    h.get_by_label("Total warnings: 0, errors: 1.");
    assert!(h.get_by_label("Update PCB").accesskit_node().is_disabled());
    click(&mut h, "Assign footprint to R1");
    click(&mut h, "basic-components / Resistor 0402 (1005 metric) · r4");
    // Equal numbers are preselected, but a manually invalid map must remain an atomic refusal.
    click(&mut h, "Terminal 1");
    click(&mut h, "Pad 2");
    click(&mut h, "Assign");
    assert!(matches!(h.state().dialog, Dialog::Footprint(_)));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Match numbers");
    click(&mut h, "Assign");
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    assert!(!h.get_by_label("Update PCB").accesskit_node().is_disabled());
    assert!(h.query_by_label("Review again").is_none());
    click(&mut h, "Update PCB");
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
    click(&mut h, "PCB canvas");
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 1);
}

/// Same-revision token replacement and ordinary edits require re-review; reopen cannot revive the form.
#[test]
fn stale_or_replaced_plan_cannot_apply_or_cancel_another_clients_preview() {
    let mut h = fixture(true);
    open(&mut h);
    let old = h.state().session.cached_board_update().unwrap().plan_token;
    let revision = h.state().session.revision();
    h.state_mut().session.apply(PREVIEW_UPDATE, &json!({"expected_revision":revision})).unwrap();
    let newer = h.state().session.cached_board_update().unwrap().plan_token;
    assert_ne!(old, newer);
    h.run();
    assert!(h.get_by_label("Update PCB").accesskit_node().is_disabled());
    click(&mut h, "Close");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.cached_board_update().unwrap().plan_token, newer);
    open(&mut h);
    change_value(&mut h);
    assert!(h.get_by_label("Update PCB").accesskit_node().is_disabled());
    click(&mut h, "Review again");
    assert!(!h.get_by_label("Update PCB").accesskit_node().is_disabled());
    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    h.run();
    assert!(h.get_by_label("Update PCB").accesskit_node().is_disabled());
    assert!(h.query_by_label("Review again").is_none());
    click(&mut h, "Close");
}

/// Existing unrecorded placements require deliberate adoption before pending changes are inferred.
#[test]
fn existing_board_adoption_is_explicit_and_survives_cancel() {
    let mut h = fixture(true);
    place_first(&mut h, 10_000_000, 20_000_000);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    open(&mut h);
    h.get_by_label("Use current state");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Close");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    open(&mut h);
    click(&mut h, "Use current state");
    h.get_by_label("PCB is up to date.");
    let adopted = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert_ne!(adopted, before);
    click(&mut h, "Close");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), adopted);
    edit(&mut h, "edit.undo", json!({}));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Placed choices are proposals until review, and accepting a replacement preserves its transform.
#[test]
fn placed_package_choice_waits_for_update_and_preserves_layout() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Update PCB");
    h.key_press(Key::Escape);
    h.run();
    place_first(&mut h, 10_000_000, 20_000_000);
    let original = h.state().session.inspect()["project"]["board"]["footprints"][0].clone();
    let component = h.state().session.project().unwrap().components().next().unwrap().id();
    h.state_mut().begin_footprint_assignment(Some(component));
    h.run();
    click(&mut h, "basic-components / Resistor 0603 (1608 metric) · r4");
    click(&mut h, "Terminal 1");
    click(&mut h, "Pad 2");
    click(&mut h, "Terminal 2");
    click(&mut h, "Pad 1");
    click(&mut h, "Assign");
    h.get_by_label("Placed · update pending");
    assert_eq!(h.state().session.inspect()["project"]["board"]["footprints"][0]["definition"], original["definition"]);
    click(&mut h, "Close");
    open(&mut h);
    h.get_by_label("Replace footprint for R1: basic-components / resistor-0603 · r4.");
    h.get_by_label("Previous footprint for R1: basic-components / resistor-0402 · r4. Previous mapping: Pin 1 → pad 1, Pin 2 → pad 2.");
    click(&mut h, "Update PCB");
    assert!(matches!(h.state().dialog, Dialog::None));
    let replaced = h.state().session.inspect()["project"]["board"]["footprints"][0].clone();
    assert_eq!(replaced["definition"], "resistor-0603");
    for field in ["id", "component", "x_nm", "y_nm", "rotation"] {
        assert_eq!(replaced[field], original[field], "preserve {field}");
    }
    assert_eq!(replaced["mapping"], json!({"1":"2","2":"1"}));
}

/// Changing desired state from the post-Apply placement handoff requires another review.
#[test]
fn package_change_from_applied_result_requires_review_before_placement() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Update PCB");
    h.key_press(Key::Escape);
    h.run();
    let component = h.state().session.project().unwrap().components().next().unwrap().id();
    h.state_mut().begin_footprint_assignment(Some(component));
    h.run();
    click(&mut h, "basic-components / Resistor 0603 (1608 metric) · r4");
    click(&mut h, "Assign");
    assert!(h.get_by_label("Place on board").accesskit_node().is_disabled());
    click(&mut h, "Close");
    open(&mut h);
    click(&mut h, "Update PCB");
    let placement = h.state().views.active.board.placement.as_ref().unwrap();
    assert_eq!(placement.candidate.boards().next().unwrap().footprints().next().unwrap().definition, "resistor-0603");
    click(&mut h, "PCB canvas");
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().definition, "resistor-0603");
}

/// An outside edit invalidates the cursor group without placing stale geometry.
#[test]
fn outside_edit_while_placement_chooser_is_open_requires_review() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Update PCB");
    assert!(h.state().views.active.board.placement.is_some());
    change_value(&mut h);
    assert!(h.state().views.active.board.placement.is_none());
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 0);
    open(&mut h);
    h.get_by_label("PCB is up to date.");
    click(&mut h, "Place footprints");
    assert!(h.state().views.active.board.placement.is_some());
}

/// The live window action opens the same review and obeys existing modal and source-pane guards.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_update_action_and_browser_context_share_the_dialog() {
    let mut h = fixture(true);
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "app.update_pcb", &json!({"view":view.0})).unwrap();
    h.run();
    let token = h.state().session.cached_board_update().unwrap().plan_token;
    assert!(h.state_mut().run_window_command(&ctx, "app.update_pcb", &json!({"view":view.0})).is_err());
    assert_eq!(h.state().session.cached_board_update().unwrap().plan_token, token);
    click(&mut h, "Close");
    open_browser_row(&mut h, "Main", "Update PCB from schematic…");
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    click(&mut h, "Close");
    assert!(h.state_mut().run_window_command(&ctx, "app.update_pcb", &json!({"view":999999})).is_err());
}

/// An MCP-opened inactive pane owns the chooser and placement, regardless of the render slot.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inactive_update_source_keeps_assignment_and_placement_in_that_pane() {
    let mut h = fixture(true);
    let ctx = h.ctx.clone();
    let original = h.state().views.active.id();
    let source = h.state_mut().views.open_beside(original).unwrap();
    h.run();
    h.state_mut().views.bind(original);
    h.state_mut().run_window_command(&ctx, "app.update_pcb", &json!({"view":source.0})).unwrap();
    h.run();
    click(&mut h, "Update PCB");
    assert!(matches!(h.state().dialog, Dialog::None));
    let placement = h.state().views.view(source).unwrap().board.placement.as_ref().unwrap();
    assert_eq!(placement.source.view(), source);
    assert_eq!(h.state().views.view(source).unwrap().workspace, Workspace::Board);
    assert_eq!(h.state().views.view(original).unwrap().workspace, Workspace::Schematic);
    assert!(h.state().views.view(original).unwrap().board.placement.is_none());
}

/// Update one manufacturing flag while preserving the component's other authored properties.
fn exclude_last_component(h: &mut Harness<'_, PcbApp>) {
    let project = h.state().session.project().unwrap();
    let component = project.components().last().unwrap();
    let symbol = project.symbols().last().unwrap().id();
    let mut metadata = serde_json::to_value(&component.metadata).unwrap();
    metadata["attributes"]["exclude_from_board"] = json!(true);
    let params = json!({"symbol":symbol,"reference":component.reference(),"value":component.value(),"metadata":metadata});
    edit(h, pcbcraft_engine::SET_SYMBOL_FIELDS, params);
}

/// A nonphysical unassigned component does not block adopting existing placed physical parts.
#[test]
fn excluded_unassigned_component_does_not_block_legacy_adoption() {
    let mut h = fixture(true);
    place_first(&mut h, 0, 0);
    edit(&mut h, pcbcraft_engine::PLACE_RESISTOR, json!({"x_nm":20_000_000,"y_nm":0}));
    exclude_last_component(&mut h);
    open(&mut h);
    assert!(!h.get_by_label("Use current state").accesskit_node().is_disabled());
    assert!(h.query_by_label("Assign footprint to R2").is_none());
    click(&mut h, "Use current state");
    h.get_by_label("PCB is up to date.");
    click(&mut h, "Close");
}

/// Resolving an eligible legacy package returns directly to the explicit adoption step.
#[test]
fn legacy_assignment_handoff_preserves_initialization_phase() {
    let mut h = fixture(true);
    place_first(&mut h, 0, 0);
    edit(&mut h, pcbcraft_engine::PLACE_RESISTOR, json!({"x_nm":20_000_000,"y_nm":0}));
    open(&mut h);
    assert!(h.get_by_label("Use current state").accesskit_node().is_disabled());
    click(&mut h, "Assign footprint to R2");
    click(&mut h, "basic-components / Resistor 0402 (1005 metric) · r4");
    click(&mut h, "Match numbers");
    click(&mut h, "Assign");
    assert!(!h.get_by_label("Use current state").accesskit_node().is_disabled());
    assert!(h.query_by_label("Review again").is_none());
    click(&mut h, "Use current state");
    h.get_by_label("PCB is up to date.");
    h.get_by_label("1 footprint ready to place");
}

/// Initialization has a bounded, scrollable body even when many assignments are missing.
/// The last action remains reachable while the header and footer stay fixed.
#[test]
fn legacy_initialization_scrolls_to_last_missing_assignment() {
    let mut h = fixture(true);
    place_first(&mut h, 0, 0);
    for index in 0..20 {
        edit(&mut h, pcbcraft_engine::PLACE_RESISTOR, json!({"x_nm":index*10_000_000,"y_nm":10_000_000}));
    }
    open(&mut h);
    h.set_size(egui::vec2(620., 650.));
    h.run();
    let header = h.get_by_label("Close Update PCB from Schematic").rect();
    let footer = h.get_by_label("Use current state").rect();
    h.get_by_label("Assign footprint to R21").scroll_to_me();
    h.run();
    let last = h.get_by_label("Assign footprint to R21").rect();
    assert!(last.top() > header.bottom() && last.bottom() < footer.top());
    assert_eq!(
        h.get_by_label("Use current state").rect(),
        footer,
        "before header: {header:?}; after header: {:?}",
        h.get_by_label("Close Update PCB from Schematic").rect()
    );
    h.get_by_label("Assign footprint to R21").click();
    h.run();
    h.get_by_label("basic-components / Resistor 0402 (1005 metric) · r4");
}

/// Reviewed removal remains visible, but retained excluded package intent is never a placement handoff.
#[test]
fn excluded_accepted_unplaced_part_is_reviewed_then_removed_from_handoff() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Update PCB");
    h.key_press(Key::Escape);
    h.run();
    exclude_last_component(&mut h);
    open(&mut h);
    h.get_by_label("Total warnings: 1, errors: 0.");
    click(&mut h, "Update PCB");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().views.active.board.placement.is_none());
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 0);
}

/// Add another exact package through the shared command so a cursor group contains several parts.
fn add_assigned_resistor(h: &mut Harness<'_, PcbApp>) {
    edit(h, pcbcraft_engine::PLACE_RESISTOR, json!({"x_nm":20_000_000,"y_nm":0}));
    let project = h.state().session.project().unwrap();
    let component = project.components().last().unwrap().id();
    let package = project.footprint_catalog().into_iter().find(|p| p.id == "resistor-0603").unwrap();
    edit(
        h,
        ASSIGN_FOOTPRINT,
        json!({"component":component,"library":package.library,"definition":package.id,"definition_revision":package.revision,"mapping":{"1":"2","2":"1"}}),
    );
}

/// Group rotation follows the screen in either view, keeps orientation visible and uses the
/// predicted physical geometry in one history edit. Cancellation preserves accepted assignments.
#[test]
fn group_cursor_rotation_commit_undo_and_cancel_share_one_placement_transaction() {
    for flipped in [false, true] {
        let mut h = fixture(true);
        add_assigned_resistor(&mut h);
        open(&mut h);
        click(&mut h, "Update PCB");
        h.state_mut().views.active.camera.set_mirrored(flipped);
        h.run();
        let accepted = h.state().session.snapshot().unwrap();
        assert_eq!(h.state().views.active.board.placement.as_ref().unwrap().components.len(), 2);
        let canvas = h.get_by_label("PCB canvas").rect();
        let pointer = canvas.center();
        h.hover_at(pointer);
        h.run();
        h.key_press(Key::R);
        h.run();
        assert_eq!(h.state().views.active.board.placement.as_ref().unwrap().rotation, if flipped { 270 } else { 90 });
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::R);
        h.run();
        assert_eq!(h.state().views.active.board.placement.as_ref().unwrap().rotation, 0);
        h.key_press(Key::R);
        h.run();
        let group = h.state().views.active.board.placement.as_ref().unwrap();
        assert_eq!(group.rotation, if flipped { 270 } else { 90 });
        assert_eq!(h.query_by_label("Bottom view").is_some(), flipped);
        assert!(Arc::ptr_eq(&accepted, &h.state().session.snapshot().unwrap()));
        let params = json!({"components":group.components,"x_nm":group.position.x_nm,"y_nm":group.position.y_nm,"rotation":group.rotation,"expected_revision":group.source.revision});
        let predicted = h.state().session.preview_footprint_placement(&params).unwrap();
        click_canvas_at(&mut h, pointer);
        let poses = |project: &pcbcraft_engine::Project| {
            project.boards().next().unwrap().footprints().map(|f| (f.component, f.position, f.rotation)).collect::<Vec<_>>()
        };
        assert_eq!(poses(&predicted), poses(h.state().session.project().unwrap()));
        assert_eq!(poses(&predicted).len(), 2);
        edit(&mut h, "edit.undo", json!({}));
        super::assert_same_content(&accepted, &h.state().session.snapshot().unwrap());
        open(&mut h);
        click(&mut h, "Place footprints");
        let before_cancel = h.state().session.snapshot().unwrap();
        h.get_by_label("PCB canvas").click_secondary();
        h.run();
        assert!(h.state().views.active.board.placement.is_none());
        assert!(Arc::ptr_eq(&before_cancel, &h.state().session.snapshot().unwrap()));
    }
}

/// A long missing-package report at compact widths keeps its reason and primary action inside
/// the viewport; toggling report details cannot push the fixed footer away.
#[test]
fn compact_update_keeps_primary_action_visible_for_a_long_component_list() {
    let mut h = fixture(false);
    for index in 0..20 {
        edit(&mut h, pcbcraft_engine::PLACE_RESISTOR, json!({"x_nm":index*10_000_000,"y_nm":10_000_000}));
    }
    open(&mut h);
    h.set_size(egui::vec2(620., 760.));
    h.run();
    h.get_by_label("Total warnings: 0, errors: 21.");
    let action = h.get_by_label("Update PCB");
    assert!(action.accesskit_node().is_disabled());
    assert!(h.ctx.content_rect().contains_rect(action.rect()));
    let choose = h.get_by_label("Assign footprint to R1");
    assert!(h.ctx.content_rect().contains_rect(choose.rect()));
}

/// The new placement command is available through the live adapter and GUI Undo restores the
/// accepted unplaced state, while a modal still refuses document edits.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_group_placement_shares_ownership_and_gui_history() {
    let mut h = fixture(true);
    add_assigned_resistor(&mut h);
    open(&mut h);
    click(&mut h, "Update PCB");
    h.key_press(Key::Escape);
    h.run();
    let accepted = h.state().session.snapshot().unwrap();
    let components: Vec<_> = h.state().session.project().unwrap().components().map(|c| c.id()).collect();
    let request = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::PLACE_ASSIGNED_FOOTPRINTS,"params":{
        "components":components,"x_nm":0,"y_nm":0,"expected_revision":h.state().session.revision()}}});
    let ctx = h.ctx.clone();
    open(&mut h);
    assert!(pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).is_err());
    click(&mut h, "Close");
    pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).unwrap();
    h.run();
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    super::assert_same_content(&accepted, &h.state().session.snapshot().unwrap());
}

/// Named pins receive defaults too: exact library data supplies the starter LED polarity, and
/// an ordinary two-pad package uses matching numbers. Browsing changes no authored state.
#[test]
fn named_terminals_receive_editable_defaults_including_known_led_polarity() {
    let mut h = fixture(false);
    edit(&mut h, pcbcraft_engine::PLACE_SYMBOL, json!({"definition":"led","x_nm":20_000_000,"y_nm":0}));
    let component = h.state().session.project().unwrap().components().last().unwrap().id();
    let before = h.state().session.snapshot().unwrap();
    h.state_mut().begin_footprint_assignment(Some(component.clone()));
    h.run();
    click(&mut h, "basic-components / Capacitor 0402 (1005 metric) · r4");
    let anode = h.get_by_label("Terminal 1 · Anode").rect();
    let cathode = h.get_by_label("Terminal 2 · Cathode").rect();
    assert_eq!(anode.x_range(), cathode.x_range(), "Named terminals share one pad-selector column");
    assert!(anode.width() > 200., "The pad selector fills the preview's control column: {anode:?}");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Assign");
    let assignment = h.state().session.project().unwrap().footprint_assignments().into_iter().find(|a| a["component"] == component).unwrap();
    assert_eq!(assignment["mapping"], json!({"1":"1","2":"2"}));
    click(&mut h, "basic-components / LED 0402 (1005 metric) · r4");
    click(&mut h, "Assign");
    let assignment = h.state().session.project().unwrap().footprint_assignments().into_iter().find(|a| a["component"] == component).unwrap();
    assert_eq!(assignment["mapping"], json!({"1":"2","2":"1"}));
    // The suggested choices remain ordinary editable controls before another assignment.
    click(&mut h, "Terminal 1 · Anode");
    click(&mut h, "Pad 1");
    click(&mut h, "Terminal 2 · Cathode");
    click(&mut h, "Pad 2");
    click(&mut h, "Assign");
    click(&mut h, "Close");
    h.state_mut().begin_footprint_assignment(Some(component.clone()));
    h.run();
    click(&mut h, "basic-components / Capacitor 0402 (1005 metric) · r4");
    click(&mut h, "basic-components / LED 0402 (1005 metric) · r4");
    click(&mut h, "Assign");
    let assignment = h.state().session.project().unwrap().footprint_assignments().into_iter().find(|a| a["component"] == component).unwrap();
    assert_eq!(assignment["mapping"], json!({"1":"1","2":"2"}), "A saved custom mapping takes precedence over the library default");
}

/// A native handoff may wait for its target window to become focused; after its first active
/// frame, a later focus loss cancels normally. Neither transition creates physical geometry.
#[test]
fn pending_native_handoff_waits_for_focus_then_uses_normal_cancellation() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Update PCB");
    let accepted = h.state().session.snapshot().unwrap();
    // Simulate the captured handoff before its originating native host's first focused frame.
    h.state_mut().views.active.board.placement.as_mut().unwrap().handoff = true;
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    h.run();
    assert!(h.state().views.active.board.placement.as_ref().unwrap().handoff);
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(true));
    h.run();
    assert!(!h.state().views.active.board.placement.as_ref().unwrap().handoff);
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    h.run();
    assert!(h.state().views.active.board.placement.is_none());
    assert!(Arc::ptr_eq(&accepted, &h.state().session.snapshot().unwrap()));
}

/// Report filters and copying never change approval or authored state; a hidden error still
/// blocks Update PCB and remains counted in the report's unfiltered totals.
#[test]
fn report_filters_and_copy_leave_blockers_and_project_state_unchanged() {
    let mut h = fixture(false);
    let before = h.state().session.snapshot().unwrap();
    open(&mut h);
    let token = h.state().session.cached_board_update().unwrap().plan_token;
    click(&mut h, "All");
    h.get_by_label("No messages match the selected filters.");
    h.get_by_label("Total warnings: 0, errors: 1.");
    assert!(h.get_by_label("Update PCB").accesskit_node().is_disabled());
    h.get_by_label("Copy report").click();
    let mut copied = None;
    for _ in 0..3 {
        h.step();
        for command in &h.output().platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = Some(text.clone());
            }
        }
    }
    h.run();
    let copied = copied.unwrap();
    assert!(copied.contains("R1: no footprint assigned.") && copied.contains("Add footprint R1:"));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().session.cached_board_update().unwrap().plan_token, token);
    click(&mut h, "Errors (1)");
    h.get_by_label("R1: no footprint assigned.");
    assert!(h.query_by_label("Add footprint R1: Unassigned.").is_none());
    click(&mut h, "Assign footprint to R1");
    assert!(matches!(h.state().dialog, Dialog::Footprint(_)));
}

/// Disabling the placement option accepts electrical changes while keeping the report open;
/// the same accepted unplaced parts can later enter the normal cursor tool.
#[test]
fn placement_option_can_leave_an_updated_report_without_starting_a_cursor_group() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Place new footprints after updating");
    click(&mut h, "Update PCB");
    h.get_by_label("PCB updated.");
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    assert!(h.state().views.active.board.placement.is_none());
    assert!(h.get_by_label("Update PCB").accesskit_node().is_disabled());
    click(&mut h, "Place footprints");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().views.active.board.placement.is_some());
}

/// Native package selection keeps its parent review on cancellation and refreshes it after Apply.
#[test]
fn native_package_blocker_returns_to_board_update_review() {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({"create_board":true}));
    h.run();
    let documents: pcbcraft_engine::library::sources::Documents = [
        ("library.json", include_str!("../../../../examples/native-parts/library.json")),
        ("symbols/resistor.json", include_str!("../../../../examples/native-parts/symbols/resistor.json")),
        ("footprints/resistor.json", include_str!("../../../../examples/native-parts/footprints/resistor-0603.json")),
        ("categories/resistor.json", include_str!("../../../../examples/native-parts/categories/resistor.json")),
        ("parts/resistor.json", include_str!("../../../../examples/native-parts/parts/resistor.json")),
    ]
    .into_iter()
    .map(|(path, text)| (path.into(), text.into()))
    .collect();
    h.state_mut().session.execute("library.load_source", &json!({"source":"native","documents":documents})).unwrap();
    let reference = pcbcraft_engine::library::parts::Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
    let resolved = h.state().session.libraries().resolve_part(&reference).unwrap();
    let digest = pcbcraft_engine::library::sources::part_digest(resolved);
    edit(
        &mut h,
        pcbcraft_engine::part_instances::PLACE_PART,
        json!({"library":"parts-demo","definition":"resistor","definition_revision":1,
        "definition_digest":digest,"package":null,"x_nm":0,"y_nm":0}),
    );
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    open(&mut h);
    click(&mut h, "Assign footprint to R1");
    assert!(matches!(h.state().dialog, Dialog::NativePart(_)));
    click(&mut h, "Cancel");
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Assign footprint to R1");
    h.get_by_role_and_label(egui::Role::ComboBox, "Package").click();
    h.run();
    click(&mut h, "0603");
    click(&mut h, "Apply");
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    assert!(!h.get_by_label("Update PCB").accesskit_node().is_disabled());
    click(&mut h, "Update PCB");
    click(&mut h, "PCB canvas");
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 1);
}

/// A mapping review identifies pads moved to or from NC in both old and proposed summaries.
#[test]
fn mapping_review_shows_current_and_previous_unconnected_pads() {
    let mut h = fixture(false);
    h.state_mut()
        .session
        .execute("library.load_source", &json!({"source":"multipad","documents":super::footprint_assignment::four_pad_source()}))
        .unwrap();
    let component = h.state().session.project().unwrap().components().next().unwrap().id();
    for (mapped, unconnected) in [("3", "4"), ("4", "3")] {
        edit(
            &mut h,
            ASSIGN_FOOTPRINT,
            json!({"component":component,"library":"parts-demo","definition":"resistor-0603","definition_revision":1,
            "mapping":{"1":["1",mapped],"2":"2"},"unconnected_pads":[unconnected]}),
        );
        if unconnected == "4" {
            place_first(&mut h, 0, 0);
            edit(&mut h, pcbcraft_engine::board::update::INITIALIZE_BASELINE, json!({}));
        }
    }
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    open(&mut h);
    h.get_by_label("R1 pin mapping: Pin 1 → pad 1, 4, Pin 2 → pad 2, unconnected pads 3.");
    h.get_by_label(
        "Previous footprint for R1: parts-demo / resistor-0603 · r1. Previous mapping: Pin 1 → pad 1, 3, Pin 2 → pad 2, unconnected pads 4.",
    );
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}
