//! Decode pre-v14 cloud-provider discriminators before the typed schema.
//!
//! The v13 -> v14 runner persists the normalized document once. This decoder
//! bridge is retained for offline upgrades from older configs.

use crate::config::schema::cloud_providers::builtin_cloud_provider;

// keep until v0.66 (review after 2027-01-11): removal requires a documented
// pre-schema-14 upgrade/import boundary, including offline workspace imports.
pub(crate) fn rewrite_document(document: &mut toml::Value) -> anyhow::Result<()> {
    let version = document
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    if version >= 14 {
        return Ok(());
    }
    let Some(providers) = document
        .get_mut("cloud_providers")
        .and_then(toml::Value::as_array_mut)
    else {
        return Ok(());
    };
    for (index, value) in providers.iter_mut().enumerate() {
        let Some(entry) = value.as_table_mut() else {
            continue;
        };
        let Some(discriminator) = entry.remove("type") else {
            continue;
        };
        let kind = discriminator
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("cloud_providers[{index}].type: expected a string"))?
            .trim();
        let builtin = builtin_cloud_provider(kind);
        for (field, default) in [
            ("slug", kind),
            ("label", builtin.map(|p| p.label).unwrap_or("Custom")),
            ("endpoint", builtin.map(|p| p.endpoint).unwrap_or("")),
        ] {
            match entry.get(field) {
                None => {
                    entry.insert(field.into(), toml::Value::String(default.into()));
                }
                Some(toml::Value::String(current)) if current.is_empty() => {
                    entry.insert(field.into(), toml::Value::String(default.into()));
                }
                _ => {}
            }
        }
        if !entry.contains_key("auth_style") {
            if let Some(provider) = builtin {
                entry.insert(
                    "auth_style".into(),
                    toml::Value::String(
                        match provider.auth_style {
                            crate::config::schema::cloud_providers::AuthStyle::Bearer => "bearer",
                            crate::config::schema::cloud_providers::AuthStyle::Anthropic => {
                                "anthropic"
                            }
                            crate::config::schema::cloud_providers::AuthStyle::OpenhumanJwt => {
                                "openhuman_jwt"
                            }
                            crate::config::schema::cloud_providers::AuthStyle::None => "none",
                        }
                        .into(),
                    ),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "retire_cloud_provider_type_tests.rs"]
mod tests;
