//! The real update dialog shares engine approval authority and leaves placement explicit.
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
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Update PCB from schematic").click();
    h.run();
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)), "dialog {:?}; message {:?}", h.state().dialog, h.state().message);
}

/// Opening and Cancel never author data. Explicit board creation is a separate undoable action.
#[test]
fn no_board_review_cancel_and_explicit_creation_have_distinct_history() {
    let mut h = harness();
    h.state_mut().start_new_project();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    open(&mut h);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Cancel");
    assert_eq!(h.state().session.revision(), revision);
    assert!(h.state().session.project().unwrap().board_target().is_none());

    open(&mut h);
    click(&mut h, "Create board");
    let created = h.state().session.snapshot().unwrap();
    assert!(created.board_target().is_some());
    assert_eq!(h.state().session.revision(), revision + 1);
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
    click(&mut h, "Cancel");
    assert!(Arc::ptr_eq(&created, &h.state().session.snapshot().unwrap()));
    edit(&mut h, "edit.undo", json!({}));
    assert!(h.state().session.project().unwrap().board_target().is_none());
}

/// Apply accepts the captured changes in one history step and leaves every new footprint unplaced.
#[test]
fn initial_update_apply_undo_reopen_and_noop_use_the_real_dialog() {
    let mut h = fixture(true);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    open(&mut h);
    h.get_by_label("Add R1");
    assert_eq!(h.state().session.revision(), revision);
    click(&mut h, "Apply update");
    h.get_by_label("Unplaced footprints (1)");
    assert_eq!(h.state().session.revision(), revision + 1);
    assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
    click(&mut h, "Close");
    let accepted = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    edit(&mut h, "edit.undo", json!({}));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    edit(&mut h, "edit.redo", json!({}));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), accepted);
    h.state_mut().session.open_snapshot(decode_documents(&accepted).unwrap());
    h.run();
    open(&mut h);
    h.get_by_label("PCB is up to date.");
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
    click(&mut h, "Cancel");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), accepted);
    open(&mut h);
    h.get_by_label("Unplaced footprints (1)");
    click(&mut h, "Place R1…");
    click(&mut h, "Place on board");
    h.get_by_label("No unplaced footprints.");
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 1);
}

/// A missing package blocks the complete update; the existing chooser returns to explicit re-review.
#[test]
fn package_blocker_chooser_returns_to_review_without_partial_apply() {
    let mut h = fixture(false);
    let before = h.state().session.snapshot().unwrap();
    open(&mut h);
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
    click(&mut h, "Apply update");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Assign footprint to R1");
    click(&mut h, "starter / Resistor 0402 (1005 metric) · r4");
    click(&mut h, "Assign");
    assert!(matches!(h.state().dialog, Dialog::Footprint(_)));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "An invalid mapping must not partly pin or assign a package");
    click(&mut h, "Terminal 1");
    click(&mut h, "Pad 1");
    click(&mut h, "Terminal 2");
    click(&mut h, "Pad 2");
    click(&mut h, "Assign");
    assert!(h.query_by_label("Place on board").is_none());
    click(&mut h, "Close");
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
    click(&mut h, "Review again");
    assert!(!h.get_by_label("Apply update").accesskit_node().is_disabled());
    click(&mut h, "Apply update");
    click(&mut h, "Place R1…");
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
    click(&mut h, "Place on board");
    h.get_by_label("No unplaced footprints.");
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().footprints().count(), 1);
    click(&mut h, "Close");
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
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
    click(&mut h, "Cancel");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.cached_board_update().unwrap().plan_token, newer);
    open(&mut h);
    change_value(&mut h);
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
    click(&mut h, "Review again");
    assert!(!h.get_by_label("Apply update").accesskit_node().is_disabled());
    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    h.run();
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
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
    click(&mut h, "Cancel");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    open(&mut h);
    click(&mut h, "Use current state");
    h.get_by_label("PCB is up to date.");
    let adopted = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert_ne!(adopted, before);
    click(&mut h, "Cancel");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), adopted);
    edit(&mut h, "edit.undo", json!({}));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Placed choices are proposals until review, and accepting a replacement preserves its transform.
#[test]
fn placed_package_choice_waits_for_update_and_preserves_layout() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Apply update");
    click(&mut h, "Close");
    place_first(&mut h, 10_000_000, 20_000_000);
    let original = h.state().session.inspect()["project"]["board"]["footprints"][0].clone();
    let component = h.state().session.project().unwrap().components().next().unwrap().id();
    h.state_mut().begin_footprint_assignment(Some(component));
    h.run();
    click(&mut h, "starter / Resistor 0603 (1608 metric) · r4");
    click(&mut h, "Terminal 1");
    click(&mut h, "Pad 2");
    click(&mut h, "Terminal 2");
    click(&mut h, "Pad 1");
    click(&mut h, "Assign");
    h.get_by_label("Placed · update pending");
    assert_eq!(h.state().session.inspect()["project"]["board"]["footprints"][0]["definition"], original["definition"]);
    click(&mut h, "Close");
    open(&mut h);
    click(&mut h, "Replace footprint R1");
    h.get_by_label("Previous footprint: starter / resistor-0402 · r4");
    h.get_by_label("Proposed footprint: starter / resistor-0603 · r4");
    click(&mut h, "Apply update");
    click(&mut h, "Close");
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
    click(&mut h, "Apply update");
    click(&mut h, "Place R1…");
    click(&mut h, "starter / Resistor 0603 (1608 metric) · r4");
    click(&mut h, "Terminal 1");
    click(&mut h, "Pad 1");
    click(&mut h, "Terminal 2");
    click(&mut h, "Pad 2");
    click(&mut h, "Assign");
    assert!(h.query_by_label("Place on board").is_none());
    click(&mut h, "Close");
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
    assert!(h.query_by_label("Place R1…").is_none());
    click(&mut h, "Review again");
    h.get_by_label("Replace footprint R1");
    click(&mut h, "Apply update");
    click(&mut h, "Place R1…");
    click(&mut h, "Place on board");
    h.get_by_label("No unplaced footprints.");
}

/// A chooser invalidated by an outside edit cannot refresh an Applied result into placement authority.
#[test]
fn outside_edit_while_placement_chooser_is_open_requires_review() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Apply update");
    click(&mut h, "Place R1…");
    change_value(&mut h);
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    assert!(h.get_by_label("Apply update").accesskit_node().is_disabled());
    assert!(h.query_by_label("Place R1…").is_none());
    click(&mut h, "Review again");
    // Values do not change the physical projection, so the refreshed no-op can resume placement.
    h.get_by_label("PCB is up to date.");
    h.get_by_label("Place R1…");
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
    click(&mut h, "Cancel");
    open_browser_row(&mut h, "Main", "Update PCB from schematic…");
    assert!(matches!(h.state().dialog, Dialog::BoardUpdate(_)));
    click(&mut h, "Cancel");
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
    click(&mut h, "Apply update");
    click(&mut h, "Place R1…");
    let Dialog::Footprint(chooser) = &h.state().dialog else { panic!("Expected placement chooser") };
    assert_eq!(chooser.source.view(), source);
    assert_eq!(h.state().views.view(source).unwrap().workspace, Workspace::Board);
    assert_eq!(h.state().views.view(original).unwrap().workspace, Workspace::Schematic);
    click(&mut h, "Place on board");
    let Dialog::BoardUpdate(review) = &h.state().dialog else { panic!("Expected update result") };
    assert_eq!(review.source.view(), source);
    click(&mut h, "Close");
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
    click(&mut h, "Cancel");
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
    click(&mut h, "starter / Resistor 0402 (1005 metric) · r4");
    click(&mut h, "Match numbers");
    click(&mut h, "Assign");
    click(&mut h, "Close");
    assert!(!h.get_by_label("Use current state").accesskit_node().is_disabled());
    assert!(h.query_by_label("Review again").is_none());
    click(&mut h, "Use current state");
    h.get_by_label("PCB is up to date.");
    h.get_by_label("Unplaced footprints (1)");
}

/// Reviewed removal remains visible, but retained excluded package intent is never a placement handoff.
#[test]
fn excluded_accepted_unplaced_part_is_reviewed_then_removed_from_handoff() {
    let mut h = fixture(true);
    open(&mut h);
    click(&mut h, "Apply update");
    click(&mut h, "Close");
    exclude_last_component(&mut h);
    open(&mut h);
    h.get_by_label("Remove R1");
    click(&mut h, "Apply update");
    h.get_by_label("No unplaced footprints.");
    assert!(h.query_by_label("Place R1…").is_none());
    click(&mut h, "Close");
}
