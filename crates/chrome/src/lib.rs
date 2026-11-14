//! PCBCraft's window chrome in the Craft family style: theme, fonts, menus, bars, dialogs.
//!
//! This crate adapts the pinned shared Craft UI crates to PCBcraft. The shared look and kit
//! supply fonts, interface weights and menu rows. The app gets its colours and sizes from
//! [`Tokens`] and its menus and dialogs from here. The app owns document and window state.
//!
//! The theme values, font setup and menu rows are adapted from SolidCraft (see `ATTRIBUTION.md`).

pub mod arrangements;
pub mod assistant;
pub mod authoring;
pub mod automation;
pub mod board;
pub mod canvas_text;
pub mod canvas_warning;
pub mod catalog;
pub mod controls;
pub mod cursor;
pub mod editor;
pub mod fonts;
pub mod grid;
pub mod home;
pub mod library_sources;
pub mod library_workspace;
pub mod menu;
pub mod net_label;
pub mod part_form;
pub mod pcb_update;
#[cfg(not(target_arch = "wasm32"))]
pub mod preview;
pub mod properties;
mod ribbon_icons;
pub mod selection;
pub mod selection_filters;
pub mod settings;
pub mod shell;
pub mod theme;
mod titlebar;
pub mod units;
pub mod workspace;

pub use menu::MenuItem;
pub use theme::Tokens;

pub mod board_setup;

/// Install the shared icon set and PCBcraft's interface fonts before drawing the application.
pub fn install(ctx: &egui::Context) {
    install_text();
    ribbon_icons::install();
    fonts::install(ctx);
}

/// Clear the toolkit's per-frame widget geometry before drawing the app and its detached views.
pub fn begin_frame() {
    solidcraft_ui_registry::clear_handles();
}

/// The kit resolves its generic labels through a host-installed catalog, including
/// text-edit context menus. Keep electronics/application copy owned by PCBcraft.
fn install_text() {
    static ENGLISH: solidcraft_ui_text::Catalog = solidcraft_ui_text::Catalog::new("en", "English", include_str!("../assets/shared-ui/en.json"));
    static CATALOGS: [&solidcraft_ui_text::Catalog; 1] = [&ENGLISH];
    solidcraft_ui_text::install(&CATALOGS);
}

#[cfg(test)]
mod text_tests {
    /// Shared context menus and disclosures must render text rather than message IDs.
    #[test]
    fn host_catalog_resolves_shared_control_labels() {
        super::install_text();
        for id in ["ui.collapse", "ui.expand", "ui.cut", "ui.copy", "ui.paste"] {
            assert_ne!(solidcraft_ui_text::tr(id), id);
        }
    }
}
