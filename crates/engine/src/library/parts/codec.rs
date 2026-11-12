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

/// Reject duplicate authored map identities rather than allowing the last JSON entry to win.
pub(crate) fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
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
                if result.len() >= 128 || result.contains_key(&key) {
                    return Err(serde::de::Error::custom("Too many or duplicate native record identities."));
                }
                result.insert(key, map.next_value()?);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Visitor(std::marker::PhantomData))
}
