//! Preserve the former bundled identity for saved projects and explicit legacy commands.
use super::{Library, sources::Documents};
use serde_json::Value;
use std::sync::LazyLock;

pub(super) const SOURCE: &str = "bundled-starter";

/// Rebuild legacy addresses from our unchanged bundled geometry, without exposing a second
/// collection in browsing. Only library references and the manifest identity are translated.
pub(super) fn documents() -> Documents {
    let Some((_, documents)) = super::bundled_documents().find(|(id, _)| *id == "basic-components") else {
        return Documents::new();
    };
    documents
        .into_iter()
        .map(|(path, text)| {
            let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
                return (path, text);
            };
            translate(&mut value);
            if path == "library.json" {
                value["id"] = "starter".into();
                value["name"] = "Default".into();
            }
            (path, value.to_string())
        })
        .collect()
}

/// Translate library-address fields recursively, retaining item IDs and all content fields.
fn translate(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if fields.get("library").is_some_and(|id| id == "basic-components") {
                fields.insert("library".into(), "starter".into());
            }
            fields.values_mut().for_each(translate);
        }
        Value::Array(values) => values.iter_mut().for_each(translate),
        _ => {}
    }
}

/// Read historical pre-pinning project formats using their original library identity.
pub(crate) fn starter() -> &'static Library {
    static LEGACY: LazyLock<Library> = LazyLock::new(|| {
        let documents = documents();
        let symbols: Vec<_> = documents.iter().filter_map(|(path, text)| path.strip_prefix("symbols/").map(|name| (name, text.as_str()))).collect();
        Library::load(documents.get("library.json").map(String::as_str).unwrap_or_default(), &symbols)
    });
    &LEGACY
}

/// Compare bundled mapping defaults across the historical rename without rewriting saved records.
pub(crate) fn current_id(library: &str) -> &str {
    if library == "starter" { "basic-components" } else { library }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ASSIGN_FOOTPRINT, PLACE_SYMBOL, Session,
        library::sources,
        persistence::{decode_documents, encode_documents},
    };
    use serde_json::json;

    /// Canonical browsing does not duplicate old identities, while exact legacy selections,
    /// pinned definitions, mixed-address mappings and offline reopen retain their addresses.
    #[test]
    fn renamed_bundle_preserves_legacy_addresses_without_duplicate_browsing() {
        let mut session = Session::default();
        let catalog = session.libraries();
        let visible = catalog.inspect();
        assert_eq!(visible.as_array().unwrap().len(), 2);
        assert!(visible.as_array().unwrap().iter().any(|s| s["library"] == "basic-components" && s["name"] == "Basic components"));
        assert!(catalog.search("", None).iter().all(|(_, item)| item.key.library != "starter"));
        assert!(catalog.symbol("starter", "led", 2).is_some());
        assert!(catalog.footprint("starter", "led-0402", 4).is_some());
        session.apply("project.new", &json!({})).unwrap();
        for library in ["starter", "basic-components"] {
            let symbol = session.libraries().symbol(library, "led", 2).unwrap();
            session.apply(PLACE_SYMBOL, &json!({"library":library,"definition":"led","definition_revision":2,"definition_digest":sources::symbol_digest(&symbol).unwrap(),"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
            let project = session.snapshot().unwrap();
            let component = project.components().last().unwrap().id();
            for footprint_library in ["starter", "basic-components"] {
                let footprint = session.libraries().footprint(footprint_library, "led-0402", 4).unwrap();
                let mapping = project.default_footprint_mapping(&component, &footprint).unwrap();
                assert_eq!(serde_json::to_value(&mapping).unwrap(), json!({"1":"2","2":"1"}));
            }
            session.apply(ASSIGN_FOOTPRINT, &json!({"component":component,"library":library,"definition":"led-0402","definition_revision":4,"mapping":{"1":"2","2":"1"},"expected_revision":session.revision()})).unwrap();
        }
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        for library in ["starter", "basic-components"] {
            assert!(saved.contains_key(&format!("library/symbols/{library}/led.r2.json")));
            assert!(saved.contains_key(&format!("library/footprints/{library}/led-0402.r4.json")));
        }
        assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
        assert!(session.execute(sources::REMOVE, &json!({"source":SOURCE})).unwrap_err().to_string().contains("read-only"));
    }

    /// Legacy records still participate in immutable-revision conflict checks; hiding their
    /// registration must not permit a changed external copy to replace the trusted definition.
    #[test]
    fn old_identity_conflicts_are_not_hidden_or_aliased_to_new_items() {
        let mut session = Session::default();
        let mut docs = documents();
        let mut symbol: Value = serde_json::from_str(&docs["symbols/resistor.json"]).unwrap();
        symbol["name"] = json!("Changed copy");
        docs.insert("symbols/resistor.json".into(), symbol.to_string());
        session.execute(sources::LOAD, &json!({"source":"old-external","documents":docs})).unwrap();
        assert!(session.libraries().latest_symbol("starter", "resistor").is_none());
        assert!(session.libraries().symbol("starter", "resistor", 3).is_none());
        assert!(session.libraries().latest_symbol("basic-components", "resistor").is_some());
        assert!(
            session
                .libraries()
                .inspect()
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["source"] == "old-external" && !s["problems"].as_array().unwrap().is_empty())
        );
    }
}
