//! The one place a `config.toml` document becomes a [`Config`].
//!
//! Deserializing the config tree straight from TOML instantiates every derived
//! `Deserialize` impl in it once per `toml` map-access type (about six), and the
//! tree has well over a hundred types. Parsing into a [`toml::Value`] first and
//! decoding that through [`serde_json::Value`] keeps one decoder instantiation
//! per type. Syntax errors still carry TOML line and column; a type error names
//! the dotted field path (`agent.agent_timeout_secs: invalid type: ...`) through
//! `serde_path_to_error`, though not the line.

use crate::config::Config;
use std::fmt;

/// A parser failure with safe location metadata and unchanged display text.
#[derive(Debug)]
pub(crate) struct ConfigParseFailure {
    pub(crate) field_path: Option<String>,
    pub(crate) line: Option<usize>,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl ConfigParseFailure {
    fn syntax(contents: &str, source: toml::de::Error) -> Self {
        let line = source.span().map(|span| {
            contents
                .as_bytes()
                .get(..span.start.min(contents.len()))
                .unwrap_or_default()
                .iter()
                .filter(|byte| **byte == b'\n')
                .count()
                + 1
        });
        let message = source.to_string();
        Self {
            field_path: None,
            line,
            message,
            source: Some(Box::new(source)),
        }
    }

    fn typed(field_path: String, source: serde_json::Error) -> Self {
        let message = format!("{field_path}: {source}");
        Self {
            field_path: Some(field_path),
            line: None,
            message,
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for ConfigParseFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConfigParseFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|error| error as _)
    }
}

/// Parse a `config.toml` document into a [`Config`].
#[inline(never)]
pub(crate) fn config_from_toml_str(contents: &str) -> anyhow::Result<Config> {
    let mut document: toml::Value =
        toml::from_str(contents).map_err(|error| ConfigParseFailure::syntax(contents, error))?;
    let provenance_document = document.clone();
    let retired_provider_type = document
        .get("cloud_providers")
        .and_then(toml::Value::as_array)
        .is_some_and(|providers| {
            providers.iter().any(|provider| {
                provider
                    .as_table()
                    .is_some_and(|entry| entry.contains_key("type"))
            })
        })
        && document
            .get("schema_version")
            .and_then(toml::Value::as_integer)
            .unwrap_or(0)
            < 14;
    crate::config::migrations::retire_cloud_provider_type::rewrite_document(&mut document)?;
    reject_non_finite_floats(&document, &mut String::new())?;
    let json = serde_json::to_string(&document)?;
    let mut unknown_keys = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(&json);
    let parsed = {
        let mut record_unknown = |path: serde_ignored::Path<'_>| {
            unknown_keys.push(path.to_string());
        };
        let deserializer = serde_ignored::Deserializer::new(&mut deserializer, &mut record_unknown);
        serde_path_to_error::deserialize(deserializer)
    };
    let mut config: Config = parsed.map_err(|error| {
        let path = error.path().to_string();
        ConfigParseFailure::typed(path, error.into_inner())
    })?;
    config
        .load_provenance_mut()
        .record_file_keys(&provenance_document);
    unknown_keys.sort();
    unknown_keys.dedup();
    config.load_diagnostics_mut().unknown_keys = unknown_keys;
    if retired_provider_type {
        config
            .load_diagnostics_mut()
            .record_migrated_key("cloud_providers[].type");
    }
    Ok(config)
}

/// JSON has no NaN or infinity: `serde_json::to_value` turns them into null,
/// which an `Option<f64>` field would silently read as unset. No config field
/// has a meaningful non-finite value, so refuse them up front with their path.
fn reject_non_finite_floats(value: &toml::Value, path: &mut String) -> anyhow::Result<()> {
    match value {
        toml::Value::Float(f) if !f.is_finite() => {
            anyhow::bail!("{path}: non-finite float {f} is not a valid config value")
        }
        toml::Value::Table(table) => {
            for (key, child) in table {
                let len = path.len();
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
                reject_non_finite_floats(child, path)?;
                path.truncate(len);
            }
        }
        toml::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let len = path.len();
                path.push_str(&format!("[{index}]"));
                reject_non_finite_floats(child, path)?;
                path.truncate(len);
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
