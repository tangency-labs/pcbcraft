//! Unsaved work is asked about before the window closes or another project replaces it.
//!
//! eframe turns every way of closing the window into a close request on the root viewport, and
//! closes unless the frame answers with `CancelClose`. These tests make that request themselves
//! and read the answer from the frame's viewport commands. Saves go to a stand-in that the test
//! finishes, cancels or fails, in place of the native picker.

use egui::{ViewportCommand, ViewportEvent, ViewportId};
use pcbcraft_engine::PLACE_SYMBOL;

use super::*;
use crate::unsaved::Then;

/// Shared prompt actions stay independent at compact widths: disabled or omitted Save never
/// disables Don't Save or Cancel, and unowned Enter never approves an unsaved-work decision.
#[test]
fn shared_save_prompt_keeps_each_decision_available() {
    use pcbcraft_chrome::shell::{self, SaveDecision};
    for width in [1280., 420.] {
        for (offer_save, save_enabled) in [(true, true), (true, false), (false, false)] {
            let mut h = Harness::builder().with_size(egui::vec2(width, 650.)).with_context_setup(pcbcraft_chrome::install).build_ui_state(
                |ui, decision: &mut Option<SaveDecision>| {
                    if let Some(answer) =
                        shell::save_changes(ui.ctx(), "Save changes?", "This project has unsaved changes.", offer_save, save_enabled)
                    {
                        *decision = Some(answer);
                    }
                },
                None,
            );
            h.run();
            h.key_press(Key::Enter);
            h.run();
            assert!(h.state().is_none());
            if offer_save {
                h.get_by_label("Save").click();
                h.run();
                assert_eq!(*h.state() == Some(SaveDecision::Save), save_enabled);
            } else {
                assert!(h.query_by_label("Save").is_none());
            }
            *h.state_mut() = None;
            h.get_by_label("Don't Save").click();
            h.run();
            assert!(*h.state() == Some(SaveDecision::Discard));
            *h.state_mut() = None;
            h.get_by_label("Cancel").click();
            h.run();
            assert!(*h.state() == Some(SaveDecision::Cancel));
            *h.state_mut() = None;
            h.get_by_label("Close Save changes?").click();
            h.run();
            assert!(*h.state() == Some(SaveDecision::Cancel));
            *h.state_mut() = None;
            h.key_press(Key::Escape);
            h.run();
            assert!(*h.state() == Some(SaveDecision::Cancel));
        }
    }
}

/// Enter activates the focused action while an unowned Enter leaves the prompt open.
#[test]
fn shared_save_prompt_supports_focused_keyboard_actions() {
    use pcbcraft_chrome::shell::{self, SaveDecision};
    for (label, expected) in [("Save", SaveDecision::Save), ("Don't Save", SaveDecision::Discard), ("Cancel", SaveDecision::Cancel)] {
        let mut h = Harness::builder().with_size(egui::vec2(600., 650.)).with_context_setup(pcbcraft_chrome::install).build_ui_state(
            |ui, decision: &mut Option<SaveDecision>| {
                if let Some(answer) = shell::save_changes(ui.ctx(), "Save changes?", "Unsaved work", true, true) {
                    *decision = Some(answer);
                }
            },
            None,
        );
        h.run();
        h.get_by_label(label).focus();
        h.run();
        assert!(h.get_by_label(label).is_focused());
        h.key_press(Key::Enter);
        h.run();
        assert!(*h.state() == Some(expected), "{label} must accept Enter");
    }
}

/// Whether the last frame sent the root viewport a command matching `command`.
fn sent(h: &Harness<'_, PcbApp>, command: impl Fn(&ViewportCommand) -> bool) -> bool {
    h.output().viewport_output.get(&ViewportId::ROOT).is_some_and(|output| output.commands.iter().any(command))
}

/// Run a few frames. A running file job keeps repainting, so the harness can't wait for idle.
fn settle(h: &mut Harness<'_, PcbApp>) {
    h.run_steps(3);
}

/// Click a control and run the frames after it, which a running file job keeps repainting.
fn press(h: &mut Harness<'_, PcbApp>, label: &str) {
    h.get_by_label(label).click();
    settle(h);
}

/// Ask the window to close, as eframe does for the OS close button, Quit and Ctrl+Q, and run that
/// frame. Returns whether the window was let close, then settles the frames that follow.
fn request_close(h: &mut Harness<'_, PcbApp>) -> bool {
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
    h.step();
    let closes = !sent(h, |command| matches!(command, ViewportCommand::CancelClose));
    settle(h);
    closes
}

/// Press a shortcut in the next frame and report whether that frame asked the window to close.
/// kittest's queued key presses run several frames in one step and keep only the last frame's
/// output, so the press goes straight into the frame's input instead.
fn shortcut_closes(h: &mut Harness<'_, PcbApp>, modifiers: Modifiers, key: Key) -> bool {
    let events = &mut h.input_mut().events;
    events.push(egui::Event::ModifiersChanged(modifiers));
    events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.step();
    let closes = sent(h, |command| matches!(command, ViewportCommand::Close));
    h.input_mut().events.push(egui::Event::ModifiersChanged(Modifiers::NONE));
    settle(h);
    closes
}

/// Click a control and report whether that frame asked the window to close.
fn click_closes(h: &mut Harness<'_, PcbApp>, label: &str) -> bool {
    h.get_by_label(label).click();
    h.step();
    let closes = sent(h, |command| matches!(command, ViewportCommand::Close));
    settle(h);
    closes
}

/// Run the frame that receives a finished file job and report whether it closed the window.
fn finish_closes(h: &mut Harness<'_, PcbApp>) -> bool {
    h.step();
    let closes = sent(h, |command| matches!(command, ViewportCommand::Close));
    settle(h);
    closes
}

/// A project with one placed resistor and its first save going to the stand-in.
fn unsaved_project() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.state_mut().files.use_stand_in();
    new_project(&mut h);
    let revision = h.state().session().revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": revision}));
    h.run();
    h
}

/// Without unsaved work the window closes at once. With it, the close is held and the question
/// names the project; Cancel keeps everything and asks again next time, and Don't Save closes
/// without writing anything.
#[test]
fn closing_asks_only_when_something_would_be_lost() {
    let mut h = harness();
    assert!(request_close(&mut h), "no project, nothing to lose");

    let mut h = unsaved_project();
    let before = h.state().session().inspect();
    assert!(!request_close(&mut h));
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(Then::Close)));
    h.get_by_label("Save changes to Untitled?");
    h.get_by_label("Untitled has unsaved changes. If you don't save, they are lost.");
    assert!(!click_closes(&mut h, "Cancel"));
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session().inspect(), before);
    assert!(!request_close(&mut h), "cancelling doesn't approve the next close");

    assert!(click_closes(&mut h, "Don't Save"));
    assert!(!h.state().files.stand_in_waiting(), "Don't Save writes nothing");
    assert!(request_close(&mut h), "the close that follows goes through");
}

/// Save closes the window only once the save succeeds. A dismissed picker, a failed save or an
/// edit made while the save ran leave the window open with the work still unsaved.
#[test]
fn save_then_close_waits_for_a_successful_save() {
    let dir = tempfile::tempdir().unwrap();

    let mut h = unsaved_project();
    assert!(!request_close(&mut h));
    assert!(!click_closes(&mut h, "Save"));
    assert!(h.state().files.stand_in_waiting());
    h.state_mut().files.finish_stand_in(dir.path().join("board"));
    assert!(finish_closes(&mut h));
    assert_eq!(h.state().session().project().unwrap().name, "board", "the first save named it");
    assert!(!h.state().files.dirty(h.state().session()));
    h.state_mut().close_allowed = false;
    assert!(request_close(&mut h), "a saved project closes without asking");

    for error in [None, Some("Disk full")] {
        let mut h = unsaved_project();
        assert!(!request_close(&mut h));
        press(&mut h, "Save");
        h.state_mut().files.end_stand_in(error);
        assert!(!finish_closes(&mut h));
        assert!(h.state().files.dirty(h.state().session()));
        assert_eq!(h.state().session().project().unwrap().name, "Untitled");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(h.state().message.as_deref(), error);
        assert!(!request_close(&mut h), "still unsaved, so it asks again");
    }

    let mut h = unsaved_project();
    assert!(!request_close(&mut h));
    press(&mut h, "Save");
    let revision = h.state().session().revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 25_400_000, "y_nm": 0, "expected_revision": revision}));
    h.state_mut().files.finish_stand_in(dir.path().join("edited"));
    assert!(!finish_closes(&mut h), "the edit made during the save is still unsaved");
    assert_eq!(h.state().file_notice.as_deref(), Some("Earlier version saved. Current edits are unsaved."));
    assert_eq!(h.state().session().project().unwrap().symbols().len(), 2);
}

/// A close requested while a save runs waits for it: a save that leaves nothing unsaved lets the
/// window close, and one that leaves edits behind asks about them.
#[test]
fn closing_during_a_save_decides_when_it_finishes() {
    let dir = tempfile::tempdir().unwrap();
    for edit in [false, true] {
        let mut h = unsaved_project();
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
        settle(&mut h);
        assert!(h.state().files.busy());
        assert!(!request_close(&mut h));
        assert!(matches!(h.state().dialog, Dialog::None), "nothing to ask until the save ends");
        assert_eq!(h.state().file_notice.as_deref(), Some("PCBCraft will close when the file operation finishes"));
        if edit {
            let revision = h.state().session().revision();
            h.state_mut().run(PLACE_SYMBOL, &json!({"definition": "led", "x_nm": 0, "y_nm": 25_400_000, "expected_revision": revision}));
        }
        h.state_mut().files.finish_stand_in(dir.path().join(format!("saved-{edit}")));
        assert_eq!(finish_closes(&mut h), !edit);
        assert_eq!(matches!(h.state().dialog, Dialog::SaveChanges(Then::Close)), edit);
    }
}

/// Quit, Ctrl+Q and the integrated title bar's close control all ask the window to close, so
/// they reach the same question.
#[test]
fn every_close_control_reaches_the_question() {
    let mut h = unsaved_project();
    h.state_mut().integrated_titlebar = true;
    h.run();
    click(&mut h, "File");
    assert!(click_closes(&mut h, "Quit"));
    assert!(!request_close(&mut h));
    click(&mut h, "Cancel");
    assert!(shortcut_closes(&mut h, Modifiers::COMMAND, Key::Q));
    assert!(!request_close(&mut h));
    click(&mut h, "Cancel");
    assert!(click_closes(&mut h, "Close window"));
    assert!(!request_close(&mut h));
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(Then::Close)));
}

/// Open asks the same question as closing. Don't Save goes on to choose a location; Save opens
/// only after the save succeeds.
#[test]
fn open_asks_the_same_question() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = unsaved_project();
    h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(Then::Open)));
    click(&mut h, "Don't Save");
    assert!(matches!(h.state().dialog, Dialog::OpenLocation));
    click(&mut h, "Cancel");

    h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    h.run();
    press(&mut h, "Save");
    assert!(matches!(h.state().dialog, Dialog::None));
    h.state_mut().files.finish_stand_in(dir.path().join("before-open"));
    settle(&mut h);
    assert!(matches!(h.state().dialog, Dialog::OpenLocation));
}

/// New PCB carries its workspace choice through a successful Save; a cancelled or failed save
/// and edits made during a save keep the original project and Home available.
#[test]
fn home_new_pcb_waits_for_save_and_keeps_work_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    for outcome in ["saved", "cancelled", "failed", "edited"] {
        let mut h = unsaved_project();
        open_home(&mut h);
        click(&mut h, "New PCB");
        assert!(matches!(h.state().dialog, Dialog::SaveChanges(Then::NewBoard)));
        let original = h.state().session().snapshot().unwrap();
        // Home also has a Save row; choose the dialog's button to its right.
        h.query_all_by_label("Save").max_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap().click();
        settle(&mut h);
        assert!(h.state().files.stand_in_waiting());
        assert!(h.state().home);
        assert!(Arc::ptr_eq(&h.state().session().snapshot().unwrap(), &original));
        match outcome {
            "cancelled" => h.state_mut().files.end_stand_in(None),
            "failed" => h.state_mut().files.end_stand_in(Some("Disk full")),
            _ => {
                if outcome == "edited" {
                    let revision = h.state().session().revision();
                    h.state_mut().run(PLACE_SYMBOL, &json!({"definition": "led", "x_nm": 0, "y_nm": 25_400_000, "expected_revision": revision}));
                }
                h.state_mut().files.finish_stand_in(dir.path().join(outcome));
            }
        }
        settle(&mut h);
        let current = h.state().session().snapshot().unwrap();
        if outcome == "saved" {
            assert!(!h.state().home);
            assert!(!current.same_document(&original));
            assert_eq!(h.state().views.active.workspace, Workspace::Board);
            assert_eq!(h.state().views.active.board.target, Some(current.board_target().unwrap()));
            h.get_by_label("PCB canvas");
            h.state_mut().run("edit.undo", &Value::Null);
            assert!(h.state().session().snapshot().unwrap().same_document(&original));
            assert!(!h.state().files.dirty(h.state().session()));
        } else {
            assert!(h.state().home);
            assert!(current.same_document(&original));
            assert!(h.state().files.dirty(h.state().session()));
            assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
            assert_eq!(current.symbols().len(), if outcome == "edited" { 2 } else { 1 });
            assert_eq!(h.state().message.as_deref(), (outcome == "failed").then_some("Disk full"));
        }
    }
}

/// Don't Save while a save is running waits for it, so Open doesn't replace the running job and
/// drop its result, then carries on. New Project, which can be asked about mid-save, waits too.
#[test]
fn dont_save_waits_for_a_running_save() {
    let dir = tempfile::tempdir().unwrap();
    for then in [Then::Open, Then::New, Then::NewBoard] {
        let mut h = unsaved_project();
        let original = h.state().session().snapshot().unwrap();
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
        settle(&mut h);
        assert!(h.state().files.busy());
        h.state_mut().dialog = Dialog::SaveChanges(then.clone());
        settle(&mut h);
        press(&mut h, "Don't Save");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(Arc::ptr_eq(&h.state().session().snapshot().unwrap(), &original), "nothing replaced yet");
        assert_eq!(h.state().file_notice.as_deref(), Some("Continuing when the file operation finishes"));

        h.state_mut().files.finish_stand_in(dir.path().join(format!("{then:?}")));
        settle(&mut h);
        match then {
            Then::Open => {
                assert!(matches!(h.state().dialog, Dialog::OpenLocation));
                assert!(!h.state().files.dirty(h.state().session()), "the finished save was recorded");
            }
            _ => {
                assert!(!h.state().session().snapshot().unwrap().same_document(&original));
                h.state_mut().run("edit.undo", &Value::Null);
                assert!(!h.state().files.dirty(h.state().session()), "the saved project is clean behind the new one");
            }
        }
    }
}

/// `file.save` refuses an Untitled project, which needs a location picked in the editor. Once
/// saved there, it writes in place like File › Save without a picker, and clears the unsaved
/// state when it finishes.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_save_window_command_saves_in_place_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = unsaved_project();
    let ctx = h.ctx.clone();
    let refused = h.state_mut().run_window_command(&ctx, "file.save", &json!({})).unwrap_err();
    assert!(refused.contains("Choose Save in the editor"), "{refused}");
    assert!(!h.state().files.stand_in_waiting(), "no picker opened");

    press(&mut h, "Save Project");
    h.state_mut().files.finish_stand_in(dir.path().join("board"));
    settle(&mut h);
    let revision = h.state().session().revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 25_400_000, "y_nm": 0, "expected_revision": revision}));
    h.run();
    assert!(h.state().files.dirty(h.state().session()));

    h.state_mut().run_window_command(&ctx, "file.save", &json!({})).unwrap();
    assert!(h.state().files.busy());
    assert_eq!(window_commands::inspect_editor(h.state())["file_operation"], true);
    h.state_mut().files.finish_stand_in(dir.path().join("unused"));
    settle(&mut h);
    assert!(!h.state().files.dirty(h.state().session()));
    let reopened = pcbcraft_storage::open(&dir.path().join("board")).unwrap();
    assert_eq!(reopened.symbols().len(), 2);
}
