use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use tinystoragedrivers_types::CollectionSpec;

use super::*;

pub(crate) const IMPORT_DOMAIN: &str = "scope_move";
const IMPORT_OWNER: &str = "storage";

fn marker_collections() -> Vec<CollectionSpec> {
    crate::storage::import_state::collection_specs()
}

/// Selects the historical v1 scope database from the configured profile or
/// workspace layout and imports its inventory through the normal boot/rebind
/// importer pipeline. The destination backend is never used to discover
/// source scopes.
///
/// A profile-layout source is `<profile>/workspace/storage/storage.db`, as
/// selected by [`crate::storage::layout::legacy::v1_db`]. An unprofiled v1
/// workspace uses the default backend's directory database at
/// `<workspace>/storage/storage.db`.
#[cfg(feature = "legacy-import")]
pub fn run_from_config(
    backend: &Arc<dyn StorageBackend>,
    config: &crate::config::Config,
) -> crate::storage::import::Report {
    let result = (|| {
        let source = legacy_scope_source(config);
        if !source.is_file() {
            return Ok(crate::storage::import::Outcome::NoSource);
        }

        let known = known_scope_ids(config)?;
        let path = source.clone();
        let inventory = crate::storage::import::scope_move::LegacyScopeInventory::read(&path)
            .with_context(|| format!("read legacy scope inventory {}", path.display()))?;
        import_inventory(backend, config, &source, &known, &inventory)
    })();

    crate::storage::import::Report {
        domain: IMPORT_DOMAIN,
        outcome: result,
    }
}

#[cfg(not(feature = "legacy-import"))]
pub fn run_from_config(
    _backend: &Arc<dyn StorageBackend>,
    _config: &crate::config::Config,
) -> crate::storage::import::Report {
    crate::storage::import::Report {
        domain: IMPORT_DOMAIN,
        outcome: Err(anyhow::anyhow!(
            "legacy scope inventory import requires the `legacy-import` feature"
        )),
    }
}

#[cfg(feature = "legacy-import")]
fn legacy_scope_source(config: &crate::config::Config) -> PathBuf {
    if let Some(target) = crate::storage::layout::Target::from_config(config) {
        return crate::storage::layout::legacy::v1_db(&target);
    }
    config
        .workspace_dir
        .join(crate::storage::config::DEFAULT_DIR)
        .join("storage.db")
}

#[cfg(feature = "legacy-import")]
fn known_scope_ids(config: &crate::config::Config) -> anyhow::Result<Known> {
    let mut profiles = Vec::new();
    if let Some(target) = crate::storage::layout::Target::from_config(config) {
        profiles.push(target.user_id);
    }
    if let Some(host) = crate::profiles::host::host() {
        let hosted = crate::storage::block_on_anyhow(async move {
            host.known_profiles().await.map_err(anyhow::Error::msg)
        })?;
        profiles.extend(hosted);
    }
    profiles.sort();
    profiles.dedup();

    let agents = crate::agent_registry::host_definition::AgentDefinitionRegistry::current()
        .map(|registry| {
            registry
                .list()
                .into_iter()
                .map(|definition| definition.id.clone())
                .collect()
        })
        .unwrap_or_default();
    let modules = crate::modules::registry::ALL
        .iter()
        .map(|module| module.id.to_owned())
        .collect();
    Ok(Known {
        agents,
        profiles,
        modules,
    })
}

#[cfg(feature = "legacy-import")]
fn import_inventory(
    backend: &Arc<dyn StorageBackend>,
    config: &crate::config::Config,
    source_path: &Path,
    known: &Known,
    inventory: &crate::storage::import::scope_move::LegacyScopeInventory,
) -> anyhow::Result<crate::storage::import::Outcome> {
    let mut report = MoveReport::default();
    let mut scopes = inventory.scopes.clone();
    scopes.extend(inventory.documents.iter().map(|row| row.scope.clone()));
    scopes.extend(inventory.streams.iter().map(|row| row.scope.clone()));
    scopes.extend(inventory.blobs.iter().map(|row| row.scope.clone()));
    scopes.sort();
    scopes.dedup();
    if scopes.is_empty() {
        return Ok(crate::storage::import::Outcome::Imported {
            found: 0,
            written: 0,
        });
    }

    let mut all_already_done = true;
    let mut in_progress = 0usize;
    let mut found = 0usize;
    let mut written = 0usize;
    for source_scope in scopes {
        let target_scope = match map_scope(&source_scope, known) {
            Mapping::Move(scope) => scope,
            Mapping::V2 => Scope::new(&source_scope)?,
            Mapping::Unknown => {
                report.unknown.push(source_scope);
                all_already_done = false;
                continue;
            }
        };
        let (scope_backend, profile_guard) =
            backend_for_target_scope(backend, &target_scope, known, config, source_path)?;
        let marker = crate::storage::documents::Repo::on(
            &scope_backend,
            &target_scope,
            "storage::scope_move",
            marker_collections,
        )?;
        let step = scope_move_step(source_path, &source_scope);
        if crate::storage::import_state::is_done(&marker, IMPORT_OWNER, &step)? {
            report.already_done += 1;
            let scope_found = inventory_scope_counts(inventory, &source_scope);
            found += scope_found;
            continue;
        }

        all_already_done = false;
        let scope_inventory = inventory_for_scope(inventory, &source_scope);
        let scope_known = known.clone();
        let scope_backend_for_claim = Arc::clone(&scope_backend);
        let marker_for_claim = marker.clone();
        let step_for_claim = step.clone();
        let result = crate::storage::import_state::with_claim(
            &marker,
            IMPORT_OWNER,
            &step,
            move |condition| {
                // Keep the admitted profile handle alive until all copied
                // documents, streams, blobs, and the completion marker finish.
                let _profile_guard = profile_guard;
                let claimed_marker = marker_for_claim.conditioned(condition)?;
                if crate::storage::import_state::is_done(
                    &claimed_marker,
                    IMPORT_OWNER,
                    &step_for_claim,
                )? {
                    return Ok(None);
                }
                let claim_bound = crate::storage::import_state::backend_with_condition(
                    scope_backend_for_claim,
                    condition.clone(),
                );
                let copied = crate::storage::block_on_anyhow(async move {
                    copy_inventory(&claim_bound, &scope_known, &scope_inventory).await
                })?;
                crate::storage::import_state::set_done(
                    &claimed_marker,
                    IMPORT_OWNER,
                    &step_for_claim,
                )?;
                Ok(Some(copied))
            },
        )?;

        match result {
            None => in_progress += 1,
            Some(None) => {
                report.already_done += 1;
                found += inventory_scope_counts(inventory, &source_scope);
            }
            Some(Some(scope_report)) => {
                remigrate_copied_scope(&scope_backend, &target_scope);
                let scope_found = inventory_scope_counts(inventory, &source_scope);
                found += scope_found;
                written += usize::try_from(
                    scope_report.documents + scope_report.entries + scope_report.blobs,
                )
                .unwrap_or(usize::MAX);
                report.moved += scope_report.moved;
                report.documents += scope_report.documents;
                report.entries += scope_report.entries;
                report.blobs += scope_report.blobs;
                report.updated_times_not_restored += scope_report.updated_times_not_restored;
            }
        }
    }

    if !report.unknown.is_empty() {
        return Ok(crate::storage::import::Outcome::Incomplete {
            found,
            written,
            skipped: report.unknown.len(),
        });
    }
    if in_progress > 0 {
        if all_already_done {
            return Ok(crate::storage::import::Outcome::InProgress);
        }
        return Ok(crate::storage::import::Outcome::Incomplete {
            found,
            written,
            skipped: in_progress,
        });
    }
    if all_already_done {
        Ok(crate::storage::import::Outcome::AlreadyDone)
    } else {
        Ok(crate::storage::import::Outcome::Imported { found, written })
    }
}

/// The copy wrote v1 graph and harness records into a scope whose owner
/// migrations already ran when it was prepared; run them again so the v2
/// readers see the records. Best effort: a failure is logged, and the cleared
/// steps re-run on the next preparation of the scope.
#[cfg(feature = "legacy-import")]
fn remigrate_copied_scope(backend: &Arc<dyn StorageBackend>, scope: &Scope) {
    let result = backend.for_scope(scope).and_then(|storage| {
        let target = scope.to_storage_scope();
        crate::storage::block_on(async move {
            crate::storage::boot::remigrate_owners(&storage, &target).await
        })
    });
    if let Err(error) = result {
        tracing::warn!(
            target: "openhuman::storage",
            scope = scope.as_str(),
            %error,
            "[storage][scope_move] owner migration after the copy failed; retried when the scope is next prepared"
        );
    }
}

#[cfg(feature = "legacy-import")]
type BackendForScope = (
    Arc<dyn StorageBackend>,
    Option<Arc<crate::profiles::host::Profile>>,
);

#[cfg(feature = "legacy-import")]
fn backend_for_target_scope(
    backend: &Arc<dyn StorageBackend>,
    target_scope: &Scope,
    known: &Known,
    config: &crate::config::Config,
    source_path: &Path,
) -> anyhow::Result<BackendForScope> {
    let parsed = target_scope.parsed()?;
    if matches!(
        parsed.tenant,
        crate::storage::Tenant::Local | crate::storage::Tenant::Cluster
    ) {
        return Ok((Arc::clone(backend), None));
    }
    let crate::storage::Tenant::User(_) = parsed.tenant else {
        anyhow::bail!(
            "scope {} is not an admitted tenant scope",
            target_scope.as_str()
        );
    };
    let original_id = known
        .profiles
        .iter()
        .find(|id| {
            crate::storage::scope_for_profile(id).tenant_scope() == target_scope.tenant_scope()
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no known profile id maps to scope {}",
                target_scope.as_str()
            )
        })?;
    // During the profile layout migration, the importer is deliberately
    // running against that profile's v1 database. Its own u:<profile> rows
    // must be copied within this source before the database is checkpointed
    // and moved to the v2 location. The profile migration lock is held by the
    // caller, so this explicit source backend is the admitted destination for
    // this one transition.
    if let Some(target) = crate::storage::layout::Target::from_config(config) {
        if target.user_id == *original_id
            && source_path == crate::storage::layout::legacy::v1_db(&target)
        {
            return Ok((Arc::clone(backend), None));
        }
    }
    let Some(host) = crate::profiles::host::host() else {
        if let Some(target) = crate::storage::layout::Target::from_config(config) {
            let expected_url = format!("sqlite:{}", target.storage_db().display());
            let installed = crate::storage::installed();
            let backend_is_installed = installed
                .as_ref()
                .is_some_and(|installed| Arc::ptr_eq(installed, backend));
            if target.user_id == *original_id
                && crate::storage::configured_url(config).as_deref() == Some(&expected_url)
                && crate::storage::installed_url().as_deref() == Some(&expected_url)
                && backend_is_installed
            {
                return Ok((Arc::clone(backend), None));
            }
        }
        anyhow::bail!(
            "cannot copy profile scope {} without its admitted profile fence or dedicated profile database",
            target_scope.as_str()
        );
    };
    let profile_id = crate::profiles::ProfileId::parse(original_id).map_err(anyhow::Error::msg)?;
    let profile = host.get(&profile_id).ok_or_else(|| {
        anyhow::anyhow!(
            "profile {original_id} is not currently admitted on this host; refusing scope copy"
        )
    })?;
    let scoped = host
        .storage_backend_for(&profile)?
        .ok_or_else(|| anyhow::anyhow!("profile {original_id} has no captured storage backend"))?;
    Ok((scoped, Some(profile)))
}

#[cfg(feature = "legacy-import")]
fn scope_move_step(source_path: &Path, source_scope: &str) -> String {
    use sha2::Digest;
    let source_hash = sha2::Sha256::digest(source_path.as_os_str().as_encoded_bytes());
    let scope_hash = sha2::Sha256::digest(source_scope.as_bytes());
    format!(
        "scope_move/{}/{}",
        hex::encode(&source_hash[..8]),
        hex::encode(&scope_hash[..8])
    )
}

#[cfg(feature = "legacy-import")]
fn inventory_for_scope(
    inventory: &crate::storage::import::scope_move::LegacyScopeInventory,
    source_scope: &str,
) -> crate::storage::import::scope_move::LegacyScopeInventory {
    crate::storage::import::scope_move::LegacyScopeInventory {
        scopes: vec![source_scope.to_owned()],
        documents: inventory
            .documents
            .iter()
            .filter(|row| row.scope == source_scope)
            .cloned()
            .collect(),
        streams: inventory
            .streams
            .iter()
            .filter(|row| row.scope == source_scope)
            .cloned()
            .collect(),
        blobs: inventory
            .blobs
            .iter()
            .filter(|row| row.scope == source_scope)
            .cloned()
            .collect(),
    }
}

#[cfg(feature = "legacy-import")]
fn inventory_scope_counts(
    inventory: &crate::storage::import::scope_move::LegacyScopeInventory,
    source_scope: &str,
) -> usize {
    inventory
        .documents
        .iter()
        .filter(|row| row.scope == source_scope)
        .count()
        + inventory
            .streams
            .iter()
            .filter(|row| row.scope == source_scope)
            .map(|row| row.entries.len())
            .sum::<usize>()
        + inventory
            .blobs
            .iter()
            .filter(|row| row.scope == source_scope)
            .count()
}
