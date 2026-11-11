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
pub mod board;
pub mod catalog;
pub mod controls;
pub mod cursor;
pub mod editor;
pub mod fonts;
pub mod grid;
pub mod home;
pub mod library_sources;
pub mod menu;
pub mod net_label;
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
    ribbon_icons::install();
    fonts::install(ctx);
}

/// Clear the toolkit's per-frame widget geometry before drawing the app and its detached views.
pub fn begin_frame() {
    solidcraft_ui_registry::clear_handles();
}
