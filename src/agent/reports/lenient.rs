//! Lenient deserializers for LLM-produced JSON.
//!
//! Agent CLIs frequently return `"8"` for numbers, `{"name": ...}` objects for
//! strings, or plain strings inside arrays. These helpers coerce such shapes
//! instead of failing outright. Ported from deepwiki-rs.

use serde::{Deserialize, Deserializer};

/// Convert any JSON value to a displayable string.
pub fn any_to_string(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s,
        serde_json::Value::Bool(v) => v.to_string(),
        serde_json::Value::Number(v) => v.to_string(),
        serde_json::Value::Array(v) => serde_json::to_string(&v).unwrap_or_default(),
        serde_json::Value::Object(v) => serde_json::to_string(&v).unwrap_or_default(),
    }
}

/// Like [`any_to_string`] but digs through objects for a textual field.
pub fn json_value_to_string(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s,
        serde_json::Value::Bool(v) => v.to_string(),
        serde_json::Value::Number(v) => v.to_string(),
        serde_json::Value::Array(v) => serde_json::to_string(&v).unwrap_or_default(),
        serde_json::Value::Object(v) => {
            for key in [
                "name",
                "module",
                "path",
                "summary",
                "description",
                "title",
                "value",
                "text",
                "id",
            ] {
                if let Some(inner) = v.get(key) {
                    let text = json_value_to_string(inner.clone());
                    if !text.is_empty() {
                        return text;
                    }
                }
            }
            serde_json::to_string(&v).unwrap_or_default()
        }
    }
}

/// Accepts string, number, bool, or object-with-text-field → `String`.
pub fn de_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(json_value_to_string(value))
}

/// Accepts anything → `Option<String>` (empty text becomes `None`).
pub fn de_opt_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(None);
    }
    let text = json_value_to_string(value);
    if text.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

/// Accepts number, numeric string, or bool → `f64`.
pub fn de_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let result = match value {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        serde_json::Value::Bool(v) => f64::from(v as u8),
        _ => 0.0,
    };
    Ok(result)
}

/// Accepts number, numeric string, or bool → `usize` (clamped ≥ 0).
pub fn de_usize<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let result = match value {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) as i64,
        serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
        serde_json::Value::Bool(v) => i64::from(v),
        _ => 0,
    };
    Ok(result.max(0) as usize)
}

/// Accepts number or numeric string → `u8` (clamped).
pub fn de_u8<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let parsed = match value {
        serde_json::Value::Number(n) => n.as_u64().map(|v| v as i64).unwrap_or(0),
        serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
        serde_json::Value::Bool(v) => i64::from(v),
        _ => 0,
    };
    Ok(parsed.clamp(0, u8::MAX as i64) as u8)
}

/// Accepts bool, "true"/"yes"/"1" strings, or numbers → `bool`.
pub fn de_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let result = match value {
        serde_json::Value::Bool(v) => v,
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        serde_json::Value::String(s) => {
            matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "yes" | "y")
        }
        _ => false,
    };
    Ok(result)
}

/// Accepts an array, a bare string, or null → `Vec<String>`.
pub fn de_vec_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::Array(items) => Ok(items
            .into_iter()
            .map(any_to_string)
            .filter(|s| !s.trim().is_empty())
            .collect()),
        other => {
            let one = any_to_string(other);
            if one.trim().is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![one])
            }
        }
    }
}

/// Accepts an array of objects/strings → `Vec<T>`; non-object items are
/// skipped, strings may salvage a `name`-like field via `fallback`.
pub fn de_vec_obj<'de, D, T, F>(deserializer: D, fallback: F) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
    F: Fn(String) -> Option<T>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                match &item {
                    serde_json::Value::Object(_) => {
                        if let Ok(parsed) = serde_json::from_value::<T>(item) {
                            out.push(parsed);
                        }
                    }
                    other => {
                        let s = any_to_string(other.clone());
                        if !s.trim().is_empty()
                            && let Some(v) = fallback(s)
                        {
                            out.push(v);
                        }
                    }
                }
            }
            Ok(out)
        }
        serde_json::Value::Object(_) => {
            if let Ok(parsed) = serde_json::from_value::<T>(value) {
                Ok(vec![parsed])
            } else {
                Ok(Vec::new())
            }
        }
        _ => Ok(Vec::new()),
    }
}
