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

/// Parse a `config.toml` document into a [`Config`].
#[inline(never)]
pub(crate) fn config_from_toml_str(contents: &str) -> anyhow::Result<Config> {
    let mut document: toml::Value = toml::from_str(contents)?;
    crate::config::migrations::retire_cloud_provider_type::rewrite_document(&mut document)?;
    crate::config::migrations::security_policy::rewrite_document(&mut document);
    reject_non_finite_floats(&document, &mut String::new())?;
    let json = serde_json::to_value(document)?;
    serde_path_to_error::deserialize(json).map_err(|err| {
        let path = err.path().to_string();
        anyhow::anyhow!("{path}: {}", err.into_inner())
    })
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
