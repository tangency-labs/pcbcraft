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
mod supply_range;
mod value;
pub use category::{Category, CategoryBinding, Field, FieldType, field_id};
pub use definition::{PartDefinition, PartLink};
pub use package::Package;
pub use quantity::{Quantity, decimal, display_quantity};
pub use reference::Reference;
pub use supply_range::SupplyRange;
pub use value::Value;
/// Historical reusable part format with optional drawings.
pub const PART_FORMAT: &str = "pcbcraft-part/2";
/// Native part envelope reserved for records with an authored terminal supply range.
pub const RATED_PART_FORMAT: &str = "pcbcraft-part/4";
/// Native part envelope for records carrying human-facing search metadata, with or without ratings.
pub const METADATA_PART_FORMAT: &str = "pcbcraft-part/5";
/// Current native format for newly authored parts, with optional ratings and metadata.
pub const CURRENT_PART_FORMAT: &str = "pcbcraft-part/6";
/// Maximum authored supply ranges per part, shared by draft editing, native decoding and the part/4 schema.
pub const MAX_TERMINAL_RATINGS: usize = 128;
/// Retained electrical-only records preserve their original encoding.
pub const LEGACY_PART_FORMAT: &str = "pcbcraft-part/1";
/// Longest authored part description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 512;
/// Most search keywords on one part.
pub const MAX_KEYWORDS: usize = 16;
/// Longest stored keyword, in characters.
pub const MAX_KEYWORD_CHARS: usize = 64;
/// Most reference links on one part.
pub const MAX_LINKS: usize = 8;
/// Longest link label, in characters.
pub const MAX_LINK_LABEL_CHARS: usize = 64;
/// Longest link URL, in characters, shared with the manufacturer datasheet field.
pub const MAX_LINK_URL_CHARS: usize = 2048;
/// Published typed category record format.
pub const CATEGORY_FORMAT: &str = "pcbcraft-category/1";
#[cfg(test)]
mod tests;
