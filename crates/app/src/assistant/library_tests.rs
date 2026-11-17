//! The Assistant in library workspaces opened without a project.

use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

use super::State;
use crate::{PcbApp, tests::click};

/// A part document open in the Parts Library is something to work on even without a
/// project, as a symbol or footprint document is in its editor.
#[test]
fn part_document_without_a_project_enables_the_composer() -> Result<(), Box<dyn std::error::Error>> {
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), PcbApp::default());
    harness.run();
    let ctx = harness.ctx.clone();
    harness.state_mut().new_part_document(&ctx, None)?;
    harness.run();
    click(&mut harness, "Assistant");
    harness.state_mut().assistant.chat.state = State::Ready;
    harness.state_mut().assistant.draft = "Describe this part".into();
    harness.run();
    assert!(harness.query_by_label("Open or create a document to start.").is_none());
    assert!(!harness.get_by_role(egui::accesskit::Role::MultilineTextInput).accesskit_node().is_disabled());
    assert!(!harness.get_by_role_and_label(egui::accesskit::Role::Button, "Send").accesskit_node().is_disabled());
    Ok(())
}
