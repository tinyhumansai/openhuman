//! The curated Composio catalogs, read from the connector module.
//!
//! The tables live in `tinyconnectors` and are served over its bus contract
//! (`ListCuratedCatalog`, `GetVocabulary`). The host's lookups are synchronous
//! and sit on hot paths (tool visibility, flow gates), so this keeps one
//! process-wide snapshot of the module's answer. The snapshot is filled by
//! [`prime`] (an async call to the module) and dropped by [`invalidate`] when
//! the module is re-initialised or its pinned version changes.
//!
//! Until the snapshot is filled — the module is not loaded yet, or this build
//! has no module loader — every toolkit reads as uncurated, which routes the
//! caller to its classify-by-verb fallback rather than hiding every action.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use tinyconnectors_bus::names::methods;
use tinyconnectors_bus::vocabulary::{
    CuratedCatalogRequest, CuratedTool, ToolScope, VocabularyRequest, VocabularyResponse,
};

use tinyconnectors_bus::vocabulary::{
    classify_unknown, find_curated, toolkit_from_slug, UserScopePref,
};

const LOG: &str = "[composio][curated]";

/// Description for a toolkit the module has no blurb for, used while the
/// snapshot is empty.
pub const FALLBACK_DESCRIPTION: &str =
    "Interact with this connected service via its available actions";

/// Spellings callers use that are not the module's canonical toolkit slug.
/// They are resolved by the module (`GetVocabulary` matches aliases), not here;
/// listing them only tells [`prime`] which extra names to ask about.
const ALIAS_SPELLINGS: &[&str] = &[
    "google_calendar",
    "google_drive",
    "google_docs",
    "google_sheets",
    "microsoft",
    "one",
    "onedrive",
    "larksuite",
];

#[derive(Default)]
struct Snapshot {
    /// Module version the snapshot was read from.
    module_version: String,
    /// Lowercased toolkit name or alias -> curated catalog.
    catalogs: HashMap<String, Arc<[CuratedTool]>>,
    descriptions: HashMap<String, &'static str>,
    native: HashSet<String>,
    /// Canonical toolkits with a catalog, in the module's order.
    toolkits: Vec<String>,
    /// The module's agent-ready toolkit list, sorted.
    agent_ready: Vec<String>,
}

fn store() -> &'static RwLock<Option<Arc<Snapshot>>> {
    static STORE: RwLock<Option<Arc<Snapshot>>> = RwLock::new(None);
    &STORE
}

fn current() -> Option<Arc<Snapshot>> {
    let installed = store()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    #[cfg(test)]
    if installed.is_none() {
        return Some(test_fixture());
    }
    installed
}

/// What the module serves, frozen from its own fixtures
/// (`tinyconnectors-sync/src/toolkits/catalog/fixtures`), so unit tests that
/// read catalogs do not need a loaded module.
#[cfg(test)]
fn test_fixture() -> Arc<Snapshot> {
    use tinyconnectors_bus::vocabulary::{ToolkitVocabulary, VocabularyAction};
    static FIXTURE: std::sync::OnceLock<Arc<Snapshot>> = std::sync::OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            #[derive(serde::Deserialize)]
            struct Row {
                slug: String,
                scope: ToolScope,
            }
            let actions: HashMap<String, Vec<Row>> =
                serde_json::from_str(include_str!("curated_fixtures/catalog-actions.json"))
                    .expect("catalog fixture");
            let descriptions: HashMap<String, String> =
                serde_json::from_str(include_str!("curated_fixtures/descriptions.json"))
                    .expect("description fixture");
            let native = ["gmail", "notion", "slack", "clickup", "github", "linear"];
            let toolkits = descriptions
                .iter()
                .map(|(name, description)| ToolkitVocabulary {
                    toolkit: name.clone(),
                    description: description.clone(),
                    native_provider: native.contains(&name.as_str()),
                    actions: actions.get(name).map(|rows| {
                        rows.iter()
                            .map(|row| VocabularyAction {
                                slug: row.slug.clone(),
                                scope: row.scope,
                            })
                            .collect()
                    }),
                })
                .collect();
            let vocabulary = VocabularyResponse {
                toolkits,
                agent_ready_toolkits: vec![
                    "github".to_owned(),
                    "gmail".to_owned(),
                    "linear".to_owned(),
                    "notion".to_owned(),
                    "slack".to_owned(),
                ],
            };
            Arc::new(build("fixture", &[], &vocabulary))
        })
        .clone()
}

/// Leak each distinct description once: `toolkit_description` hands out `&'static str`, and a
/// re-read after reinit must not grow the heap by another copy of the tables.
fn intern(text: &str) -> &'static str {
    static POOL: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut pool = POOL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let pool = pool.get_or_insert_with(HashSet::new);
    if let Some(found) = pool.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
    pool.insert(leaked);
    leaked
}

fn key(toolkit: &str) -> String {
    toolkit.trim().to_ascii_lowercase()
}

/// Build a snapshot from the module's two replies. Pure, so it is tested
/// without a loaded module.
fn build(
    module_version: &str,
    listed: &[CuratedTool],
    vocabulary: &VocabularyResponse,
) -> Snapshot {
    let mut snapshot = Snapshot {
        module_version: module_version.to_owned(),
        ..Snapshot::default()
    };
    for tool in listed {
        if !snapshot.toolkits.contains(&tool.toolkit) {
            snapshot.toolkits.push(tool.toolkit.clone());
        }
    }
    for entry in &vocabulary.toolkits {
        let name = key(&entry.toolkit);
        snapshot
            .descriptions
            .insert(name.clone(), intern(&entry.description));
        if entry.native_provider {
            snapshot.native.insert(name.clone());
        }
        if let Some(actions) = &entry.actions {
            let catalog: Arc<[CuratedTool]> = actions
                .iter()
                .map(|action| CuratedTool {
                    toolkit: entry.toolkit.clone(),
                    slug: action.slug.clone(),
                    scope: action.scope,
                })
                .collect();
            snapshot.catalogs.insert(name, catalog);
        }
    }
    snapshot.agent_ready = vocabulary.agent_ready_toolkits.clone();
    snapshot.agent_ready.sort_unstable();
    snapshot.agent_ready.dedup();
    snapshot
}

/// Registry id of the connector module.
const MODULE_ID: &str = "tinyconnectors";

#[cfg(feature = "modules")]
async fn module_call<Request, Reply>(
    config: &crate::config::Config,
    member: &str,
    request: Request,
) -> Result<Reply, String>
where
    Request: serde::Serialize + Send,
    Reply: serde::de::DeserializeOwned,
{
    crate::modules::connectors::call_stateless(config, member, request).await
}

/// Without the module loader there is no catalog to read.
#[cfg(not(feature = "modules"))]
async fn module_call<Request, Reply>(
    _config: &crate::config::Config,
    member: &str,
    _request: Request,
) -> Result<Reply, String>
where
    Request: serde::Serialize + Send,
    Reply: serde::de::DeserializeOwned,
{
    Err(format!("{member}: this build has no module loader"))
}

/// Read the catalogs from the connector module into the process snapshot.
/// A snapshot read from the currently pinned module version is kept as is.
///
/// # Errors
///
/// The module could not be reached; the previous snapshot (if any) stays.
pub async fn prime(config: &crate::config::Config) -> Result<(), String> {
    let version = crate::modules::registry::find(MODULE_ID)
        .map(|record| record.version.to_owned())
        .unwrap_or_default();
    if current().is_some_and(|snapshot| snapshot.module_version == version) {
        return Ok(());
    }
    let listed: Vec<CuratedTool> = module_call(
        config,
        methods::LIST_CURATED_CATALOG,
        CuratedCatalogRequest { toolkit: None },
    )
    .await?;
    let mut names: Vec<String> = Vec::new();
    for tool in &listed {
        if !names.contains(&tool.toolkit) {
            names.push(tool.toolkit.clone());
        }
    }
    names.extend(ALIAS_SPELLINGS.iter().map(|alias| (*alias).to_owned()));
    let vocabulary: VocabularyResponse = module_call(
        config,
        methods::GET_VOCABULARY,
        VocabularyRequest { toolkits: names },
    )
    .await?;
    let snapshot = build(&version, &listed, &vocabulary);
    tracing::debug!(
        "{LOG} primed toolkits={} module_version={version}",
        snapshot.toolkits.len()
    );
    *store()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(snapshot));
    Ok(())
}

/// Drop the snapshot so the next [`prime`] re-reads it (module reinit).
pub fn invalidate() {
    *store()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    tracing::debug!("{LOG} invalidated");
}

/// Replace the snapshot directly. Test seam for callers that exercise the
/// host adapters without a loaded module.
#[cfg(test)]
pub(crate) fn install_for_test(listed: &[CuratedTool], vocabulary: &VocabularyResponse) {
    *store()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(Arc::new(build("test", listed, vocabulary)));
}

/// The curated catalog for `toolkit` (an action prefix or any known alias).
#[must_use]
pub fn catalog_for_toolkit(toolkit: &str) -> Option<Arc<[CuratedTool]>> {
    current()?.catalogs.get(&key(toolkit)).cloned()
}

/// Toolkits the module marks agent-ready, sorted. Empty until [`prime`] runs.
#[must_use]
pub fn agent_ready_toolkits() -> Vec<String> {
    current().map(|s| s.agent_ready.clone()).unwrap_or_default()
}

/// Canonical toolkits that have a curated catalog.
#[must_use]
pub fn curated_toolkits() -> Vec<String> {
    current().map(|s| s.toolkits.clone()).unwrap_or_default()
}

/// Does `toolkit` have a native identity provider?
#[must_use]
pub fn has_native_provider(toolkit: &str) -> bool {
    current().is_some_and(|s| s.native.contains(&key(toolkit)))
}

/// What the module says `toolkit` can do, or [`FALLBACK_DESCRIPTION`].
#[must_use]
pub fn toolkit_description(toolkit: &str) -> &'static str {
    current()
        .and_then(|s| s.descriptions.get(&key(toolkit)).copied())
        .unwrap_or(FALLBACK_DESCRIPTION)
}

/// Should this action slug appear in the agent's tool surface, given an
/// already-loaded user scope preference? See `ListTools`: the module applies
/// the same rule when it lists tools.
#[must_use]
pub fn is_action_visible_with_pref(slug: &str, pref: &UserScopePref) -> bool {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        return true;
    };
    match catalog_for_toolkit(&toolkit) {
        Some(catalog) => find_curated(&catalog, slug).is_some_and(|c| pref.allows(c.scope)),
        None => pref.allows(classify_unknown(slug)),
    }
}

/// The curated scope `slug` requires, if it appears in a catalog.
#[must_use]
pub fn curated_scope_for(slug: &str) -> Option<ToolScope> {
    let catalog = catalog_for_toolkit(&toolkit_from_slug(slug)?)?;
    find_curated(&catalog, slug).map(|c| c.scope)
}

#[cfg(test)]
#[path = "curated_tests.rs"]
mod tests;
