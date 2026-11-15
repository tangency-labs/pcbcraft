//! Native library record contracts: parts, categories, values, packages and manufacturer parts.
mod category;
mod codec;
pub(crate) use codec::unique_map;
mod definition;
mod manufacturer;
pub(crate) mod mapping;
pub use mapping::PadIds;
mod package;
mod quantity;
mod reference;
mod requirement;
pub use manufacturer::{MANUFACTURER_PART_FORMAT, ManufacturerPart};
pub use requirement::{Requirement, requirements_schema};
mod value;
pub use category::{Category, CategoryBinding, Field, FieldType, field_id};
pub use definition::PartDefinition;
pub use package::Package;
pub use quantity::{Quantity, decimal, display_quantity};
pub use reference::Reference;
pub use value::Value;
/// Published reusable part record format.
pub const PART_FORMAT: &str = "pcbcraft-part/2";
/// Retained electrical-only records preserve their original encoding.
pub const LEGACY_PART_FORMAT: &str = "pcbcraft-part/1";
/// Published typed category record format.
pub const CATEGORY_FORMAT: &str = "pcbcraft-category/1";
#[cfg(test)]
mod tests;
