//! Profile import previews stay in the captured form; export owns an immutable draft snapshot.
use super::*;
use pcbcraft_engine::board::{setup::profile::SetupProfile, silkscreen::Settings};

/// File actions capture or stage a profile without applying the board draft.
#[derive(Clone, Copy)]
pub(super) enum ProfileAction {
    Import,
    Export,
}
impl Edit {
    /// Require every print dimension, naming blank or malformed fields without rounding untouched
    /// values. Preview, profile export and Apply share the same complete engine validation.
    pub(super) fn silk_style(&self) -> Result<Settings, String> {
        let values = ["text height", "stroke width", "pad clearance"]
            .into_iter()
            .zip(&self.silkscreen)
            .map(|(label, length)| {
                length.value(self.unit).map_err(|error| format!("Invalid {label}: {error}"))?.ok_or_else(|| format!("Enter a {label}."))
            })
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::from_value(json!({"text_height_nm":values[0],"stroke_width_nm":values[1],"pad_clearance_nm":values[2]}))
            .map_err(|e| e.to_string())
    }
    /// The profile page shows editable identity and a human-readable preview of its supported
    /// contents. File operations never mark the project saved or apply a pending draft.
    pub(super) fn profiles(&mut self, ui: &mut Ui, busy: bool) -> Option<ProfileAction> {
        form::heading(ui, "Setup profiles");
        ui.label(
            "Profiles share stackup, embedded materials and silkscreen settings. Import stages the settings for review; Apply updates the board.",
        );
        let mut action = None;
        form::card(ui, |ui| {
            form::text(ui, "Profile name", &mut self.profile_name);
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, pcbcraft_chrome::catalog::button("Import profile…", false)).clicked() {
                    action = Some(ProfileAction::Import);
                }
                if ui.add_enabled(!busy && self.material_edit.is_none(), pcbcraft_chrome::catalog::button("Export profile…", false)).clicked() {
                    action = Some(ProfileAction::Export);
                }
            });
        });
        properties::section(ui, "Current settings");
        form::card(ui, |ui| {
            form::field(ui, "Copper layers", |ui| {
                ui.label(self.candidate.setup.stackup().strata.len().div_ceil(2).to_string());
            });
            let total = self
                .candidate
                .value(self.unit)
                .ok()
                .and_then(|setup| setup.laminate_thickness_nm())
                .map(|n| units::label(n, self.unit))
                .unwrap_or_else(|| "Unspecified".into());
            form::field(ui, "Laminate thickness", |ui| {
                ui.label(total);
            });
            form::field(ui, "Materials", |ui| {
                ui.label(self.candidate.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "));
            });
            for (label, length) in ["Text height", "Stroke width", "Pad clearance"].into_iter().zip(&self.silkscreen) {
                form::field(ui, label, |ui| {
                    ui.label(&length.text);
                });
            }
        });
        if busy {
            ui.spinner();
        }
        if let Some(notice) = &self.profile_notice {
            ui.label(notice);
        }
        action
    }
}
impl PcbApp {
    /// A completion belongs only to the exact form ticket and captured view/revision. Closing,
    /// reopening or replacing the project discards stale imports without releasing running jobs.
    pub(super) fn poll_setup_profile(&mut self, edit: &mut Edit, valid: bool) {
        let Some((ticket, result)) = self.board_profile_files.poll() else { return };
        if edit.profile_ticket != Some(ticket) {
            return;
        }
        edit.profile_ticket = None;
        if !valid && matches!(&result, Ok(profile_io::Outcome::Imported(_))) {
            edit.error = Some("The project or view changed; the imported profile was not adopted.".into());
            return;
        }
        match result {
            Ok(profile_io::Outcome::Imported(profile)) => {
                match edit.candidate.value(edit.unit).and_then(|setup| setup.with_profile(&profile).map_err(|e| e.to_string())) {
                    Ok(setup) => {
                        edit.candidate = Candidate::new(setup, edit.unit);
                        edit.silkscreen =
                            [profile.silkscreen.text_height_nm, profile.silkscreen.stroke_width_nm, profile.silkscreen.pad_clearance_nm]
                                .map(|n| Length::new(Some(n), edit.unit));
                        edit.profile_name = profile.name.clone();
                        edit.locked.clear();
                        edit.material_edit = None;
                        edit.error = None;
                        edit.profile_notice = Some(format!("Imported {}.", profile.name));
                    }
                    Err(error) => edit.error = Some(error),
                }
            }
            Ok(profile_io::Outcome::Exported) => {
                edit.profile_notice = Some(if crate::IN_BROWSER { "Profile download started." } else { "Profile exported." }.into())
            }
            Ok(profile_io::Outcome::Cancelled) => {}
            Err(error) => edit.error = Some(error),
        }
    }
    /// Encode before starting export, capturing exactly this draft. Import/export file errors
    /// remain in the form; the shared project save lifecycle and undo history are untouched.
    pub(super) fn start_setup_profile(&mut self, ctx: &Context, edit: &mut Edit, action: ProfileAction) {
        let save = match action {
            ProfileAction::Import => Ok(None),
            ProfileAction::Export => edit
                .candidate
                .value(edit.unit)
                .and_then(|setup| SetupProfile::new(edit.profile_name.clone(), setup, edit.silk_style()?))
                .and_then(|profile| profile.encode())
                .map(|bytes| {
                    let name = edit.profile_name.chars().filter(|c| c.is_alphanumeric() || *c == '-').take(48).collect::<String>();
                    Some((format!("{}.pcsetup.json", if name.is_empty() { "board-setup" } else { &name }), bytes))
                }),
        };
        match save.and_then(|save| self.board_profile_files.start(ctx, save)) {
            Ok(ticket) => {
                edit.profile_ticket = Some(ticket);
                edit.profile_notice = None;
                edit.error = None;
            }
            Err(error) => edit.error = Some(error),
        }
    }
}
