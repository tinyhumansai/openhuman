//! `storage.status`: where a profile is in the layout move, read-only.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::state::{LayoutError, LayoutState, Step};
use super::{legacy, Target};
use crate::config::schema::profile_layout::LayoutVersion;
use crate::config::Config;

/// The schema version one owner has reached in the profile's database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerVersion {
    /// The schema owner (`threads`, `harness`, ...).
    pub owner: String,
    /// The highest version a finished step wrote.
    pub version: u32,
}

/// A profile's storage state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// `1` or `2`.
    pub layout_version: u8,
    /// `users/<id>`.
    pub profile_dir: String,
    /// The database file the profile uses under its layout.
    pub database: Option<String>,
    /// Its size in bytes, when it exists.
    pub database_bytes: Option<u64>,
    /// Schema version per owner, from `tsd.migrations`.
    pub schema_versions: Vec<OwnerVersion>,
    /// When the last migration step finished (ms since the epoch).
    pub last_migration_ms: Option<i64>,
    /// A layout migration was started and not finished.
    pub migration_in_progress: bool,
    /// The steps that finished, when one is in progress.
    pub migration_steps_done: Vec<Step>,
    /// An explicit storage URL names the backend, so the layout move does not
    /// apply (its credentials are not shown).
    pub explicit_url: bool,
}

/// Reads `target`'s status. Nothing is written: the database is opened raw
/// (no schema is prepared or migrated).
///
/// # Errors
///
/// An unreadable progress file. A database that cannot be read reports no
/// versions rather than failing.
pub async fn status(
    target: &Target,
    config: &Config,
    env_url: Option<String>,
) -> Result<Status, LayoutError> {
    let layout = LayoutVersion::detect(&target.profile_dir);
    let state = LayoutState::load(&target.profile_dir)?;
    let database = match layout {
        LayoutVersion::V2 => target.storage_db(),
        LayoutVersion::V1 => legacy::v1_db(target),
    };
    let meta = std::fs::metadata(&database).ok();
    let (schema_versions, last_migration_ms) = if meta.is_some() {
        read_versions(&database).await.unwrap_or_default()
    } else {
        (Vec::new(), None)
    };
    Ok(Status {
        layout_version: if layout == LayoutVersion::V2 { 2 } else { 1 },
        profile_dir: target.profile_dir.display().to_string(),
        database: meta.as_ref().map(|_| database.display().to_string()),
        database_bytes: meta.map(|m| m.len()),
        schema_versions,
        last_migration_ms,
        migration_in_progress: state.is_some(),
        migration_steps_done: state.map(|s| s.done).unwrap_or_default(),
        explicit_url: matches!(
            crate::storage::config::mode_from(env_url, config),
            crate::storage::config::StorageMode::Url(_)
        ),
    })
}

async fn read_versions(
    db: &std::path::Path,
) -> Result<(Vec<OwnerVersion>, Option<i64>), LayoutError> {
    use tinystoragedrivers::schema::Progress;
    use tinystoragedrivers::{DocumentStoreExt, Query, MIGRATIONS_COLLECTION};
    let url = crate::storage::StorageUrl::parse(&format!("sqlite:{}", db.display()))
        .map_err(|e| LayoutError::Other(anyhow::anyhow!("{e}")))?;
    let backend = tinystoragedrivers::open(&url)
        .await
        .map_err(|e| LayoutError::Other(anyhow::anyhow!("{e}")))?;
    let scoped = backend
        .for_scope(&crate::storage::Scope::local())
        .map_err(|e| LayoutError::Other(anyhow::anyhow!("{e}")))?;
    let rows = scoped
        .documents()
        .query_all(MIGRATIONS_COLLECTION, &Query::all())
        .await
        .map_err(|e| LayoutError::Other(anyhow::anyhow!("{e}")))?;
    let mut owners: BTreeMap<String, u32> = BTreeMap::new();
    let mut last = None::<i64>;
    for row in rows {
        let Ok(progress) = serde_json::from_value::<Progress>(row.doc) else {
            continue;
        };
        if progress.done && progress.to > 0 {
            let version = owners.entry(progress.owner).or_default();
            *version = (*version).max(progress.to);
            last = last.max(Some(row.meta.updated_ms).filter(|ms| *ms > 0));
        }
    }
    Ok((
        owners
            .into_iter()
            .map(|(owner, version)| OwnerVersion { owner, version })
            .collect(),
        last,
    ))
}
