//! The unsaved-changes question asked before closing the window, starting a new project or
//! opening another: **Save**, **Don't Save** or **Cancel**, worded and ordered as in SolidCraft.
//!
//! Native window Close dismisses just that window while another editor survives. The last
//! editor and explicit File → Quit / Ctrl+Q use one project-close decision. The browser keeps
//! its own leave warning on while the project has unsaved changes (see [`PcbApp::warn_before_leaving`]).

use egui::{ViewportCommand, ViewportId};

use super::{Dialog, IN_BROWSER, PcbApp, shell};

/// What the person was doing when the open project's unsaved changes had to be answered for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Then {
    Close,
    New,
    Open,
}

/// An action held back until the running file operation finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Waiting {
    /// Closing was requested while a file operation ran: decide once it ends, asking if anything
    /// is still unsaved.
    ToDecide(Then),
    /// The person answered Save: carry on only if that save leaves nothing unsaved.
    ForSave(Then),
    /// The person answered Don't Save while a file operation ran: carry on once it ends, so its
    /// result is still recorded, whatever is left unsaved.
    Discarding(Then),
}

/// Longest project name shown in the question before it is shortened with an ellipsis.
const SHOWN_NAME: usize = 40;

impl PcbApp {
    /// Carry out `then` at once when nothing unsaved would be lost, otherwise ask first.
    pub(crate) fn unless_unsaved(&mut self, then: Then, ctx: &egui::Context) {
        if self.files.dirty(&self.session) {
            self.dialog = Dialog::SaveChanges(then);
        } else {
            self.carry_on(then, ctx);
        }
    }

    /// Distinguish a native window dismissal from explicit Quit. Closing the original window
    /// hides its framework shell when editors survive; only the final window or Quit ends the app.
    pub(crate) fn handle_close_request(&mut self, ctx: &egui::Context) {
        if IN_BROWSER || self.close_allowed || !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if std::mem::take(&mut self.windows.quit_requested) {
            self.begin_project_close(ctx);
        } else {
            #[cfg(not(target_arch = "wasm32"))]
            self.close_editor_host(crate::editor_layout::HostId::ROOT, ctx);
        }
        if !self.close_allowed {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::CancelClose);
        }
    }

    /// Ask once before ending the project. Running file jobs retain their owner and captured
    /// decision; a repeated request cannot overwrite a pending Save or Don't Save answer.
    pub(super) fn begin_project_close(&mut self, ctx: &egui::Context) {
        self.views.cancel_gestures();
        if self.waiting.is_some() || matches!(self.dialog, Dialog::SaveChanges(Then::Close)) {
            return;
        }
        if self.files.busy() {
            self.waiting = Some(Waiting::ToDecide(Then::Close));
            self.file_notice = Some("PCBCraft will close when the file operation finishes".to_owned());
        } else if self.files.dirty(&self.session) {
            ctx.send_viewport_cmd_to(crate::editor_windows::viewport(self.views.main), ViewportCommand::Focus);
            self.dialog = Dialog::SaveChanges(Then::Close);
        } else {
            self.carry_on(Then::Close, ctx);
        }
    }

    /// Once the file operation an action waits on has finished, carry it out if nothing is left
    /// unsaved. A close that was waiting to decide asks now when there is. After a Save answer,
    /// unsaved work left behind (a cancelled or failed save, or edits made while it ran) keeps the
    /// project open; the save's own status explains why. After Don't Save, carry on regardless.
    pub(crate) fn finish_waiting(&mut self, ctx: &egui::Context) {
        let Some(waiting) = self.waiting else { return };
        if self.files.busy() {
            return;
        }
        self.waiting = None;
        let (then, ask) = match waiting {
            Waiting::ToDecide(then) => (then, true),
            Waiting::ForSave(then) => (then, false),
            Waiting::Discarding(then) => {
                self.carry_on(then, ctx);
                return;
            }
        };
        if !self.files.dirty(&self.session) {
            self.carry_on(then, ctx);
        } else if ask {
            self.dialog = Dialog::SaveChanges(then);
        }
    }

    /// Do what the question held back. Closing marks the window as allowed to close and asks again.
    fn carry_on(&mut self, then: Then, ctx: &egui::Context) {
        match then {
            Then::Close => {
                self.close_allowed = true;
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            }
            Then::New => self.start_new_project(),
            Then::Open => self.choose_open_location(ctx),
        }
    }

    /// Draw the question and act on the answer. Save starts the ordinary save, which asks for a
    /// destination for a project never saved, and `then` follows once it succeeds. Don't Save
    /// carries on without writing anything, after any running file operation ends so that its
    /// result isn't dropped (an Open would replace it). Cancel, Escape or a click outside keep working. The
    /// browser offers no Save here, since a download doesn't confirm the project was kept.
    pub(crate) fn save_changes_dialog(&mut self, ctx: &egui::Context, then: Then) {
        let name = self.session.project().map_or_else(|| "The project".to_owned(), |project| shown_name(&project.name));
        let (mut save, mut discard, mut cancel) = (false, false, false);
        let closing = shell::dialog(ctx, "save_changes", &format!("Save changes to {name}?"), |ui| {
            ui.label(format!("{name} has unsaved changes. If you don't save, they are lost."));
            ui.horizontal(|ui| {
                if !IN_BROWSER {
                    save = ui.add_enabled(!self.files.busy(), egui::Button::new("Save")).clicked();
                }
                discard = ui.button("Don't Save").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if save {
            self.save_project_file(None, ctx);
            if self.message.is_none() {
                self.waiting = Some(Waiting::ForSave(then));
            }
        } else if discard && self.files.busy() {
            self.waiting = Some(Waiting::Discarding(then));
            self.file_notice = Some("Continuing when the file operation finishes".to_owned());
        } else if discard {
            self.carry_on(then, ctx);
        } else if !(cancel || closing) {
            self.dialog = Dialog::SaveChanges(then);
        }
    }

    /// Keep the browser's leave-page warning on exactly while the project has unsaved changes.
    /// Downloads leave it on, because a download doesn't confirm the project was kept.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn warn_before_leaving(&mut self) {
        let unsaved = self.files.dirty(&self.session);
        if self.leave_warning != Some(unsaved) {
            self.leave_warning = Some(unsaved);
            super::project_files::warn_before_leaving(unsaved);
        }
    }
}

/// Shorten a long project name for the question, keeping whole characters.
fn shown_name(name: &str) -> String {
    if name.chars().count() <= SHOWN_NAME { name.to_owned() } else { format!("{}…", name.chars().take(SHOWN_NAME - 1).collect::<String>()) }
}

#[cfg(test)]
mod tests {
    /// Long names are cut to a fixed number of whole characters; short ones are shown as they are.
    #[test]
    fn long_names_are_shortened_for_the_question() {
        assert_eq!(super::shown_name("Untitled"), "Untitled");
        let long = "é".repeat(super::SHOWN_NAME + 5);
        let shown = super::shown_name(&long);
        assert_eq!(shown.chars().count(), super::SHOWN_NAME);
        assert!(shown.ends_with('…'));
    }
}
