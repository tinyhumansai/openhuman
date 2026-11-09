//! Shared projection of typed JSON payloads from product tool-result history.

use serde_json::Value;

/// Return the last tool-result JSON object whose `type` matches `expected_type`.
/// Callers adapt their transcript representation to an iterator of result text.
pub(crate) fn extract_last_typed_payload<'a, I>(contents: I, expected_type: &str) -> Option<Value>
where
    I: IntoIterator<Item = &'a str>,
    I::IntoIter: DoubleEndedIterator,
{
    contents.into_iter().rev().find_map(|content| {
        let value = serde_json::from_str::<Value>(content.trim()).ok()?;
        (value.get("type").and_then(Value::as_str) == Some(expected_type)).then_some(value)
    })
}
