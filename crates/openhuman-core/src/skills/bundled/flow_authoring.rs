//! Bus-backed materialization for the flow-authoring builtin skill.
//!
//! TinyFlows owns the resource bytes. This adapter asks the published Copilot
//! interface for its current files, lets TinySkills own their safe atomic
//! materialization, and keeps a process snapshot so synchronous discovery can
//! verify the disk tree without reaching across the async bus boundary.

use sha2::Digest;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

use crate::config::Config;
use crate::core::runtime::{DispatchCtx, DomainSet};
use tinyflows_bus::copilot::ResourceFile;
use tinyskills::{MaterializeEntry, MaterializeFile, MaterializeSource, Skill, SkillScope};

use super::super::ops_discover::DiscoveryGate;

const FLOW_SKILL_ID: &str = "flow-authoring";
const FLOW_ROOT_NAME: &str = "builtin-flow-skills";
const COPILOT_RESOURCE_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
struct Snapshot {
    generation: u64,
    files: Vec<ResourceFile>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct SnapshotKey {
    workspace: PathBuf,
    gate: DiscoveryGate,
}

static SNAPSHOTS: LazyLock<RwLock<HashMap<SnapshotKey, Snapshot>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static WARMUP_LOCKS: LazyLock<Mutex<HashMap<SnapshotKey, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SCHEDULED: LazyLock<Mutex<HashSet<SnapshotKey>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Product-owned root for one explicit workspace/config/tenant snapshot,
/// separate from static bundles and other profiles sharing the workspace.
pub(crate) fn root(workspace: &Path, gate: &DiscoveryGate) -> PathBuf {
    workspace
        .join(".openhuman")
        .join(FLOW_ROOT_NAME)
        .join(scope_directory(gate))
}

/// Schedule optional resource loading without holding up runtime bootstrap,
/// channel construction, or the first ordinary agent turn.
pub(crate) fn schedule_prewarm(ctx: &DispatchCtx, config: &Config, domains: Option<DomainSet>) {
    let owned_ctx = ctx.clone();
    let owned_config = config.clone();
    schedule_prewarm_operation(ctx, config, domains, async move {
        prewarm(&owned_ctx, &owned_config, domains).await;
    });
}

fn schedule_prewarm_operation(
    ctx: &DispatchCtx,
    config: &Config,
    domains: Option<DomainSet>,
    operation: impl std::future::Future<Output = ()> + Send + 'static,
) -> bool {
    let gate = DiscoveryGate::from_config(ctx, config, domains);
    if !gate.flow_authoring_enabled() {
        return false;
    }
    let workspace = config.workspace_dir.clone();
    let key = snapshot_key(&workspace, &gate);
    if generation_if_current(&workspace, &gate).is_some() {
        return false;
    }
    let state = crate::modules::ops::state_of(crate::modules::tinyflows::MODULE_ID);
    if state == crate::modules::types::ModuleState::Failed {
        return false;
    }

    spawn_background(key, operation)
}

fn spawn_background(
    key: SnapshotKey,
    operation: impl std::future::Future<Output = ()> + Send + 'static,
) -> bool {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return false;
    };
    let inserted = SCHEDULED
        .lock()
        .is_ok_and(|mut scheduled| scheduled.insert(key.clone()));
    if !inserted {
        return false;
    }
    handle.spawn(async move {
        struct ScheduledGuard(SnapshotKey);
        impl Drop for ScheduledGuard {
            fn drop(&mut self) {
                if let Ok(mut scheduled) = SCHEDULED.lock() {
                    scheduled.remove(&self.0);
                }
            }
        }
        let _guard = ScheduledGuard(key);
        operation.await;
    });
    true
}

fn warmup_lock(key: &SnapshotKey) -> Arc<AsyncMutex<()>> {
    let mut locks = WARMUP_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
    )
}

/// Fetch and materialize Copilot resources when every owning capability is
/// enabled. Failures clear the snapshot and return normally so optional module
/// availability never blocks ordinary chat startup.
pub(crate) async fn prewarm(ctx: &DispatchCtx, config: &Config, domains: Option<DomainSet>) {
    let gate = DiscoveryGate::from_config(ctx, config, domains);
    if !gate.flow_authoring_enabled() {
        tracing::debug!(
            skills = config.components.skills,
            flows = config.components.flows,
            tinyflows = config.components.tinyflows,
            "[skills][bundled] flow-authoring prewarm skipped by capability gates"
        );
        return;
    }

    let workspace = config.workspace_dir.as_path();
    let key = snapshot_key(workspace, &gate);
    let lock = warmup_lock(&key);
    let _guard = match tokio::time::timeout(COPILOT_RESOURCE_DEADLINE, lock.lock()).await {
        Ok(guard) => guard,
        Err(_) => {
            tracing::warn!(
                timeout_seconds = COPILOT_RESOURCE_DEADLINE.as_secs(),
                "[skills][bundled] flow-authoring prewarm is already busy; skipping optional fetch"
            );
            return;
        }
    };
    if generation_if_current(workspace, &gate).is_some() {
        return;
    }
    prewarm_locked(workspace, &gate, &key, config).await;
}

async fn prewarm_locked(
    workspace: &Path,
    gate: &DiscoveryGate,
    key: &SnapshotKey,
    config: &Config,
) {
    let fetched = tokio::time::timeout(COPILOT_RESOURCE_DEADLINE, async {
        let proxy = crate::modules::tinyflows::copilot_proxy(config).await?;
        let member = tinyflows_bus::copilot::METHODS
            .iter()
            .copied()
            .find(|member| *member == "ListResources")
            .ok_or_else(|| "TinyFlows Copilot contract has no ListResources member".to_string())?;
        proxy
            .call::<Vec<ResourceFile>>(member, ())
            .await
            .map_err(|error| error.wire_message())
    })
    .await;

    let mut resources = match fetched {
        Ok(Ok(files)) => files,
        Ok(Err(error)) => {
            tracing::warn!(%error, "[skills][bundled] flow-authoring resources unavailable; hiding builtin");
            clear_snapshot(key);
            return;
        }
        Err(_) => {
            tracing::warn!(
                timeout_seconds = COPILOT_RESOURCE_DEADLINE.as_secs(),
                "[skills][bundled] flow-authoring resource prewarm timed out; hiding builtin"
            );
            clear_snapshot(key);
            return;
        }
    };
    resources.sort_by(|left, right| left.path.cmp(&right.path));

    if !module_is_serving() {
        tracing::warn!(
            "[skills][bundled] TinyFlows module is not serving after ListResources; hiding builtin"
        );
        clear_snapshot(key);
        return;
    }

    if let Err(error) = publish_snapshot(workspace, gate, resources) {
        tracing::warn!(%error, "[skills][bundled] flow-authoring materialization failed; hiding builtin");
        clear_snapshot(key);
    }
}

/// Return the current resource generation only while the owning module and
/// TinySkills-validated on-disk tree still agree.
pub(crate) fn generation_if_current(workspace: &Path, gate: &DiscoveryGate) -> Option<u64> {
    let key = snapshot_key(workspace, gate);
    if !module_is_serving() {
        clear_snapshot(&key);
        return None;
    }
    let snapshot = SNAPSHOTS
        .read()
        .ok()
        .and_then(|snapshots| snapshots.get(&key).cloned())?;
    if !materialization_matches(workspace, gate, &snapshot) {
        tracing::warn!(
            "[skills][bundled] cached flow-authoring materialization changed; hiding builtin"
        );
        clear_snapshot(&key);
        return None;
    }
    Some(snapshot.generation)
}

/// Load the dynamic skill for synchronous discovery after the metadata path
/// has checked the expected snapshot generation.
pub(crate) fn materialized_skill(
    workspace: &Path,
    gate: &DiscoveryGate,
    generation: u64,
) -> Option<Skill> {
    let key = snapshot_key(workspace, gate);
    let snapshot = snapshot_for(&key, generation)?;
    if !module_is_serving() || !materialization_matches(workspace, gate, &snapshot) {
        clear_snapshot(&key);
        return None;
    }
    let skill = tinyskills::load_skill_dir(
        &root(workspace, gate).join(FLOW_SKILL_ID),
        SkillScope::Builtin,
    )?;
    (skill.id() == FLOW_SKILL_ID).then_some(skill)
}

/// Read one resource only when the module lifecycle, complete resource
/// inventory, and returned bytes all still match the retained bus snapshot.
pub(crate) fn read_resource(
    workspace: &Path,
    gate: &DiscoveryGate,
    generation: u64,
    skill: &Skill,
    relative_path: &Path,
) -> Result<String, String> {
    let key = snapshot_key(workspace, gate);
    let snapshot = snapshot_for(&key, generation)
        .ok_or_else(|| "flow-authoring resources are unavailable".to_string())?;
    if !module_is_serving() || !materialization_matches(workspace, gate, &snapshot) {
        clear_snapshot(&key);
        return Err("flow-authoring resources are stale or changed".to_string());
    }
    let requested = relative_path
        .to_str()
        .ok_or_else(|| "resource path is not UTF-8".to_string())?;
    let expected = snapshot
        .files
        .iter()
        .find(|file| file.path == requested)
        .map(|file| file.contents.as_str())
        .ok_or_else(|| "resource is not present in the current TinyFlows snapshot".to_string())?;
    let content =
        tinyskills::read_resource(skill, relative_path).map_err(|error| error.to_string())?;
    if content != expected {
        clear_snapshot(&key);
        return Err("flow-authoring resource bytes changed".to_string());
    }
    Ok(content)
}

fn snapshot_for(key: &SnapshotKey, generation: u64) -> Option<Snapshot> {
    SNAPSHOTS
        .read()
        .ok()?
        .get(key)
        .filter(|snapshot| snapshot.generation == generation)
        .cloned()
}

fn module_is_serving() -> bool {
    crate::modules::ops::state_of(crate::modules::tinyflows::MODULE_ID)
        == crate::modules::types::ModuleState::Ready
        && crate::modules::runtime::is_module_serving(crate::modules::tinyflows::MODULE_ID)
}

fn valid_resource_manifest(files: &[ResourceFile]) -> bool {
    if files.is_empty() {
        return false;
    }
    let mut paths = BTreeSet::new();
    if files.iter().any(|file| !paths.insert(file.path.as_str())) {
        return false;
    }
    files
        .iter()
        .any(|file| file.path == tinyskills::WORKFLOW_MD || file.path == tinyskills::SKILL_MD)
}

fn materialize(
    workspace: &Path,
    gate: &DiscoveryGate,
    resources: &[ResourceFile],
) -> Result<(), String> {
    let parent_path = workspace.join(".openhuman").join(FLOW_ROOT_NAME);
    std::fs::create_dir_all(&parent_path)
        .map_err(|error| format!("creating builtin skill parent failed: {error}"))?;
    let parent = tinyskills::cap_std::fs::Dir::open_ambient_dir(
        &parent_path,
        tinyskills::cap_std::ambient_authority(),
    )
    .map_err(|error| format!("opening builtin skill parent failed: {error}"))?;
    let files = resources
        .iter()
        .map(|file| MaterializeFile::new(file.path.clone(), file.contents.as_bytes().to_vec()))
        .collect();
    let entry = MaterializeEntry {
        dir_name: FLOW_SKILL_ID.to_string(),
        source: MaterializeSource::Files(files),
    };
    tinyskills::materialize_tree(&parent, &scope_directory(gate), &[entry])
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn publish_snapshot(
    workspace: &Path,
    gate: &DiscoveryGate,
    resources: Vec<ResourceFile>,
) -> Result<u64, String> {
    if !valid_resource_manifest(&resources) {
        return Err("TinyFlows returned an invalid resource manifest".to_string());
    }
    let key = snapshot_key(workspace, gate);
    let previous = SNAPSHOTS
        .read()
        .ok()
        .and_then(|snapshots| snapshots.get(&key).cloned());
    if let Some(previous) = &previous {
        if !materialization_matches(workspace, gate, previous) {
            clear_snapshot(&key);
            return Err("flow-authoring disk tree changed".to_string());
        }
        if previous.files == resources {
            return Ok(previous.generation);
        }
    }

    let generation = NEXT_GENERATION.fetch_add(1, Ordering::SeqCst);
    materialize(workspace, gate, &resources)?;
    if let Ok(mut snapshots) = SNAPSHOTS.write() {
        snapshots.insert(
            key,
            Snapshot {
                generation,
                files: resources,
            },
        );
    }
    crate::skills::ops_discover::invalidate_workflow_metadata_cache();
    Ok(generation)
}

fn materialization_matches(workspace: &Path, gate: &DiscoveryGate, snapshot: &Snapshot) -> bool {
    let directory = root(workspace, gate).join(FLOW_SKILL_ID);
    let Some(skill) = tinyskills::load_skill_dir(&directory, SkillScope::Builtin) else {
        return false;
    };
    if skill.id() != FLOW_SKILL_ID || skill.scope != SkillScope::Builtin {
        return false;
    }

    let Some(manifest) = snapshot
        .files
        .iter()
        .find(|file| file.path == tinyskills::WORKFLOW_MD || file.path == tinyskills::SKILL_MD)
    else {
        return false;
    };
    let Some(location) = skill.location.as_deref() else {
        return false;
    };
    if tinyskills::read_document(location).ok().as_deref() != Some(manifest.contents.as_str()) {
        return false;
    }

    let expected_resources: Vec<PathBuf> = snapshot
        .files
        .iter()
        .filter(|file| file.path != manifest.path)
        .map(|file| PathBuf::from(&file.path))
        .collect();
    let mut actual_resources = skill.resources.clone();
    actual_resources.sort();
    if actual_resources != expected_resources {
        return false;
    }
    expected_resources.iter().all(|path| {
        let Some(expected) = snapshot
            .files
            .iter()
            .find(|file| Path::new(file.path.as_str()) == path.as_path())
            .map(|file| file.contents.as_str())
        else {
            return false;
        };
        tinyskills::read_resource(&skill, path).ok().as_deref() == Some(expected)
    })
}

fn snapshot_key(workspace: &Path, gate: &DiscoveryGate) -> SnapshotKey {
    SnapshotKey {
        workspace: workspace.to_path_buf(),
        gate: gate.clone(),
    }
}

fn scope_directory(gate: &DiscoveryGate) -> String {
    let encoded = serde_json::to_vec(gate).unwrap_or_default();
    hex::encode(sha2::Sha256::digest(encoded))
}

fn clear_snapshot(key: &SnapshotKey) {
    let removed = SNAPSHOTS
        .write()
        .is_ok_and(|mut snapshots| snapshots.remove(key).is_some());
    if removed {
        crate::skills::ops_discover::invalidate_workflow_metadata_cache();
    }
}

#[cfg(test)]
#[path = "flow_authoring_tests.rs"]
mod tests;
