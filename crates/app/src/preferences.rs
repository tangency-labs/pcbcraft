//! The person's settings. They persist through eframe's storage: a file on the desktop, local
//! storage in the browser.

use egui::Ui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Follow the desktop or browser.
    System,
    /// The family's default look.
    #[default]
    Dark,
    Light,
}

/// The interface scales on offer, in percent.
const SCALES: [u16; 5] = [75, 100, 125, 150, 200];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: ThemeChoice,
    pub scale_percent: u16,
}

impl Default for Preferences {
    fn default() -> Self {
        Self { theme: ThemeChoice::default(), scale_percent: 100 }
    }
}

impl Preferences {
    /// Read saved settings. Anything unreadable falls back to the defaults rather than stopping the
    /// app from starting, and a scale we no longer offer goes back to 100%.
    pub fn parse(text: &str) -> Self {
        let mut preferences: Self = serde_json::from_str(text).unwrap_or_else(|e| {
            log::warn!("preferences: ignoring what was saved: {e}");
            Self::default()
        });
        if !SCALES.contains(&preferences.scale_percent) {
            preferences.scale_percent = 100;
        }
        preferences
    }

    pub fn to_text(self) -> String {
        serde_json::to_string(&self).unwrap_or_default()
    }

    /// Whether to use the dark palette. With no answer from the system, dark, like SolidCraft.
    pub fn is_dark(self, system: Option<egui::Theme>) -> bool {
        match self.theme {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => system != Some(egui::Theme::Light),
        }
    }

    pub fn zoom(self) -> f32 {
        f32::from(self.scale_percent) / 100.0
    }

    pub(crate) fn show(&mut self, ui: &mut Ui) {
        egui::Grid::new("preferences_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Theme");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.theme, ThemeChoice::System, "System");
                ui.selectable_value(&mut self.theme, ThemeChoice::Dark, "Dark");
                ui.selectable_value(&mut self.theme, ThemeChoice::Light, "Light");
            });
            ui.end_row();
            ui.label("Interface scale");
            ui.horizontal(|ui| {
                for scale in SCALES {
                    ui.selectable_value(&mut self.scale_percent, scale, format!("{scale}%"));
                }
            });
            ui.end_row();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_settings_come_back() {
        let preferences = Preferences { theme: ThemeChoice::Light, scale_percent: 150 };
        assert_eq!(Preferences::parse(&preferences.to_text()), preferences);
    }

    #[test]
    fn unreadable_settings_fall_back_to_the_defaults() {
        assert_eq!(Preferences::parse("not json"), Preferences::default());
        assert_eq!(Preferences::parse("{\"theme\":\"Plaid\"}"), Preferences::default());
        assert_eq!(Preferences::parse("{\"scale_percent\":9000}"), Preferences::default());
        assert_eq!(Preferences::parse("{\"theme\":\"Dark\"}").theme, ThemeChoice::Dark);
    }

    #[test]
    fn the_system_theme_is_followed_only_when_asked() {
        let of = |theme| Preferences { theme, ..Preferences::default() };
        assert!(of(ThemeChoice::System).is_dark(None));
        assert!(!of(ThemeChoice::System).is_dark(Some(egui::Theme::Light)));
        assert!(of(ThemeChoice::Dark).is_dark(Some(egui::Theme::Light)));
        assert!(!of(ThemeChoice::Light).is_dark(Some(egui::Theme::Dark)));
    }
}
