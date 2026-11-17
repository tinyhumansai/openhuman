//! Static checks for config parsing, ignored keys, and legacy-key migrations.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::schema::load::ConfigLoadFallback;
use crate::config::Config;
use crate::doctor::contract::{DoctorCheck, DoctorCtx, DoctorTier, Finding, FindingSeverity};

const CATEGORY: &str = "configuration";

pub(super) fn checks() -> Vec<Arc<dyn DoctorCheck>> {
    vec![
        Arc::new(ConfigParseCheck),
        Arc::new(UnknownConfigKeyCheck),
        Arc::new(LegacyConfigKeyCheck),
    ]
}

struct ConfigParseCheck;

#[async_trait]
impl DoctorCheck for ConfigParseCheck {
    fn id(&self) -> &str {
        "config/parse"
    }

    fn category(&self) -> &str {
        CATEGORY
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let config = ctx.config();
        let diagnostics = config.load_diagnostics();
        let fallback = diagnostics.fallback;
        let recovered = config.recovered_from_corruption;
        let Some(mut finding) = crate::doctor::ops::config_parse_finding(config) else {
            if fallback == ConfigLoadFallback::None && !recovered {
                return Vec::new();
            }

            let message = match fallback {
                ConfigLoadFallback::None => {
                    "The config loader recovered after detecting a corrupt config file.".to_owned()
                }
                ConfigLoadFallback::Backup => {
                    "The config file was invalid, so the loader used its backup.".to_owned()
                }
                ConfigLoadFallback::Defaults => {
                    "The config file and backup were invalid, so the loader used defaults."
                        .to_owned()
                }
            };
            return vec![Finding::error(
                self.id(),
                message,
                "Repair config.toml and review any backup before relying on the loaded settings.",
            )];
        };

        finding.check_id = self.id().to_owned();
        match fallback {
            ConfigLoadFallback::None if recovered => {
                finding
                    .message
                    .push_str(" The loader recovered after detecting a corrupt config file.");
                finding.fix_hint =
                    "Repair config.toml and review any backup before relying on the loaded settings."
                        .to_owned();
            }
            ConfigLoadFallback::None => {}
            ConfigLoadFallback::Backup => {
                finding
                    .message
                    .push_str(" The loader recovered using the backup config.");
                finding.fix_hint =
                    "Repair config.toml and review the backup before relying on the loaded settings."
                        .to_owned();
            }
            ConfigLoadFallback::Defaults => {
                finding
                    .message
                    .push_str(" The loader used default settings after recovery failed.");
                finding.fix_hint =
                    "Repair config.toml and review the backup before relying on the loaded settings."
                        .to_owned();
            }
        }
        vec![finding]
    }
}

struct UnknownConfigKeyCheck;

#[async_trait]
impl DoctorCheck for UnknownConfigKeyCheck {
    fn id(&self) -> &str {
        "config/unknown-key"
    }

    fn category(&self) -> &str {
        CATEGORY
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let config = ctx.config();
        let unknown_keys = &config.load_diagnostics().unknown_keys;
        if unknown_keys.is_empty() {
            return Vec::new();
        }

        let schema = serde_json::to_value(Config::default()).ok();
        unknown_keys
            .iter()
            .map(|path| unknown_key_finding(path, schema.as_ref()))
            .collect()
    }
}

fn unknown_key_finding(path: &str, schema: Option<&Value>) -> Finding {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    let suggestion = schema
        .and_then(|schema| known_sibling_keys(schema, path))
        .and_then(|candidates| closest_key(leaf, &candidates));

    let (message, fix_hint) = match suggestion {
        Some(suggestion) => (
            format!("Unknown config key `{leaf}` is ignored; did you mean `{suggestion}`?"),
            format!("Rename `{leaf}` to `{suggestion}` and rerun doctor."),
        ),
        None => (
            format!("Unknown config key `{leaf}` is ignored."),
            "Remove or rename the ignored key to a supported config field.".to_owned(),
        ),
    };
    let mut finding = Finding::new(
        "config/unknown-key",
        FindingSeverity::Warn,
        message,
        fix_hint,
    );
    finding.config_path = Some(path.to_owned());
    finding
}

fn known_sibling_keys(schema: &Value, path: &str) -> Option<Vec<String>> {
    let mut parts = path.split('.').collect::<Vec<_>>();
    let leaf = parts.pop()?;
    if leaf.is_empty() {
        return None;
    }

    let mut parent = schema;
    for part in parts {
        parent = descend_schema_path(parent, part)?;
    }
    let object = parent.as_object()?;
    Some(object.keys().cloned().collect())
}

fn descend_schema_path<'a>(mut value: &'a Value, part: &str) -> Option<&'a Value> {
    let bracket = part.find('[').unwrap_or(part.len());
    if bracket > 0 {
        value = value.as_object()?.get(&part[..bracket])?;
    }
    let mut remaining = &part[bracket..];
    while !remaining.is_empty() {
        let end = remaining.find(']')?;
        let index = remaining.get(1..end)?.parse::<usize>().ok()?;
        value = value.as_array()?.get(index)?;
        remaining = &remaining[end + 1..];
        if remaining.is_empty() {
            break;
        }
        if !remaining.starts_with('[') {
            return None;
        }
    }
    Some(value)
}

fn closest_key(unknown: &str, candidates: &[String]) -> Option<String> {
    let max_distance = (unknown.chars().count() / 3).max(1);
    candidates
        .iter()
        .map(|candidate| (edit_distance(unknown, candidate), candidate))
        .filter(|(distance, _)| *distance <= max_distance)
        .min_by(|(left_distance, left), (right_distance, right)| {
            left_distance
                .cmp(right_distance)
                .then_with(|| left.cmp(right))
        })
        .map(|(_, candidate)| candidate.clone())
}

/// Optimal-string-alignment distance, including adjacent transpositions.
fn edit_distance(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut distances = vec![vec![0; right.len() + 1]; left.len() + 1];
    for (index, row) in distances.iter_mut().enumerate() {
        row[0] = index;
    }
    for (index, distance) in distances[0].iter_mut().enumerate() {
        *distance = index;
    }

    for left_index in 1..=left.len() {
        for right_index in 1..=right.len() {
            let substitution = usize::from(left[left_index - 1] != right[right_index - 1]);
            distances[left_index][right_index] = (distances[left_index - 1][right_index] + 1)
                .min(distances[left_index][right_index - 1] + 1)
                .min(distances[left_index - 1][right_index - 1] + substitution);
            if left_index > 1
                && right_index > 1
                && left[left_index - 1] == right[right_index - 2]
                && left[left_index - 2] == right[right_index - 1]
            {
                distances[left_index][right_index] = distances[left_index][right_index]
                    .min(distances[left_index - 2][right_index - 2] + 1);
            }
        }
    }
    distances[left.len()][right.len()]
}

struct LegacyConfigKeyCheck;

#[async_trait]
impl DoctorCheck for LegacyConfigKeyCheck {
    fn id(&self) -> &str {
        "config/legacy-key"
    }

    fn category(&self) -> &str {
        CATEGORY
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        ctx.config()
            .load_diagnostics()
            .migrated_keys
            .iter()
            .map(|key| {
                let replacement = replacement_key(key);
                let message = match replacement {
                    Some(replacement) => {
                        format!("Legacy config key `{key}` was migrated to `{replacement}`.")
                    }
                    None => format!(
                        "Legacy config key `{key}` was migrated to the current config format."
                    ),
                };
                let fix_hint = match replacement {
                    Some(replacement) => {
                        format!("Update the config to use `{replacement}` and remove `{key}`.")
                    }
                    None => "Update the config to the current format and remove the legacy key."
                        .to_owned(),
                };
                let mut finding = Finding::new(self.id(), FindingSeverity::Info, message, fix_hint);
                finding.config_path = Some(key.clone());
                finding
            })
            .collect()
    }
}

fn replacement_key(key: &str) -> Option<&'static str> {
    match key {
        "api_url" => Some("inference_url"),
        _ => None,
    }
}
