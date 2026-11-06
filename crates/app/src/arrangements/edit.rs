//! Transient forms capture project, arrangement and host identities before accepting input.

use super::{Action, Arrangements, valid_name};
use crate::{Dialog, PcbApp, editor_layout::HostId};
use pcbcraft_chrome::arrangements::{Form, Intent, Outcome};

#[derive(Debug)]
pub(crate) struct Edit {
    pub(super) host: HostId,
    project: String,
    opening: u64,
    active: u64,
    form: Form,
}

impl Arrangements {
    /// Reject ambiguous labels and invalid text without using a label as identity.
    pub(super) fn name_error(&self, name: &str, except: Option<u64>) -> Option<&'static str> {
        let name = name.trim();
        if !valid_name(name) {
            return Some("Use a name of 1–80 characters on one line.");
        }
        if self.tabs().iter().any(|(id, other)| Some(*id) != except && other.to_lowercase() == name.to_lowercase()) {
            return Some("An arrangement already uses that name.");
        }
        None
    }

    /// Suggest a distinct name; existing records may contain labels from earlier versions.
    fn suggested_name(&self, base: &str) -> String {
        let base: String = base.chars().take(70).collect();
        if self.name_error(&base, None).is_none() {
            return base;
        }
        (2..).map(|index| format!("{base} {index}")).find(|name| self.name_error(name, None).is_none()).unwrap_or_default()
    }
}

impl PcbApp {
    /// Open a form in the initiating window, or queue selection directly. Menus never hold
    /// editable text, and opening a form neither creates an arrangement nor changes its layout.
    pub(crate) fn arrangement_intent(&mut self, ctx: &egui::Context, intent: Intent) {
        if self.home || self.files.busy() || self.project_action_pending() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        if let Intent::Select(id) = intent {
            self.queue_arrangement(ctx, Action::Select(id));
            return;
        }
        let Some(project) = &self.arrangements.project else { return };
        let tabs = self.arrangements.tabs();
        let active = self.arrangements.active();
        let Some((_, current)) = tabs.iter().find(|(id, _)| *id == active) else { return };
        if intent == Intent::Remove && tabs.len() <= 1 {
            return;
        }
        if matches!(intent, Intent::New | Intent::Duplicate) && tabs.len() >= 32 {
            self.message = Some("An arrangement collection can contain up to 32 arrangements.".into());
            return;
        }
        let name = match intent {
            Intent::New => self.arrangements.suggested_name("Arrangement"),
            Intent::Duplicate => self.arrangements.suggested_name(&format!("{current} copy")),
            _ => current.clone(),
        };
        self.dialog = Dialog::Arrangement(Edit {
            host: self.views.host,
            project: project.clone(),
            opening: self.arrangements.opening,
            active,
            form: Form::new(intent, name),
        });
        self.views.cancel_gestures();
        self.repaint_editors(ctx);
    }

    /// Keep arrangement forms in their initiating editor; other project dialogs use the main host.
    pub(crate) fn arrangement_dialog_host(&self) -> Option<HostId> {
        match &self.dialog {
            Dialog::Arrangement(edit) => Some(edit.host),
            _ => None,
        }
    }

    /// Validate captured identities before rendering or submitting. Cancelling discards only
    /// the draft; naming edits never restore layouts, close windows or touch project history.
    pub(crate) fn arrangement_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        if self.arrangements.project.as_ref() != Some(&edit.project)
            || self.arrangements.opening != edit.opening
            || self.arrangements.active() != edit.active
            || self.session.history_root_revision() != edit.opening
            || !self.views.hosts.contains_key(&edit.host)
        {
            return;
        }
        match edit.form.show(ctx) {
            Outcome::Cancel => {}
            Outcome::Open => self.dialog = Dialog::Arrangement(edit),
            Outcome::Submit => {
                let except = (edit.form.intent == Intent::Rename).then_some(edit.active);
                if edit.form.intent != Intent::Remove
                    && let Some(error) = self.arrangements.name_error(&edit.form.name, except)
                {
                    edit.form.reject(error);
                    self.dialog = Dialog::Arrangement(edit);
                    return;
                }
                let action = match edit.form.intent {
                    Intent::Rename => Action::Rename(edit.form.name),
                    Intent::Remove => Action::Remove,
                    _ => Action::Create { name: edit.form.name, single_view: edit.form.single_view },
                };
                self.queue_arrangement(ctx, action);
            }
        }
    }
}
