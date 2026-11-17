//! Per-context config overrides reapplied after a fresh persisted reload.

use crate::config::schema::{
    AppliedInferenceOverride, ComposioHostCredential, EphemeralRoute, ResolvedEndpoints,
    SearchHostCredential,
};
use crate::config::Config;
use crate::profiles::provider_settings::ProfileProviderSettings;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ConfigContextOverlay {
    changes: Vec<Change>,
    runtime: RuntimeBindings,
}

#[derive(Clone)]
struct RuntimeBindings {
    profile_providers: Option<Arc<ProfileProviderSettings>>,
    workspace_dir: PathBuf,
    action_dir: PathBuf,
    config_path: PathBuf,
    cli_inference_snapshot: Option<AppliedInferenceOverride>,
    recovered_from_corruption: bool,
    ephemeral_route: Option<EphemeralRoute>,
    resolved_endpoints: ResolvedEndpoints,
    legacy_memory_sources: Vec<Value>,
    memory_legacy_backend: Option<crate::config::schema::memory::LegacyBackend>,
    composio_host_credential: Option<ComposioHostCredential>,
    search_engine: Option<String>,
    search_enabled_providers: Option<BTreeSet<String>>,
    search_parallel_route: Option<String>,
    search_gemini_route: Option<String>,
    search_host_credentials: BTreeMap<String, SearchHostCredential>,
}

#[derive(Clone)]
struct Change {
    path: Vec<String>,
    value: Option<Value>,
}

impl std::fmt::Debug for ConfigContextOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigContextOverlay")
            .field("changed_paths", &self.changes.len())
            .finish_non_exhaustive()
    }
}

impl ConfigContextOverlay {
    pub(crate) fn derive(parent: &Config, child: &Config) -> anyhow::Result<Arc<Self>> {
        let same_source = same_source(parent, child);
        anyhow::ensure!(
            same_source,
            "cannot derive config overrides from different persistence sources"
        );
        let inherited = parent.runtime_state.context_overlay.lock().unwrap().clone();
        let mut changes = inherited
            .as_ref()
            .map(|overlay| overlay.changes.clone())
            .unwrap_or_default();
        let parent_value = serde_json::to_value(parent)?;
        let target = serde_json::to_value(child)?;
        diff_values(&parent_value, &target, &mut Vec::new(), &mut changes);

        let runtime = RuntimeBindings::capture(child);
        Ok(Arc::new(Self { changes, runtime }))
    }

    /// Record the small set of serialized config fields owned by a hosted
    /// profile. A profile config is derived from operator config but persisted
    /// under another scope, so diffing the two complete documents would pin
    /// operator settings into the profile after a conflict refresh.
    pub(crate) fn derive_profile(
        parent: Option<&Config>,
        child: &Config,
    ) -> anyhow::Result<Arc<Self>> {
        let mut changes = Vec::new();
        if let Some(parent) = parent.filter(|parent| same_source(parent, child)) {
            if let Some(inherited) = parent.runtime_state.context_overlay.lock().unwrap().clone() {
                changes.extend(inherited.changes.clone());
            }
            let parent_value = serde_json::to_value(parent)?;
            let child_value = serde_json::to_value(child)?;
            diff_values(&parent_value, &child_value, &mut Vec::new(), &mut changes);
        }

        let child_value = serde_json::to_value(child)?;
        for path in PROFILE_OWNED_PATHS {
            // Some owned defaults are omitted by serde (for example Legacy
            // memory layout). Record absence as an explicit removal so a
            // refreshed document cannot override the profile's default.
            let value = value_at_path(&child_value, path).cloned();
            let path: Vec<String> = path.iter().map(|part| (*part).to_string()).collect();
            changes.retain(|change| !change.path.starts_with(&path));
            changes.push(Change { path, value });
        }

        let runtime = RuntimeBindings::capture(child);
        Ok(Arc::new(Self { changes, runtime }))
    }

    pub(crate) fn reapply(&self, fresh: &Config) -> anyhow::Result<Config> {
        let mut value = serde_json::to_value(fresh)?;
        for change in &self.changes {
            apply_change(&mut value, &change.path, change.value.clone());
        }
        let mut result: Config = serde_json::from_value(value)?;
        copy_runtime_bindings(fresh, &mut result);
        self.runtime.apply(&mut result);
        result.runtime_state.document_snapshot = std::sync::Mutex::new(
            fresh
                .runtime_state
                .document_snapshot
                .lock()
                .unwrap()
                .clone(),
        );
        result.runtime_state.context_snapshot = fresh.runtime_state.context_snapshot.clone();
        result.runtime_state.context_overlay = std::sync::Mutex::new(Some(Arc::new(Self {
            changes: self.changes.clone(),
            runtime: self.runtime.clone(),
        })));
        Ok(result)
    }
}

const PROFILE_OWNED_PATHS: &[&[&str]] = &[
    &["files_dir_override"],
    &["memory", "agent_id"],
    &["memory", "root"],
    &["memory", "layout"],
    &["agent", "channel_permissions"],
    &["agent", "max_tool_iterations_override"],
    &["autonomy"],
    &["cron", "agent_jobs_only"],
    &["cron", "min_interval_secs"],
    &["cron", "max_active_jobs"],
];

fn same_source(parent: &Config, child: &Config) -> bool {
    let parent_anchor = parent
        .runtime_state
        .document_snapshot
        .lock()
        .unwrap()
        .anchor
        .clone();
    let child_anchor = child
        .runtime_state
        .document_snapshot
        .lock()
        .unwrap()
        .anchor
        .clone();
    match (&parent_anchor, &child_anchor) {
        (Some(parent), Some(child)) => parent == child,
        (None, None) => {
            super::source::normalized_path(&parent.config_path)
                == super::source::normalized_path(&child.config_path)
        }
        _ => false,
    }
}

fn value_at_path<'a>(mut value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    for part in path {
        value = value.get(*part)?;
    }
    Some(value)
}

impl RuntimeBindings {
    fn capture(source: &Config) -> Self {
        Self {
            profile_providers: source.profile_providers.clone(),
            workspace_dir: source.workspace_dir.clone(),
            action_dir: source.action_dir.clone(),
            config_path: source.config_path.clone(),
            cli_inference_snapshot: source.cli_inference_snapshot.clone(),
            recovered_from_corruption: source.recovered_from_corruption,
            ephemeral_route: source.ephemeral_route.clone(),
            resolved_endpoints: source.resolved_endpoints.clone(),
            legacy_memory_sources: source.legacy_memory_sources.clone(),
            memory_legacy_backend: source.memory.legacy_backend.clone(),
            composio_host_credential: source.composio.host_credential.clone(),
            search_engine: source.search.engine.clone(),
            search_enabled_providers: source.search.enabled_providers.clone(),
            search_parallel_route: source.search.parallel_route.clone(),
            search_gemini_route: source.search.gemini_route.clone(),
            search_host_credentials: source.search.host_credentials.clone(),
        }
    }

    fn apply(&self, target: &mut Config) {
        target.profile_providers = self.profile_providers.clone();
        target.workspace_dir = self.workspace_dir.clone();
        target.action_dir = self.action_dir.clone();
        target.config_path = self.config_path.clone();
        target.cli_inference_snapshot = self.cli_inference_snapshot.clone();
        target.recovered_from_corruption = self.recovered_from_corruption;
        target.ephemeral_route = self.ephemeral_route.clone();
        target.resolved_endpoints = self.resolved_endpoints.clone();
        target.legacy_memory_sources = self.legacy_memory_sources.clone();
        target.memory.legacy_backend = self.memory_legacy_backend.clone();
        target.composio.host_credential = self.composio_host_credential.clone();
        target.search.engine = self.search_engine.clone();
        target.search.enabled_providers = self.search_enabled_providers.clone();
        target.search.parallel_route = self.search_parallel_route.clone();
        target.search.gemini_route = self.search_gemini_route.clone();
        target.search.host_credentials = self.search_host_credentials.clone();
    }
}

fn diff_values(base: &Value, target: &Value, path: &mut Vec<String>, changes: &mut Vec<Change>) {
    match (base, target) {
        (Value::Object(base), Value::Object(target)) => {
            for (key, value) in target {
                path.push(key.clone());
                if let Some(previous) = base.get(key) {
                    diff_values(previous, value, path, changes);
                } else {
                    changes.push(Change {
                        path: path.clone(),
                        value: Some(value.clone()),
                    });
                }
                path.pop();
            }
            for key in base.keys().filter(|key| !target.contains_key(*key)) {
                path.push(key.clone());
                changes.push(Change {
                    path: path.clone(),
                    value: None,
                });
                path.pop();
            }
        }
        _ if base != target => changes.push(Change {
            path: path.clone(),
            value: Some(target.clone()),
        }),
        _ => {}
    }
}

fn apply_change(current: &mut Value, path: &[String], value: Option<Value>) {
    let Some((key, rest)) = path.split_first() else {
        if let Some(value) = value {
            *current = value;
        }
        return;
    };
    if !current.is_object() {
        if value.is_none() {
            return;
        }
        *current = Value::Object(Default::default());
    }
    let object = current
        .as_object_mut()
        .expect("object created for nested config override");
    if rest.is_empty() {
        if let Some(value) = value {
            object.insert(key.clone(), value);
        } else {
            object.remove(key);
        }
        return;
    }
    let child = object
        .entry(key.clone())
        .or_insert_with(|| Value::Object(Default::default()));
    apply_change(child, rest, value);
}

pub(crate) fn copy_runtime_bindings(source: &Config, target: &mut Config) {
    RuntimeBindings::capture(source).apply(target);
}
