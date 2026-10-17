//! The one-time move of v1 scopes to the one scope grammar.
//!
//! v1 addressed one tenant five ways: a raw agent id (hashed to `sha256:<hex>`
//! when it was not a valid scope), `<profile>~<agent>` for a SaaS session,
//! `profile:<id>` / `profile-sha256:<hex>` for domain stores,
//! `module:<id>@<base>` / `<profile>~module:<id>` / `module-sha256:<hex>` for
//! loadable modules, and `default` for the agent-less session. v2 has
//! `local`, `u:<id>` and a principal under either (`/agent:<id>`,
//! `/module:<id>`), see [`super::scope`].
//!
//! Owner migration steps run inside one scope, so they cannot move a record
//! between scopes. The host does it first, here: for each v1 scope found by
//! `StorageBackend::scopes`, copy every document (with its schema, creation
//! time and expiry), stream (and its sealed state) and blob into the v2 scope
//! it maps to, then remove the v1 copy. The owners' steps then rewrite names
//! and shapes inside the new scope.
//!
//! * **Resumable.** A document is written only when absent and removed from
//!   the source after its collection is copied, a stream continues from the
//!   target's length, a blob is skipped when the target already holds it, and
//!   the source scope is deleted last. A crash re-runs the same copy to the same
//!   result. Completion is recorded in the target scope's `tsd.migrations`
//!   (owner `host`, step `scope_move:<source scope>`).
//! * **Merging.** Several v1 scopes can map to one v2 scope (the
//!   `module:<id>@<base>` of each acting agent all become the tenant's
//!   `module:<id>`). A record already present in the target wins.
//! * **Hashed scopes.** A `sha256:` / `profile-sha256:` / `module-sha256:` v1
//!   scope cannot be inverted. It maps only when a known agent, profile or
//!   module id hashes to it (the recorded agents, the profile registry and the
//!   module registry); any other is left in place and logged.

use std::collections::HashSet;
use std::sync::Arc;

use sha2::Digest;
use tinystoragedrivers::schema::Migrations;
use tinystoragedrivers::{
    Capability, DocumentStore, DocumentStoreExt, ErrorKind, Filter, Precondition, Query, Scope,
    ScopedStorage, StorageBackend, StorageError, WriteOptions,
};

use super::scope::{scope_for_agent, scope_for_profile, tenant_scope};

/// The `tsd.migrations` owner the move records under.
pub const MIGRATION_OWNER: &str = "host";

const PAGE: usize = 200;
const STREAM_PAGE: usize = 500;

/// The ids a hashed v1 scope can be matched against.
#[derive(Debug, Default, Clone)]
pub struct Known {
    /// Agent ids recorded by earlier processes ([`super::agents`]).
    pub agents: Vec<String>,
    /// Provisioned profile ids (the profile registry).
    pub profiles: Vec<String>,
    /// Registry module ids.
    pub modules: Vec<String>,
}

/// What a scope string maps to.
#[derive(Debug, PartialEq, Eq)]
pub enum Mapping {
    /// Already in the v2 grammar; nothing to move.
    V2,
    /// Move everything here.
    Move(Scope),
    /// A hashed or malformed v1 scope no known id explains.
    Unknown,
}

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

fn unescape_profile(escaped: &str) -> String {
    escaped.replace("%7E", "~").replace("%25", "%")
}

fn module_scope(profile: Option<&str>, id: &str) -> Mapping {
    match tenant_scope(profile).module(id) {
        Ok(scope) => Mapping::Move(scope),
        Err(_) => Mapping::Unknown,
    }
}

/// The v2 scope the v1 scope `src` maps to.
pub fn map_scope(src: &str, known: &Known) -> Mapping {
    if Scope::parse(src).is_ok() {
        return Mapping::V2;
    }
    if src == "default" {
        return Mapping::Move(scope_for_agent("default"));
    }
    if let Some(id) = src.strip_prefix("profile:") {
        return Mapping::Move(scope_for_profile(id));
    }
    if let Some(hex) = src.strip_prefix("profile-sha256:") {
        return known
            .profiles
            .iter()
            .find(|id| sha_hex(id.as_bytes()) == hex)
            .map_or(Mapping::Unknown, |id| Mapping::Move(scope_for_profile(id)));
    }
    if let Some(hex) = src.strip_prefix("module-sha256:") {
        // sha256(module id, 0, base scope); the base is `local` or an agent.
        let bases: Vec<&str> = std::iter::once("local")
            .chain(known.agents.iter().map(String::as_str))
            .collect();
        for id in &known.modules {
            for base in &bases {
                let mut digest = sha2::Sha256::new();
                digest.update(id.as_bytes());
                digest.update([0]);
                digest.update(base.as_bytes());
                if hex::encode(digest.finalize()) == hex {
                    return module_scope(None, id);
                }
            }
        }
        return Mapping::Unknown;
    }
    if let Some(hex) = src.strip_prefix("sha256:") {
        return known
            .agents
            .iter()
            .find(|id| sha_hex(id.as_bytes()) == hex)
            .map_or(Mapping::Unknown, |id| Mapping::Move(scope_for_agent(id)));
    }
    if let Some(rest) = src.strip_prefix("module:") {
        // `module:<id>@<base>`: modules are tenant-level, whichever agent was acting.
        let id = rest.rsplit_once('@').map_or(rest, |(id, _)| id);
        return module_scope(None, id);
    }
    if let Some((profile, agent)) = src.split_once('~') {
        let profile = unescape_profile(profile);
        if let Some(module) = agent.strip_prefix("module:") {
            return module_scope(Some(&profile), module);
        }
        let agent = if agent.is_empty() { "default" } else { agent };
        return match scope_for_profile(&profile).agent(agent) {
            Ok(scope) => Mapping::Move(scope),
            Err(_) => Mapping::Unknown,
        };
    }
    if src == "cluster" {
        return Mapping::V2;
    }
    // Any other literal v1 scope is a raw agent id.
    Mapping::Move(scope_for_agent(src))
}

/// What a run did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MoveReport {
    /// Source scopes moved to a v2 scope.
    pub moved: usize,
    /// Source scopes already recorded as moved.
    pub already_done: usize,
    /// v1 scopes no known id maps; left in place.
    pub unknown: Vec<String>,
    /// Documents copied.
    pub documents: u64,
    /// Stream entries copied.
    pub entries: u64,
    /// Blobs copied.
    pub blobs: u64,
}

/// Reads the ids hashed scopes are matched against.
pub async fn known_ids(backend: &Arc<dyn StorageBackend>) -> Known {
    let mut known = Known::default();
    if let Ok(local) = backend.for_scope(&Scope::local()) {
        if let Ok(found) = ids_of(local.documents(), super::agents::AGENTS).await {
            known.agents = found;
        }
    }
    if let Ok(cluster) = backend.for_scope(&Scope::cluster()) {
        if let Ok(found) = ids_of(cluster.documents(), "profiles").await {
            known.profiles = found;
        }
    }
    #[cfg(feature = "modules")]
    {
        known.modules = crate::modules::registry::ALL
            .iter()
            .map(|record| record.id.to_string())
            .collect();
    }
    known
}

async fn ids_of(
    docs: &Arc<dyn DocumentStore>,
    collection: &str,
) -> Result<Vec<String>, StorageError> {
    // Absent collections read as empty; only names that exist are listed.
    match docs.collections(collection).await {
        Ok(found) if !found.iter().any(|name| name == collection) => return Ok(Vec::new()),
        Ok(_) | Err(_) => {}
    }
    Ok(docs
        .query_all(collection, &Query::all())
        .await?
        .into_iter()
        .map(|doc| doc.id)
        .collect())
}

/// Moves every v1 scope of `backend` to its v2 scope. See the module docs.
///
/// # Errors
///
/// The first storage error. Nothing is lost: re-running resumes.
pub async fn run(backend: &Arc<dyn StorageBackend>) -> Result<MoveReport, StorageError> {
    let mut report = MoveReport::default();
    if !backend
        .capabilities()
        .contains(Capability::ScopeEnumeration)
    {
        tracing::debug!(
            target: "openhuman::storage",
            driver = backend.driver(),
            "[storage][scope-move] backend cannot enumerate scopes; nothing to move"
        );
        return Ok(report);
    }
    let sources = backend.scopes("").await?;
    let candidates: Vec<&Scope> = sources
        .iter()
        .filter(|scope| scope.parsed().is_err())
        .collect();
    if candidates.is_empty() {
        return Ok(report);
    }
    let known = known_ids(backend).await;
    for src in candidates {
        match map_scope(src.as_str(), &known) {
            Mapping::V2 => {}
            Mapping::Unknown => {
                tracing::warn!(
                    target: "openhuman::storage",
                    scope = src.as_str(),
                    "[storage][scope-move] no known id maps this v1 scope; left in place"
                );
                report.unknown.push(src.as_str().to_owned());
            }
            Mapping::Move(target) => {
                if move_scope(backend, src, &target, &mut report).await? {
                    report.moved += 1;
                } else {
                    report.already_done += 1;
                }
            }
        }
    }
    tracing::info!(
        target: "openhuman::storage",
        moved = report.moved,
        documents = report.documents,
        entries = report.entries,
        blobs = report.blobs,
        unknown = report.unknown.len(),
        "[storage][scope-move] finished"
    );
    Ok(report)
}

fn step_id(src: &Scope) -> String {
    format!("scope_move:{}", src.as_str())
}

/// Moves one scope. `false` when it was already recorded as moved.
async fn move_scope(
    backend: &Arc<dyn StorageBackend>,
    src: &Scope,
    target: &Scope,
    report: &mut MoveReport,
) -> Result<bool, StorageError> {
    let to = backend.for_scope(target)?;
    let migrations = Migrations::new(&to);
    let step = step_id(src);
    to.documents()
        .ensure_collection(&tinystoragedrivers::CollectionSpec::new(
            tinystoragedrivers::MIGRATIONS_COLLECTION,
        ))
        .await?;
    if migrations.done(MIGRATION_OWNER, &step).await? {
        // The copy finished earlier; make sure no source records remain.
        let from = backend.for_scope(src)?;
        purge(&from).await?;
        return Ok(false);
    }
    tracing::info!(
        target: "openhuman::storage",
        from = src.as_str(),
        to = target.as_str(),
        "[storage][scope-move] moving a v1 scope"
    );
    let from = backend.for_scope(src)?;
    copy_documents(&from, &to, report).await?;
    copy_streams(&from, &to, report).await?;
    copy_blobs(&from, &to, report).await?;
    migrations.mark_done(MIGRATION_OWNER, &step).await?;
    purge(&from).await?;
    Ok(true)
}

async fn copy_documents(
    from: &ScopedStorage,
    to: &ScopedStorage,
    report: &mut MoveReport,
) -> Result<(), StorageError> {
    let source = from.documents();
    let target = to.documents();
    let metadata = source.capabilities().contains(Capability::RecordMetadata)
        && target.capabilities().contains(Capability::RecordMetadata);
    for collection in source.collections("").await? {
        target
            .ensure_collection(&tinystoragedrivers::CollectionSpec::new(&collection))
            .await?;
        let mut cursor = None;
        loop {
            let mut query = Query::filter(Filter::default())
                .sort(tinystoragedrivers::Sort::asc(tinystoragedrivers::ID_FIELD))
                .limit(PAGE);
            if let Some(after) = cursor.take() {
                query = query.after(after);
            }
            let page = source.query(&collection, &query).await?;
            for doc in page.items {
                let options = if metadata {
                    let mut options = WriteOptions::new().created_ms(doc.meta.created_ms);
                    options.schema = doc.meta.schema;
                    options.expires_ms = doc.meta.expires_ms;
                    options
                } else {
                    WriteOptions::new()
                };
                match target
                    .put_with(&collection, &doc.id, doc.doc, Precondition::Absent, options)
                    .await
                {
                    Ok(_) => report.documents += 1,
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::Conflict | ErrorKind::AlreadyExists
                        ) => {}
                    Err(error) => return Err(error),
                }
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
    }
    Ok(())
}

async fn copy_streams(
    from: &ScopedStorage,
    to: &ScopedStorage,
    report: &mut MoveReport,
) -> Result<(), StorageError> {
    let source = from.streams();
    let target = to.streams();
    for name in source.streams("").await? {
        let info = source.info(&name).await?;
        let base = info.as_ref().map_or(0, |info| info.base);
        let mut offset = base + target.len(&name).await?;
        loop {
            let window = source.read_window(&name, offset, STREAM_PAGE).await?;
            if window.is_empty() {
                break;
            }
            offset = window.last().map_or(offset, |entry| entry.offset + 1);
            let count = window.len() as u64;
            target
                .append_batch(&name, window.into_iter().map(|entry| entry.value).collect())
                .await?;
            report.entries += count;
        }
        if info.is_some_and(|info| info.sealed_ms.is_some()) {
            match target.seal(&name).await {
                Ok(_) => {}
                Err(error) if matches!(error.kind(), ErrorKind::Unsupported(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

async fn copy_blobs(
    from: &ScopedStorage,
    to: &ScopedStorage,
    report: &mut MoveReport,
) -> Result<(), StorageError> {
    let source = from.blobs();
    let target = to.blobs();
    for meta in source.list("").await? {
        if target.head(&meta.key).await?.is_none() {
            let Some(blob) = source.get(&meta.key).await? else {
                continue;
            };
            target
                .put(&meta.key, blob.bytes, meta.content_type.as_deref())
                .await?;
            report.blobs += 1;
        }
    }
    Ok(())
}

/// Removes every record of a moved scope.
async fn purge(from: &ScopedStorage) -> Result<(), StorageError> {
    let docs = from.documents();
    for collection in docs.collections("").await? {
        docs.drop_collection(&collection).await?;
    }
    let streams = from.streams();
    for name in streams.streams("").await? {
        streams.delete_stream(&name).await?;
    }
    let blobs = from.blobs();
    for meta in blobs.list("").await? {
        blobs.delete(&meta.key).await?;
    }
    Ok(())
}

/// The scopes of `scopes` that are not in the v2 grammar (for tests and the
/// boot log).
pub fn v1_scopes(scopes: &[Scope]) -> HashSet<&str> {
    scopes
        .iter()
        .filter(|scope| scope.parsed().is_err())
        .map(Scope::as_str)
        .collect()
}

#[cfg(test)]
#[path = "scope_move_tests.rs"]
mod tests;
