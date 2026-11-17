//! Value-source metadata collected while a config document is loaded.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// The source that supplied a resolved config value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProvenanceSource {
    /// The value came from a process-local CLI launch option.
    Cli,
    /// The value came from the process environment.
    Env,
    /// The value came from a `.env` file loaded for this process.
    Dotenv,
    /// The value was explicitly present in the persisted config document.
    File,
    /// The value came from the config schema's default.
    #[default]
    Default,
}

/// Source metadata for one logical config field. Config values are never kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ValueProvenance {
    /// Origin of the resolved value.
    pub(crate) source: ProvenanceSource,
    /// Environment variable name when an environment source won.
    pub(crate) env_var: Option<String>,
    /// Whether the persisted file also explicitly contained this field.
    pub(crate) file_also_set: bool,
}

/// Metadata about the dotenv file considered by the CLI loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DotenvLoadProvenance {
    /// File selected by dotenvy.
    pub(crate) path: PathBuf,
    /// Names present in the process environment, so dotenvy preserved them.
    pub(crate) skipped_vars: Vec<String>,
    /// Names supplied by the dotenv file.
    pub(crate) loaded_vars: Vec<String>,
}

impl DotenvLoadProvenance {
    /// Build dotenv metadata from variable names only; values are discarded.
    pub(crate) fn from_variables<I, K, P, PK>(path: PathBuf, variables: I, process_vars: P) -> Self
    where
        I: IntoIterator<Item = K>,
        K: AsRef<str>,
        P: IntoIterator<Item = PK>,
        PK: AsRef<str>,
    {
        let process_vars: HashSet<String> = process_vars
            .into_iter()
            .map(|name| name.as_ref().to_owned())
            .collect();
        let mut seen = HashSet::new();
        let mut skipped_vars = Vec::new();
        let mut loaded_vars = Vec::new();

        for name in variables {
            let name = name.as_ref();
            if !seen.insert(name.to_owned()) {
                continue;
            }
            if process_vars.contains(name) {
                skipped_vars.push(name.to_owned());
            } else {
                loaded_vars.push(name.to_owned());
            }
        }

        Self {
            path,
            skipped_vars,
            loaded_vars,
        }
    }

    /// Keep the original source classification when the same file is loaded
    /// again and its previously loaded variables are already in the process.
    pub(crate) fn preserve_previous_origin(
        &mut self,
        previous: Option<&Self>,
        process_vars: &HashSet<String>,
    ) {
        let Some(previous) = previous.filter(|previous| same_file(&self.path, &previous.path))
        else {
            return;
        };
        let current_vars: HashSet<String> = self
            .loaded_vars
            .iter()
            .chain(&self.skipped_vars)
            .cloned()
            .collect();

        for name in &previous.loaded_vars {
            if process_vars.contains(name) && current_vars.contains(name) {
                self.skipped_vars.retain(|current| current != name);
                push_unique(&mut self.loaded_vars, name);
            }
        }
        for name in &previous.skipped_vars {
            if process_vars.contains(name) && current_vars.contains(name) {
                self.loaded_vars.retain(|current| current != name);
                push_unique(&mut self.skipped_vars, name);
            }
        }
    }
}

fn same_file(left: &std::path::Path, right: &std::path::Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}

/// Provenance for a config snapshot. It stores paths and source labels only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConfigProvenance {
    fields: BTreeMap<String, ValueProvenance>,
    pub(crate) dotenv: Option<DotenvLoadProvenance>,
}

impl ConfigProvenance {
    /// Mark explicit leaves in the parsed TOML document as file-sourced.
    pub(crate) fn record_file_keys(&mut self, document: &toml::Value) {
        record_toml_paths(document, &mut String::new(), &mut self.fields);
    }

    /// Make a source map directly from the persisted TOML tree.
    #[cfg(test)]
    pub(crate) fn from_file_document(document: &toml::Value) -> Self {
        let mut provenance = Self::default();
        provenance.record_file_keys(document);
        provenance
    }

    /// Record an environment override after its value has passed validation.
    pub(crate) fn record_env_override(
        &mut self,
        path: &str,
        env_var: &str,
        source: ProvenanceSource,
    ) {
        debug_assert!(matches!(
            source,
            ProvenanceSource::Env | ProvenanceSource::Dotenv
        ));
        let file_also_set = self
            .fields
            .get(path)
            .is_some_and(|entry| entry.source == ProvenanceSource::File || entry.file_also_set);
        self.fields.insert(
            path.to_owned(),
            ValueProvenance {
                source,
                env_var: Some(env_var.to_owned()),
                file_also_set,
            },
        );
    }

    pub(crate) fn record_cli_override(&mut self, path: &str) {
        let file_also_set = self
            .fields
            .get(path)
            .is_some_and(|entry| entry.source == ProvenanceSource::File || entry.file_also_set);
        self.fields.insert(
            path.to_owned(),
            ValueProvenance {
                source: ProvenanceSource::Cli,
                env_var: None,
                file_also_set,
            },
        );
    }

    /// Return source metadata for a field, defaulting to the schema default.
    pub(crate) fn field(&self, path: &str) -> super::ValueProvenance {
        self.fields.get(path).cloned().unwrap_or_default()
    }

    /// Iterate over recorded fields without exposing or retaining config values.
    pub(crate) fn fields(&self) -> impl Iterator<Item = (&str, &ValueProvenance)> {
        self.fields
            .iter()
            .map(|(path, provenance)| (path.as_str(), provenance))
    }

    /// Whether a logical config field or any of its descendants was supplied
    /// explicitly instead of coming from schema defaults.
    pub(crate) fn has_explicit_under(&self, path: &str) -> bool {
        self.fields.iter().any(|(candidate, provenance)| {
            let is_field_or_descendant = candidate == path
                || candidate
                    .strip_prefix(path)
                    .is_some_and(|suffix| suffix.starts_with('.') || suffix.starts_with("[]"));
            is_field_or_descendant && provenance.source != ProvenanceSource::Default
        })
    }

    pub(crate) fn set_dotenv(&mut self, dotenv: Option<DotenvLoadProvenance>) {
        self.dotenv = dotenv;
    }
}

fn record_toml_paths(
    value: &toml::Value,
    path: &mut String,
    fields: &mut BTreeMap<String, ValueProvenance>,
) {
    match value {
        toml::Value::Table(table) if !table.is_empty() => {
            if !path.is_empty() {
                fields.insert(
                    path.clone(),
                    ValueProvenance {
                        source: ProvenanceSource::File,
                        env_var: None,
                        file_also_set: false,
                    },
                );
            }
            for (key, child) in table {
                let original_len = path.len();
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
                record_toml_paths(child, path, fields);
                path.truncate(original_len);
            }
        }
        toml::Value::Array(values) if values.iter().any(toml::Value::is_table) => {
            if !path.is_empty() {
                fields.insert(
                    path.clone(),
                    ValueProvenance {
                        source: ProvenanceSource::File,
                        env_var: None,
                        file_also_set: false,
                    },
                );
            }
            for child in values.iter().filter(|child| child.is_table()) {
                let original_len = path.len();
                path.push_str("[]");
                record_toml_paths(child, path, fields);
                path.truncate(original_len);
            }
        }
        _ => {
            if !path.is_empty() {
                fields.insert(
                    path.clone(),
                    ValueProvenance {
                        source: ProvenanceSource::File,
                        env_var: None,
                        file_also_set: false,
                    },
                );
            }
        }
    }
}

static CURRENT_DOTENV_LOAD: OnceLock<Mutex<Option<DotenvLoadProvenance>>> = OnceLock::new();

fn current_dotenv_slot() -> &'static Mutex<Option<DotenvLoadProvenance>> {
    CURRENT_DOTENV_LOAD.get_or_init(|| Mutex::new(None))
}

pub(crate) fn set_current_dotenv_load(load: Option<DotenvLoadProvenance>) {
    *current_dotenv_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = load;
}

pub(crate) fn current_dotenv_load() -> Option<DotenvLoadProvenance> {
    current_dotenv_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

pub(crate) fn dotenv_loaded_variable(name: &str) -> bool {
    current_dotenv_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .is_some_and(|load| load.loaded_vars.iter().any(|loaded| loaded == name))
}

impl crate::config::schema::Config {
    pub(crate) fn load_provenance(&self) -> &ConfigProvenance {
        &self.runtime_state.provenance
    }

    pub(crate) fn load_provenance_mut(&mut self) -> &mut ConfigProvenance {
        &mut self.runtime_state.provenance
    }
}
