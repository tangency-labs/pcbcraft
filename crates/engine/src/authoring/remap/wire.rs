//! Exact decimal strings for remap routing only; native authoring codecs remain unchanged.
use serde_json::{Value, json};
#[cfg(test)]
mod tests;

/// The independent numeric domains exposed by the remap commands.
#[derive(Clone, Copy, Debug)]
pub enum Scalar {
    Object,
    BaseRevision,
    Runtime,
    Token,
}
impl Scalar {
    /// Return inclusive native bounds without narrowing existing identities.
    fn bounds(self) -> (u64, u64) {
        match self {
            Self::BaseRevision => (1, u64::from(u32::MAX)),
            Self::Runtime => (0, u64::MAX),
            Self::Object | Self::Token => (1, u64::MAX),
        }
    }

    /// Refuse numeric JSON and noncanonical, overflowing or out-of-domain strings.
    pub fn parse(self, value: &Value) -> Result<u64, String> {
        let (min, max) = self.bounds();
        if let Some(text) = value.as_str()
            && !text.is_empty()
            && text.bytes().all(|byte| byte.is_ascii_digit())
            && (text == "0" || !text.starts_with('0'))
            && let Ok(number) = text.parse::<u64>()
            && (min..=max).contains(&number)
        {
            return Ok(number);
        }
        Err(format!("Expected a canonical decimal string from {min} through {max}."))
    }

    /// Publish the lexical bound exactly, including the last valid u32/u64 decimal string.
    /// Shorter positive strings fit. Same-length strings may equal the maximum or first differ below it.
    pub fn schema(self) -> Value {
        let (min, max) = self.bounds();
        let maximum = max.to_string();
        let mut alternatives = vec![format!("[1-9][0-9]{{0,{}}}", maximum.len() - 2)];
        if min == 0 {
            alternatives.push("0".into());
        }
        for (index, digit) in maximum.bytes().enumerate() {
            let lower = if index == 0 { b'1' } else { b'0' };
            if digit > lower {
                let lower = char::from(lower);
                let upper = char::from(digit - 1);
                alternatives.push(format!("{}[{lower}-{upper}][0-9]{{{}}}", &maximum[..index], maximum.len() - index - 1));
            }
        }
        alternatives.push(maximum.clone());
        json!({"type":"string","minLength":1,"maxLength":maximum.len(),"pattern":format!("^({})$", alternatives.join("|")),"not":{"pattern":"[^0-9]"}})
    }
}

/// Serialize remap object IDs losslessly without changing their typed/native identity.
pub(super) fn serialize_id<S: serde::Serializer>(id: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&id.to_string())
}

/// Decode a positive remap object ID, never a rounded JSON number.
pub(super) fn primitive_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    use serde::{Deserialize as _, de::Error as _};
    Scalar::Object.parse(&Value::deserialize(deserializer)?).map_err(D::Error::custom)
}
