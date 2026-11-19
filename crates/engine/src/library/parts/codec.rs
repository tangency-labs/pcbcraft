//! Shared bounded native-record envelopes and duplicate-key rejection.
use super::Reference;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// Validate a shared native header before any semantic reference resolution.
pub(super) fn header(format: &str, expected: &str, reference: &Reference, name: &str, license: &str) -> Result<(), String> {
    if format != expected {
        return Err(format!("Unsupported native format {format:?}; expected {expected}."));
    }
    reference.validate()?;
    crate::library::check_text("name", name, 64)?;
    crate::library::check_text("license", license, 128)
}

/// All native record parsers share source-loader bounds even when called outside a loaded source.
pub(super) fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    if text.len() > crate::library::sources::MAX_FILE_BYTES {
        return Err("Native library record exceeds 1 MiB.".into());
    }
    serde_json::from_str(text).map_err(|error| error.to_string())
}

/// Stable pretty JSON with a final newline; validation is performed by the record's encoder.
pub(super) fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map(|text| text + "\n").map_err(|error| error.to_string())
}

/// Read at most 128 authored map identities, rejecting a duplicate rather than letting the last
/// JSON entry win.
pub(crate) fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    bounded_map(deserializer, 128, "A native record map")
}

/// [`unique_map`] for a package's terminal-to-pad map, which holds one entry per symbol terminal.
pub(crate) fn terminal_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    bounded_map(deserializer, crate::library::MAX_TERMINALS, "A package's `mapping`")
}

/// Read at most `limit` distinct identities; a repeated key or one more entry fails the record.
fn bounded_map<'de, D, T>(deserializer: D, limit: usize, field: &'static str) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(usize, &'static str, std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = BTreeMap<String, T>;
        /// Describe identity-keyed native maps in malformed-input diagnostics.
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map of distinct native identities")
        }
        /// Bound map allocation and reject repeated keys before adopting a record.
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if result.contains_key(&key) {
                    return Err(serde::de::Error::custom("A map has duplicate native record identities."));
                }
                if result.len() >= self.0 {
                    return Err(serde::de::Error::custom(format!("{} holds at most {} identities; read {}.", self.1, self.0, result.len() + 1)));
                }
                result.insert(key, map.next_value()?);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Visitor(limit, field, std::marker::PhantomData))
}
