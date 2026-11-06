//! Command discovery for material records; semantic adoption still uses shared validation.
use super::electrical::{MAX_FREQUENCY_HZ, MAX_REPORTS};
use super::{MAX_LENGTH_NM, MAX_MATERIALS, MAX_PLIES_PER_GAP, MAX_TEMPERATURE_MILLI_C, MAX_TG_REPORTS, MIN_TEMPERATURE_MILLI_C};
use serde_json::{Value, json};

/// Require bounded text; whitespace and control characters are checked during adoption.
fn text(max: usize) -> Value {
    json!({"type":"string","minLength":1,"maxLength":max})
}

/// Address one exact revision of a project-local material.
fn reference() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["id","revision"],"properties":{
        "id":{"type":"string","format":"uuid"},"revision":{"type":"integer","minimum":1,"maximum":u32::MAX}
    }})
}

/// Keep source descriptions offline; a locator need not be a network URL.
fn source() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["document"],"properties":{
        "document":text(200),"revision":text(80),"locator":text(1024)
    }})
}

/// Publish supported property types and resource limits without inventing unknown values.
pub(crate) fn catalog() -> Value {
    json!({"type":"array","maxItems":MAX_MATERIALS,"items":{
        "type":"object","additionalProperties":false,"required":["id","revision","name"],"properties":{
            "id":{"type":"string","format":"uuid"},"revision":{"type":"integer","minimum":1,"maximum":u32::MAX},
            "name":text(80),"family":text(80),"manufacturer":text(80),"product":text(80),"source":source(),
            "dielectric": dielectric(),
            "tg":{"type":"array","maxItems":MAX_TG_REPORTS,"items":{
                "type":"object","additionalProperties":false,"required":["temperature_milli_c","method","basis","source"],"properties":{
                    "temperature_milli_c":{"type":"integer","minimum":MIN_TEMPERATURE_MILLI_C,"maximum":MAX_TEMPERATURE_MILLI_C},
                    "method":{"type":"string","enum":["dsc","tma","dma"]},
                    "basis":{"type":"string","enum":["typical","minimum","maximum","measured"]},
                    "test_standard":text(80),"source":source()
                }
            }}
        }
    }})
}

/// Reports preserve known conditions; missing conditions do not imply universal applicability.
fn dielectric() -> Value {
    json!({"type":"array","maxItems":MAX_REPORTS,"items":{
        "type":"object","additionalProperties":false,"required":["basis","source"],
        "anyOf":[{"required":["dk_millionths"]},{"required":["df_millionths"]}],
        "properties":{
            "dk_millionths":{"type":"integer","minimum":1_000_000,"maximum":1_000_000_000},
            "df_millionths":{"type":"integer","minimum":0,"maximum":1_000_000},
            "frequency_hz":{"type":"integer","minimum":1,"maximum":MAX_FREQUENCY_HZ},
            "method":text(80),"value_kind":{"enum":["test","design"]},
            "basis":{"enum":["typical","minimum","maximum","measured"]},"source":source(),
            "temperature_milli_c":{"type":"integer","minimum":MIN_TEMPERATURE_MILLI_C,"maximum":MAX_TEMPERATURE_MILLI_C},
            "axis":{"enum":["x","y","z"]},"glass_construction":text(80),
            "resin_content_ppm":{"type":"integer","minimum":1,"maximum":999_999},
            "specimen_thickness_nm":{"type":"integer","minimum":1,"maximum":MAX_LENGTH_NM}
        }
    }})
}

/// Describe a core or ordered prepreg group; nominal ply dimensions are independent of finished gap thickness.
pub(crate) fn construction() -> Value {
    json!({"oneOf":[
        {"type":"object","additionalProperties":false,"required":["kind"],"properties":{
            "kind":{"const":"core"},"material":reference(),"glass_construction":text(80)
        }},
        {"type":"object","additionalProperties":false,"required":["kind","plies"],"properties":{
            "kind":{"const":"prepreg"},"plies":{"type":"array","minItems":1,"maxItems":MAX_PLIES_PER_GAP,"items":{
                "type":"object","additionalProperties":false,"required":["id"],"properties":{
                    "id":{"type":"string","format":"uuid"},"material":reference(),"glass_style":text(80),
                    "resin_content_ppm":{"type":"integer","minimum":1,"maximum":999_999},
                    "nominal_thickness_nm":{"type":"integer","minimum":1,"maximum":MAX_LENGTH_NM}
                }
            }}
        }}
    ]})
}
