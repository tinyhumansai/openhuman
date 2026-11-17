//! Static, parse-once parameter schemas for agent tools.
//!
//! A tool whose `parameters_schema` is constant keeps the schema as a JSON
//! file next to its module and returns it through [`static_schema!`] instead
//! of rebuilding a large `json!` literal on every call. The file is embedded
//! with `include_str!`, parsed once into a `LazyLock<Value>`, and each call
//! returns a clone.

use serde_json::Value;

/// Parse an embedded schema. Panics with the call site on malformed JSON;
/// every converted tool has a test that forces its static, so a bad file
/// fails in CI rather than at runtime.
pub(crate) fn parse_embedded(site: &str, src: &str) -> Value {
    match serde_json::from_str(src) {
        Ok(value) => value,
        Err(error) => panic!("embedded tool schema at {site} is not valid JSON: {error}"),
    }
}

/// Return a constant tool schema parsed once from an embedded JSON string.
///
/// ```ignore
/// fn parameters_schema(&self) -> Value {
///     static_schema!(include_str!("parameters/my_tool.json"))
/// }
/// ```
macro_rules! static_schema {
    ($src:expr) => {{
        static SCHEMA: ::std::sync::LazyLock<::serde_json::Value> =
            ::std::sync::LazyLock::new(|| {
                $crate::tools::schema_cache::parse_embedded(concat!(file!(), ":", line!()), $src)
            });
        ::std::clone::Clone::clone(&*SCHEMA)
    }};
}

pub(crate) use static_schema;

#[cfg(test)]
#[path = "schema_cache_tests.rs"]
mod tests;
