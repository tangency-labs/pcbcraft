//! Command discovery for material records; semantic adoption still uses shared validation.
use super::electrical::{MAX_FREQUENCY_HZ, MAX_REPORTS};
use super::{MAX_LENGTH_NM, MAX_MATERIALS, MAX_PLIES_PER_GAP, MAX_TEMPERATURE_MILLI_C, MAX_TG_REPORTS, MIN_TEMPERATURE_MILLI_C};
use super::{insulation, thermal};
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
            "thermal_conductivity": thermal_conductivity(),
            "cte": cte(),
            "dielectric_strength": dielectric_strength(),
            "cti": cti(),
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

/// Report basis, shared by every material report kind.
fn basis() -> Value {
    json!({"enum":["typical","minimum","maximum","measured"]})
}

/// Temperatures share the Tg report range.
fn temperature() -> Value {
    json!({"type":"integer","minimum":MIN_TEMPERATURE_MILLI_C,"maximum":MAX_TEMPERATURE_MILLI_C})
}

/// Conductivity in mW/(m·K); an absent axis is not an isotropy claim.
fn thermal_conductivity() -> Value {
    json!({"type":"array","maxItems":thermal::MAX_REPORTS,"items":{
        "type":"object","additionalProperties":false,"required":["conductivity_mw_per_m_k","basis","source"],
        "properties":{
            "conductivity_mw_per_m_k":{"type":"integer","minimum":1,"maximum":thermal::MAX_CONDUCTIVITY_MW_PER_M_K},
            "axis":{"enum":["x","y","z"]},"temperature_milli_c":temperature(),"method":text(80),
            "basis":basis(),"source":source()
        }
    }})
}

/// Expansion in ppb/K; a temperature range names both ends.
fn cte() -> Value {
    json!({"type":"array","maxItems":thermal::MAX_REPORTS,"items":{
        "type":"object","additionalProperties":false,"required":["cte_ppb_per_k","basis","source"],
        "dependentRequired":{"from_milli_c":["to_milli_c"],"to_milli_c":["from_milli_c"]},
        "properties":{
            "cte_ppb_per_k":{"type":"integer","minimum":thermal::MIN_CTE_PPB_PER_K,"maximum":thermal::MAX_CTE_PPB_PER_K},
            "axis":{"enum":["x","y","z"]},"region":{"enum":["below_tg","above_tg"]},
            "from_milli_c":temperature(),"to_milli_c":temperature(),"method":text(80),
            "basis":basis(),"source":source()
        }
    }})
}

/// Breakdown voltage at a required specimen thickness.
fn dielectric_strength() -> Value {
    json!({"type":"array","maxItems":insulation::MAX_REPORTS,"items":{
        "type":"object","additionalProperties":false,"required":["breakdown_v","specimen_thickness_nm","basis","source"],
        "properties":{
            "breakdown_v":{"type":"integer","minimum":1,"maximum":insulation::MAX_BREAKDOWN_V},
            "specimen_thickness_nm":{"type":"integer","minimum":1,"maximum":MAX_LENGTH_NM},
            "axis":{"enum":["x","y","z"]},"method":text(80),"basis":basis(),"source":source()
        }
    }})
}

/// CTI in volts and/or the IEC 60664-1 material group; both must agree when given.
fn cti() -> Value {
    json!({"type":"array","maxItems":insulation::MAX_REPORTS,"items":{
        "type":"object","additionalProperties":false,"required":["basis","source"],
        "anyOf":[{"required":["cti_v"]},{"required":["material_group"]}],
        "properties":{
            "cti_v":{"type":"integer","minimum":1,"maximum":insulation::MAX_CTI_V},
            "material_group":{"enum":["I","II","IIIa","IIIb"]},"method":text(80),"basis":basis(),"source":source()
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
