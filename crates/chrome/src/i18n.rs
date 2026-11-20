//! Interface localization (`en` / `he`), BCP 47 / legacy `iw` tag normalization, and RTL helpers.
//!
//! `egui 0.37` and `solidcraft-ui-text` handle visual UAX #9 BiDi ordering inside
//! `Catalog::shown()`, so `assets/shared-ui/he.json` stores logical Hebrew text.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Preference value that follows the operating system or browser language.
pub const SYSTEM: &str = solidcraft_ui_text::SYSTEM;
/// English catalog identifier (fallback).
pub const ENGLISH: &str = "en";
/// Hebrew catalog identifier (`עברית`, right-to-left).
pub const HEBREW: &str = "he";

/// Saved interface language preference (`system`, `en`, or `he`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LanguageChoice {
    /// Follow the operating system or browser locale.
    #[default]
    System,
    /// Force English (`en`).
    English,
    /// Force Hebrew (`he`, normalized from legacy `iw` tags on load).
    Hebrew,
}

impl LanguageChoice {
    /// Return the stable serialized tag (`"system"`, `"en"`, or `"he"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => SYSTEM,
            Self::English => ENGLISH,
            Self::Hebrew => HEBREW,
        }
    }
}

impl Serialize for LanguageChoice {
    /// Serialize `self` as its stable lowercase preference tag.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LanguageChoice {
    /// Deserialize a string preference tag, normalizing BCP 47 / legacy `iw` tags and falling back
    /// to [`LanguageChoice::System`] for unrecognized strings while rejecting non-string JSON.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(parse_language_choice(&raw))
    }
}

/// Map a serialized language tag (`"system"`, `"en"`, `"he"`, `"iw-IL"`, etc.) to [`LanguageChoice`].
fn parse_language_choice(raw: &str) -> LanguageChoice {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case(SYSTEM) {
        return LanguageChoice::System;
    }
    let normalized = normalize_language_tag(trimmed);
    match normalized.split('-').next().unwrap_or_default() {
        ENGLISH => LanguageChoice::English,
        HEBREW => LanguageChoice::Hebrew,
        _ => LanguageChoice::System,
    }
}

/// Whether Hebrew glyphs can render in this build (always on desktop; on `wasm32` only when
/// `CRAFT_FONTS_DIR` supplied `NotoSansHebrew-Regular.ttf` at compile time).
pub const fn hebrew_font_available() -> bool {
    cfg!(any(not(target_arch = "wasm32"), has_hebrew_craft_font))
}

/// Strip POSIX encoding/modifier suffixes, normalize separators to `-`, lowercase, reject `x-*`
/// private-use tags, and map legacy ISO 639-1 `iw` primary subtags to `he`.
pub fn normalize_language_tag(tag: &str) -> String {
    let base = tag.trim().split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
    let mut parts = base.split('-').filter(|part| !part.is_empty());
    let Some(primary) = parts.next() else {
        return String::new();
    };
    if primary == "x" {
        return String::new();
    }
    let normalized_primary = if primary == "iw" { HEBREW } else { primary };
    let mut out = String::from(normalized_primary);
    for part in parts {
        out.push('-');
        out.push_str(part);
    }
    out
}

/// Whether `code` names a right-to-left language or ISO 15924 script (normalizing legacy `iw` to
/// `he`, ignoring private-use `x-*` tags, and inspecting the RFC 5646 script subtag after any
/// 3-letter `extlang` subtags).
pub fn is_rtl_code(code: &str) -> bool {
    let cleaned = code.trim().split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
    let mut parts = cleaned.split('-').filter(|part| !part.is_empty());
    let Some(primary) = parts.next() else {
        return false;
    };
    if primary == "x" {
        return false;
    }
    // Skip RFC 5646 extlang subtags (3 ASCII letters); stop at extension/private-use singletons
    // or any other subtag before inspecting the 4-letter ISO 15924 script subtag position.
    let after_extlang = parts.find(|part| !(part.len() == 3 && part.bytes().all(|b| b.is_ascii_alphabetic())));
    if let Some(script) = after_extlang.filter(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic())) {
        return matches!(script, "arab" | "hebr" | "nkoo" | "rohg" | "syrc" | "thaa" | "yezi");
    }
    matches!(primary, "ar" | "ckb" | "dv" | "fa" | "he" | "iw" | "ps" | "sd" | "syr" | "ug" | "ur" | "yi")
}

/// Resolve `choice` to an installed catalog identifier (`"en"` or `"he"`).
pub fn resolve_preference(choice: LanguageChoice) -> &'static str {
    match choice {
        LanguageChoice::English => ENGLISH,
        LanguageChoice::Hebrew => {
            if hebrew_font_available() {
                HEBREW
            } else {
                ENGLISH
            }
        }
        LanguageChoice::System => resolve_system_language(),
    }
}

/// Resolve the system locale to `"he"` when Hebrew fonts are available, falling back to POSIX
/// environment variables for legacy `iw` desktop locales that `sys_locale` skips.
fn resolve_system_language() -> &'static str {
    if cfg!(test) {
        return ENGLISH;
    }
    let detected = solidcraft_ui_text::system_language();
    if detected == HEBREW && hebrew_font_available() {
        return HEBREW;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if detected == ENGLISH && hebrew_font_available() {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(val) = std::env::var(var)
                && normalize_language_tag(&val).split('-').next() == Some(HEBREW)
            {
                return HEBREW;
            }
        }
    }
    ENGLISH
}

/// Whether the active thread locale is right-to-left.
pub fn is_rtl() -> bool {
    is_rtl_code(solidcraft_ui_text::language())
}

/// Install the catalogs and enter a scoped thread-local locale for `choice`.
pub fn scope(choice: LanguageChoice) -> solidcraft_ui_text::LanguageScope {
    crate::install_text();
    solidcraft_ui_text::scope(resolve_preference(choice))
}

/// Return selectable language options for Preferences, preserving an unavailable saved Hebrew
/// preference as a distinct entry so leaving Preferences unchanged does not overwrite it.
pub fn language_options(current: LanguageChoice) -> Vec<(LanguageChoice, String)> {
    crate::install_text();
    language_options_with(current, hebrew_font_available())
}

/// Build Preferences language options for `current` given whether Hebrew fonts are available.
fn language_options_with(current: LanguageChoice, hebrew_available: bool) -> Vec<(LanguageChoice, String)> {
    let mut options = vec![(LanguageChoice::System, "System".into()), (LanguageChoice::English, "English".into())];
    if hebrew_available {
        options.push((LanguageChoice::Hebrew, solidcraft_ui_text::Locale::for_language(HEBREW).display_name().into_owned()));
    } else if current == LanguageChoice::Hebrew {
        // ASCII label on font-less builds because Hebrew glyphs are absent from the embedded font.
        options.push((LanguageChoice::Hebrew, "Hebrew (unavailable in this build)".into()));
    }
    options
}

#[cfg(test)]
mod tests {
    use serde::de::{IntoDeserializer, value::Error as DeError};

    use super::*;

    fn placeholder_names(text: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = text;
        while let Some((_, after)) = rest.split_once('{') {
            let Some((name, tail)) = after.split_once('}') else {
                break;
            };
            names.push(name.to_owned());
            rest = tail;
        }
        names.sort();
        names
    }

    #[test]
    fn legacy_iw_normalization_and_rtl_detection() {
        assert_eq!(normalize_language_tag("iw"), "he");
        assert_eq!(normalize_language_tag("iw-IL"), "he-il");
        assert_eq!(normalize_language_tag("iw_IL.UTF-8"), "he-il");
        assert_eq!(normalize_language_tag("he_IL@euro"), "he-il");
        assert_eq!(normalize_language_tag("x-he"), "");

        for rtl in ["he", "he-IL", "he_IL.UTF-8", "iw", "iw-IL", "en-Hebr", "zh-Arab-CN", "he-u-nu-latn", "ckb", "ku-Arab-IQ"] {
            assert!(is_rtl_code(rtl), "{rtl} should be RTL");
        }
        for ltr in ["", "en", "en-US", "x-he", "x-Arab", "en-x-he", "en-x-hebr", "he-Latn", "ar-Latn", "ku", "ku-Latn-TR", "en-u-ca-hebrew"] {
            assert!(!is_rtl_code(ltr), "{ltr} should be LTR");
        }
    }

    #[test]
    fn hebrew_catalog_covers_all_english_keys_and_placeholders() {
        static EN_CAT: solidcraft_ui_text::Catalog = solidcraft_ui_text::Catalog::new("en", "English", include_str!("../assets/shared-ui/en.json"));
        static HE_CAT: solidcraft_ui_text::Catalog = solidcraft_ui_text::Catalog::new("he", "עברית", include_str!("../assets/shared-ui/he.json"));

        let en = EN_CAT.messages();
        let he = HE_CAT.messages();
        assert!(!en.is_empty());

        let missing: Vec<&String> = en.keys().filter(|k| !he.contains_key(k.as_str())).collect();
        assert!(missing.is_empty(), "missing Hebrew translations in shared-ui/he.json: {missing:?}");

        for (key, en_val) in en {
            let he_val = he.get(key).expect("checked above");
            assert!(!he_val.trim().is_empty(), "{key} has empty Hebrew translation");
            assert_eq!(placeholder_names(en_val), placeholder_names(he_val), "placeholder mismatch for {key}");
            if !matches!(key.as_str(), "X" | "Y" | "Z" | "color.hex_hint" | "ui.viewport_status_summary_more") {
                assert_ne!(en_val, he_val, "{key} was left untranslated in shared-ui/he.json");
            }
        }
    }

    #[test]
    fn scoped_hebrew_translation_and_rtl_state() {
        crate::install_text();
        assert!(!is_rtl());
        assert_eq!(resolve_preference(LanguageChoice::Hebrew), HEBREW);
        {
            let _guard = scope(LanguageChoice::Hebrew);
            assert!(is_rtl());
            assert_eq!(solidcraft_ui_text::language(), HEBREW);
            assert_ne!(solidcraft_ui_text::tr("ui.copy"), "Copy");
            assert_ne!(solidcraft_ui_text::tr("ui.collapse"), "Collapse");
            let formatted = solidcraft_ui_text::format("ui.details_show_all", &[("count", "12")]);
            assert!(formatted.contains("12"), "got {formatted:?}");
        }
        assert!(!is_rtl());
    }

    #[test]
    fn language_choice_deserialization_and_unavailable_options() {
        let de = |s: &str| LanguageChoice::deserialize(IntoDeserializer::<DeError>::into_deserializer(s));
        crate::install_text();
        assert_eq!(de("iw"), Ok(LanguageChoice::Hebrew));
        assert_eq!(de("iw-IL"), Ok(LanguageChoice::Hebrew));
        assert_eq!(de("he_IL.UTF-8"), Ok(LanguageChoice::Hebrew));
        assert_eq!(de("en-US"), Ok(LanguageChoice::English));
        assert_eq!(de("system"), Ok(LanguageChoice::System));
        assert_eq!(de("x-he"), Ok(LanguageChoice::System));
        assert_eq!(de("fr-FR"), Ok(LanguageChoice::System));

        let unavailable = language_options_with(LanguageChoice::Hebrew, false);
        assert_eq!(unavailable.len(), 3);
        assert_eq!(unavailable[0].0, LanguageChoice::System);
        assert_eq!(unavailable[1].0, LanguageChoice::English);
        assert_eq!(unavailable[2].0, LanguageChoice::Hebrew);
        assert_eq!(unavailable[2].1, "Hebrew (unavailable in this build)");

        let available = language_options_with(LanguageChoice::System, true);
        assert_eq!(available.len(), 3);
        assert_eq!(available[2].0, LanguageChoice::Hebrew);
        assert!(!available[2].1.is_empty());
    }
}
