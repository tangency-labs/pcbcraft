//! PCBCraft's window chrome in the Craft family style: theme, fonts, menus, bars, dialogs.
//!
//! This crate stands in for the shared Craft UI crate that will be extracted from SolidCraft. It
//! knows nothing about PCBCraft's engine, and it uses SolidCraft's names (`Tokens`, `MenuItem`), so
//! moving over should mostly mean changing a dependency. Keep it that way: app code gets its colours
//! and sizes from [`Tokens`] and its menus and dialogs from here, not from its own egui styling.
//!
//! The theme values, font setup and menu rows are adapted from SolidCraft (see `ATTRIBUTION.md`).

pub mod fonts;
pub mod menu;
pub mod shell;
pub mod theme;
pub mod workspace;

pub use menu::MenuItem;
pub use theme::Tokens;
