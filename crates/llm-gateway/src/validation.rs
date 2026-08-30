//! Domain limits and validation helpers.

use std::{collections::BTreeMap, fmt};

use serde::de::DeserializeSeed;
use serde_json::{Map, Value};

use crate::LlmError;

pub const MAX_IDENTIFIER_BYTES: usize = 256;
pub const MAX_TOOL_NAME_BYTES: usize = 128;
pub const MAX_PROMPT_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_PROMPT_BYTES: usize = 32 * 1024;
pub const MAX_TOOLS: usize = 64;
pub const MAX_TOOL_DESCRIPTION_BYTES: usize = 4 * 1024;
pub const MAX_JSON_OBJECT_BYTES: usize = 16 * 1024;
pub const MAX_TOOL_SCHEMAS_BYTES: usize = 64 * 1024;
pub const MAX_RESPONSE_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_TOOL_CALLS: usize = 64;
pub const MAX_TOOL_ARGUMENTS_BYTES: usize = 64 * 1024;
pub const MAX_OUTPUT_TOKENS: u32 = 1_000_000;
pub const MAX_REPORTED_TOKENS: u32 = 1_000_000;
pub const MAX_TOTAL_TOKENS: u32 = 2_000_000;

pub(crate) fn validate_identifier(value: &str) -> Result<(), LlmError> {
    if value.is_empty() {
        return Err(LlmError::InvalidRequest);
    }
    validate_len(value.len(), MAX_IDENTIFIER_BYTES)
}

pub(crate) fn validate_tool_name(value: &str) -> Result<(), LlmError> {
    validate_len(value.len(), MAX_TOOL_NAME_BYTES)?;
    let mut bytes = value.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
    {
        return Err(LlmError::InvalidRequest);
    }
    Ok(())
}

pub(crate) fn validate_len(actual: usize, maximum: usize) -> Result<(), LlmError> {
    if actual > maximum {
        Err(LlmError::LimitExceeded)
    } else {
        Ok(())
    }
}

pub(crate) fn checked_sum<I>(values: I, maximum: usize) -> Result<usize, LlmError>
where
    I: IntoIterator<Item = usize>,
{
    let total = values.into_iter().try_fold(0_usize, |total, value| {
        total.checked_add(value).ok_or(LlmError::LimitExceeded)
    })?;
    validate_len(total, maximum)?;
    Ok(total)
}

pub(crate) fn canonical_object(input: &str) -> Result<String, LlmError> {
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let value = DuplicateRejectingSeed
        .deserialize(&mut deserializer)
        .map_err(|_| LlmError::InvalidRequest)?;
    deserializer.end().map_err(|_| LlmError::InvalidRequest)?;
    let Value::Object(object) = value else {
        return Err(LlmError::InvalidRequest);
    };
    let canonical = serde_json::to_string(&Value::Object(sort_object(object)))
        .map_err(|_| LlmError::InvalidRequest)?;
    validate_len(canonical.len(), MAX_JSON_OBJECT_BYTES)?;
    Ok(canonical)
}

pub(crate) fn validate_object_schema(canonical: &str) -> Result<(), LlmError> {
    let value: Value = serde_json::from_str(canonical).map_err(|_| LlmError::InvalidRequest)?;
    match value.get("type") {
        Some(Value::String(value)) if value == "object" => Ok(()),
        _ => Err(LlmError::Unsupported),
    }
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sort_value).collect()),
        Value::Object(object) => Value::Object(sort_object(object)),
        value => value,
    }
}

fn sort_object(object: Map<String, Value>) -> Map<String, Value> {
    object
        .into_iter()
        .map(|(key, value)| (key, sort_value(value)))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect()
}

struct DuplicateRejectingVisitor;

impl<'de> serde::de::Visitor<'de> for DuplicateRejectingVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(Self)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(DuplicateRejectingSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom("duplicate JSON object key"));
            }
            let value = object.next_value_seed(DuplicateRejectingSeed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

struct DuplicateRejectingSeed;

impl<'de> serde::de::DeserializeSeed<'de> for DuplicateRejectingSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateRejectingVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_sum_rejects_machine_overflow() {
        assert_eq!(
            checked_sum([usize::MAX, 1], usize::MAX),
            Err(LlmError::LimitExceeded)
        );
    }

    #[test]
    fn checked_sum_accepts_exact_limit_and_rejects_one_over() {
        assert_eq!(checked_sum([2, 3], 5), Ok(5));
        assert_eq!(checked_sum([2, 4], 5), Err(LlmError::LimitExceeded));
    }
}
