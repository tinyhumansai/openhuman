//! Helpers the host domains' `storage_schema()` declarations and their v1 -> v2
//! steps share: owner construction, time conversion and id construction.
//!
//! A step is a pure function of its input (schema-v2 section 4), so none of
//! these read a clock or the environment.

use serde_json::{Map, Value};
use tinystoragedrivers::{Key, StorageError};

use crate::storage::import::Record;

/// Source row passed through a typed owner import conversion.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LegacySourceRecord {
    pub id: String,
    pub body: Value,
    pub meta: tinystoragedrivers::Meta,
}

/// Target row emitted by a typed owner import conversion.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LegacyTargetRecord {
    pub collection: String,
    pub id: String,
    pub body: Value,
    pub created_ms: Option<i64>,
    pub schema: Option<u32>,
    pub expires_ms: Option<i64>,
    pub updated_ms: Option<i64>,
}

impl LegacyTargetRecord {
    pub fn new(collection: impl Into<String>, id: impl Into<String>, body: Value) -> Self {
        Self {
            collection: collection.into(),
            id: id.into(),
            body,
            created_ms: None,
            schema: None,
            expires_ms: None,
            updated_ms: None,
        }
    }

    pub fn created_ms(mut self, created_ms: i64) -> Self {
        self.created_ms = Some(created_ms);
        self
    }

    /// Carries source driver metadata through a body-shape conversion.
    pub fn source_metadata(mut self, source: tinystoragedrivers::Meta) -> Self {
        self.schema = (source.schema != 0).then_some(source.schema);
        self.expires_ms = source.expires_ms;
        self.updated_ms = Some(source.updated_ms);
        if self.created_ms.is_none() && source.created_ms != 0 {
            self.created_ms = Some(source.created_ms);
        }
        self
    }
}

/// Converts one legacy collection using an owner-specific pure transform.
/// Records from other collections pass through unchanged.
pub(crate) fn map_legacy_collection<F>(
    records: Vec<Record>,
    source_collection: &str,
    convert: F,
) -> anyhow::Result<Vec<Record>>
where
    F: FnOnce(Vec<LegacySourceRecord>) -> anyhow::Result<Vec<LegacyTargetRecord>>,
{
    let mut source = Vec::new();
    let mut output = Vec::new();
    for record in records {
        if record.collection == source_collection {
            source.push(LegacySourceRecord {
                id: record.id,
                body: record.doc,
                meta: tinystoragedrivers::Meta {
                    schema: record.schema.unwrap_or_default(),
                    created_ms: record.created_ms.unwrap_or_default(),
                    updated_ms: record.updated_ms.unwrap_or_default(),
                    expires_ms: record.expires_ms,
                },
            });
        } else {
            output.push(record);
        }
    }
    output.extend(convert(source)?.into_iter().map(|record| Record {
        collection: record.collection,
        id: record.id,
        doc: record.body,
        created_ms: record.created_ms,
        schema: record.schema,
        expires_ms: record.expires_ms,
        updated_ms: record.updated_ms,
    }));
    Ok(output)
}

/// Unix epoch milliseconds of an RFC 3339 timestamp, `None` when it does not
/// parse.
#[must_use]
pub fn rfc3339_ms(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.timestamp_millis())
}

/// The RFC 3339 form of `ms` epoch milliseconds, for an RPC type that still
/// presents a formatted time. An out-of-range value reads as the epoch.
#[must_use]
pub fn ms_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The time held by `body[field]`, as epoch milliseconds: an RFC 3339 string.
#[must_use]
pub fn time_ms(body: &Value, field: &str) -> Option<i64> {
    body.get(field).and_then(Value::as_str).and_then(rfc3339_ms)
}

/// The document id for a single-part natural key.
///
/// # Errors
///
/// When `part` is empty.
pub fn key_of(part: &str) -> Result<String, StorageError> {
    key_of_parts([part])
}

/// Builds a validated key from logical parts without allowing a part to
/// introduce a separator or the reserved leading hash marker.
pub fn key_of_parts<'a>(parts: impl IntoIterator<Item = &'a str>) -> Result<String, StorageError> {
    let mut encoded = String::new();
    for (index, part) in parts.into_iter().enumerate() {
        if part.is_empty() {
            return Err(StorageError::invalid_input("key parts must not be empty"));
        }
        if index != 0 {
            encoded.push('/');
        }
        for ch in part.chars() {
            match ch {
                '%' => encoded.push_str("%25"),
                '/' => encoded.push_str("%2F"),
                '\0' => encoded.push_str("%00"),
                '~' => encoded.push_str("%7E"),
                _ => encoded.push(ch),
            }
        }
    }
    Key::new(encoded).map(|key| key.as_str().to_owned())
}

/// Creates an opaque id for a newly generated owner record.
#[must_use]
pub fn fresh_key() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// `camelCase` as `snake_case`: `authorId` -> `author_id`.
#[must_use]
pub fn snake_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `snake_case` as `camelCase`: `author_id` -> `authorId`.
#[must_use]
pub fn camel_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper = false;
    for ch in key.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// A copy of `value` with every object key (at any depth) in snake case.
/// `renames` lists `(camel key, snake key)` pairs that take precedence, for
/// fields whose v2 name is not the mechanical one (`createdAtMillis` ->
/// `created_ms`). Array elements are converted; strings and numbers are not
/// touched.
#[must_use]
pub fn snake_keys(value: &Value, renames: &[(&str, &str)]) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, inner)| {
                    let key = renames
                        .iter()
                        .find(|(camel, _)| camel == key)
                        .map_or_else(|| snake_case(key), |(_, snake)| (*snake).to_owned());
                    (key, snake_keys(inner, renames))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|v| snake_keys(v, renames)).collect()),
        other => other.clone(),
    }
}

/// The inverse of [`snake_keys`] for the same `renames`.
#[must_use]
pub fn camel_keys(value: &Value, renames: &[(&str, &str)]) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, inner)| {
                    let key = renames
                        .iter()
                        .find(|(_, snake)| snake == key)
                        .map_or_else(|| camel_case(key), |(camel, _)| (*camel).to_owned());
                    (key, camel_keys(inner, renames))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|v| camel_keys(v, renames)).collect()),
        other => other.clone(),
    }
}

/// A body object builder that drops absent values, so a v2 body never carries
/// `null` for a field that was never set.
#[derive(Default)]
pub struct Body(Map<String, Value>);

impl Body {
    /// An empty body.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `field` to `value`.
    #[must_use]
    pub fn set(mut self, field: &str, value: impl Into<Value>) -> Self {
        self.0.insert(field.to_owned(), value.into());
        self
    }

    /// Sets `field` when `value` is present.
    #[must_use]
    pub fn set_some<V: Into<Value>>(self, field: &str, value: Option<V>) -> Self {
        match value {
            Some(value) => self.set(field, value),
            None => self,
        }
    }

    /// Copies `field` from `from` when it is present and not null.
    #[must_use]
    pub fn copy(self, from: &Value, field: &str) -> Self {
        match from.get(field) {
            Some(value) if !value.is_null() => self.set(field, value.clone()),
            _ => self,
        }
    }

    /// Copies `from_field` of `from` to `field` when present and not null.
    #[must_use]
    pub fn copy_as(self, from: &Value, from_field: &str, field: &str) -> Self {
        match from.get(from_field) {
            Some(value) if !value.is_null() => self.set(field, value.clone()),
            _ => self,
        }
    }

    /// The finished body.
    #[must_use]
    pub fn build(self) -> Value {
        Value::Object(self.0)
    }
}

#[cfg(test)]
#[path = "schema_support_tests.rs"]
mod tests;
