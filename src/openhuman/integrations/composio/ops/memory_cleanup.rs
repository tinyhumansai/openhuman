//! Memory cleanup helpers used when deleting a Composio connection.

use crate::openhuman::config::Config;
use crate::openhuman::memory::binding::MemoryBinding;
use tinymemory_api::chunks::SourceKind;
use tinymemory_api::provider::ForgetSelector;

/// KV namespace the Composio sync pipelines keep their per-connection cursor
/// state under, and the state record itself.
///
/// Both named at the **contract** (#5560). This file used to hold the namespace
/// as a hand-copied literal, with a comment explaining that two engine crates
/// also spelled it and that the row could have been written by either. The
/// contract publishes it — and marks it, with the record's serde field names, a
/// compatibility surface — so there is one spelling now, and it is the one the
/// driver that writes these rows reads.
///
/// [`SyncState`] likewise: this module only ever deserialised it to reach
/// `synced_ids`, and the contract's declaration is the engine's field for
/// field, in the same order, under the same `#[serde(default)]`s. Naming the
/// contract's is what makes "the shape this reads is the shape that was
/// written" a fact about the build rather than about two files agreeing.
use tinymemory_api::composio::{SyncState, STATE_NAMESPACE as SYNC_STATE_NAMESPACE};

/// One thing a connection delete has to remove from memory.
///
/// The three variants are the three [`ForgetSelector`] arms this domain needs,
/// named in this domain's own vocabulary: a Composio connection files content
/// under an exact source id, under a family of derived ids sharing a prefix,
/// and — for a mailbox shared with other connections — under an owner. They
/// stay a local enum rather than becoming `ForgetSelector` directly because
/// `label` is what a partial-failure message shows the user, and that wording
/// is this host's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MemoryCleanupTarget {
    Exact(SourceKind, String),
    Prefix(SourceKind, String),
    Owner(SourceKind, String),
}

impl MemoryCleanupTarget {
    /// The contract selector this target names.
    ///
    /// `source_kind` crosses as a wire string because the set of source kinds
    /// belongs to the host's sync machinery and grows without a contract
    /// change; `SourceKind::as_str` is the same spelling `label` already
    /// shows.
    fn selector(&self) -> ForgetSelector {
        match self {
            Self::Exact(source_kind, source_id) => ForgetSelector::Source {
                source_kind: source_kind.as_str().to_string(),
                source_id: source_id.clone(),
            },
            Self::Prefix(source_kind, source_id_prefix) => ForgetSelector::SourcePrefix {
                source_kind: source_kind.as_str().to_string(),
                source_id_prefix: source_id_prefix.clone(),
            },
            Self::Owner(source_kind, owner) => ForgetSelector::Owner {
                source_kind: source_kind.as_str().to_string(),
                owner: owner.clone(),
            },
        }
    }

    /// Remove this target through the bound memory driver, returning the chunk
    /// count it took with it.
    ///
    /// A driver without the `Sources` family is **refused**, not degraded to
    /// zero: this is a delete, and its only empty answer — zero chunks
    /// removed — is byte-identical to a successful delete of nothing. The
    /// caller sums these into `memory_chunks_deleted` on the delete-connection
    /// reply, so a silent zero would tell the user a disconnected account's
    /// mail had left memory while it is still on disk. The caller already
    /// collects per-target failures and reports them beside the count, so a
    /// refusal here is surfaced rather than fatal.
    ///
    /// `ForgetOutcome::trees_cleaned` is dropped on purpose —
    /// `memory_chunks_deleted` has always been a chunk count, and this does
    /// not change the reply's shape.
    ///
    /// No `spawn_blocking`: the driver owns whether its own writes block, and
    /// the module's do not run on this thread at all.
    pub(super) async fn delete(&self, config: &Config) -> anyhow::Result<usize> {
        let binding = crate::openhuman::memory::binding::for_config(config)
            .map_err(|e| anyhow::anyhow!("forget_matching: {e}"))?;
        let Some(sources) = binding.provider().as_sources() else {
            return Err(anyhow::anyhow!(
                "forget_matching: driver '{}' does not serve Sources",
                binding.driver_id()
            ));
        };
        let selector = self.selector();
        let outcome = sources
            .forget_matching(&selector)
            .await
            .map_err(|e| anyhow::anyhow!("forget_matching: {e}"))?;
        log::debug!(
            "[composio][memory] forget_matching target={} removed chunks={} trees={} (driver='{}')",
            self.label(),
            outcome.chunks_removed,
            outcome.trees_cleaned,
            binding.driver_id()
        );
        Ok(usize::try_from(outcome.chunks_removed).unwrap_or(usize::MAX))
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Exact(source_kind, source_id) => {
                format!("{}:{source_id}", source_kind.as_str())
            }
            Self::Prefix(source_kind, source_id_prefix) => {
                format!("{}:{source_id_prefix}*", source_kind.as_str())
            }
            Self::Owner(source_kind, owner) => {
                format!("{}:owner:{owner}", source_kind.as_str())
            }
        }
    }
}

/// Enumerate what a connection delete has to remove, for one toolkit.
///
/// **Takes a `&Config`, not a memory handle, and that is the openhuman#5560
/// change.** This used to take `&MemoryClientRef` — the caller's handle on the
/// live in-process store — because the notion arm loaded sync state through
/// the engine's `HostSyncAdapter`. That adapter is two lines
/// (`kv_get` / `kv_set` on the client), the contract's `MemoryGraph` serves
/// exactly those, and the in-process engine is gone — so the read goes over the
/// bus and the parameter became the config the binding is keyed on.
///
/// The old signature's warning still applies to whoever is tempted to construct
/// a client here instead: `MemoryClient::from_workspace_dir` starts an
/// ingestion worker at construction, so building one per connection-delete put
/// a second worker on the live store every time — the hazard
/// `memory::bypass_allowlist_tests` names for that constructor. The binding is
/// a workspace-keyed cache and starts nothing.
///
/// The binding is resolved here rather than inside the notion arm so the
/// ordering the caller had is preserved: `composio_delete_connection` resolved
/// the memory handle **before** target discovery and refused the whole delete
/// if it could not, rather than deleting the connection and then discovering
/// memory was unreachable.
pub(crate) async fn composio_memory_targets_for_connection(
    config: &Config,
    toolkit: Option<&str>,
    connection_id: &str,
) -> anyhow::Result<Vec<MemoryCleanupTarget>> {
    let Some(toolkit) = toolkit.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let binding = crate::openhuman::memory::binding::for_config(config)
        .map_err(|error| anyhow::anyhow!("memory driver unavailable: {error}"))?;

    let targets = match toolkit.to_ascii_lowercase().as_str() {
        "slack" => vec![MemoryCleanupTarget::Exact(
            SourceKind::Chat,
            format!("slack:{connection_id}"),
        )],
        "gmail" => gmail_memory_sources_for_connection(connection_id),
        "notion" => notion_memory_targets_for_connection(&binding, connection_id).await?,
        "drive" | "googledrive" | "google_drive" => {
            drive_memory_targets_for_connection(connection_id)
        }
        _ => Vec::new(),
    };
    Ok(targets)
}

fn gmail_memory_sources_for_connection(connection_id: &str) -> Vec<MemoryCleanupTarget> {
    vec![
        MemoryCleanupTarget::Owner(SourceKind::Email, format!("gmail-sync:{connection_id}")),
        MemoryCleanupTarget::Exact(SourceKind::Email, format!("gmail:{connection_id}")),
        MemoryCleanupTarget::Prefix(SourceKind::Email, format!("gmail:{connection_id}:")),
        MemoryCleanupTarget::Prefix(SourceKind::Email, format!("gmail:{connection_id}/")),
    ]
}

/// Notion files one memory document per synced page, so the delete has to know
/// which pages this connection ever synced — that list lives in the sync
/// pipeline's own KV row, not in any chunk.
///
/// Read through the contract's `Graph` family. The three outcomes are kept
/// exactly as `SyncState::load` had them, because the caller's contract is
/// "every page this connection synced, or an error":
///
/// - **no row** → an empty synced set, same as `SyncState::new`. A connection
///   that never synced has nothing extra to forget.
/// - **a row that will not deserialise** → `Err`, *not* an empty set. A corrupt
///   cursor means the page list is unknown, and reporting "nothing to delete"
///   would leave a disconnected account's pages in memory while telling the
///   user they were removed. `notion_cleanup_targets_surface_corrupt_sync_state`
///   pins the message.
/// - **a driver with no `Graph` family** → `Err`, for the same reason.
///
/// All three failures render through [`sync_state_load_error`] so they stay one
/// message to the caller, exactly as they were when `SyncState::load` produced
/// all three itself.
async fn notion_memory_targets_for_connection(
    binding: &MemoryBinding,
    connection_id: &str,
) -> anyhow::Result<Vec<MemoryCleanupTarget>> {
    let mut targets = connection_scoped_document_targets("notion", connection_id);

    let Some(graph) = binding.provider().as_graph() else {
        return Err(sync_state_load_error(format!(
            "driver '{}' does not serve Graph",
            binding.driver_id()
        )));
    };
    let key = format!("notion:{connection_id}");
    let record = graph
        .kv_get(Some(SYNC_STATE_NAMESPACE), &key)
        .await
        .map_err(sync_state_load_error)?;
    let synced_ids = match record {
        Some(record) => {
            serde_json::from_value::<SyncState>(record.value)
                .map_err(sync_state_load_error)?
                .synced_ids
        }
        None => {
            log::debug!(
                "[composio][memory] no notion sync state for connection (driver='{}'); no page targets",
                binding.driver_id()
            );
            Default::default()
        }
    };
    for raw_id in synced_ids {
        let Some(page_id) = notion_synced_page_id(&raw_id) else {
            continue;
        };
        targets.push(MemoryCleanupTarget::Exact(
            SourceKind::Document,
            format!("notion:{page_id}"),
        ));
        targets.push(MemoryCleanupTarget::Exact(
            SourceKind::Document,
            format!("composio-notion-page-{page_id}"),
        ));
    }

    Ok(dedupe_memory_targets(targets))
}

/// The one message the notion sync-state read reports, whatever went wrong.
///
/// `notion_cleanup_targets_surface_corrupt_sync_state` asserts on the
/// `"failed to load notion sync state"` prefix, and the caller in
/// `composio_delete_connection` renders it into a user-facing "cannot enumerate
/// memory targets" refusal — so the wording is load-bearing in both directions
/// and is kept identical to the one `SyncState::load`'s caller used to build.
fn sync_state_load_error(detail: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("failed to load notion sync state for memory cleanup: {detail}")
}

fn drive_memory_targets_for_connection(connection_id: &str) -> Vec<MemoryCleanupTarget> {
    ["drive", "googledrive", "google_drive"]
        .into_iter()
        .flat_map(|prefix| connection_scoped_document_targets(prefix, connection_id))
        .collect()
}

fn connection_scoped_document_targets(
    prefix: &str,
    connection_id: &str,
) -> Vec<MemoryCleanupTarget> {
    vec![
        MemoryCleanupTarget::Exact(SourceKind::Document, format!("{prefix}:{connection_id}")),
        MemoryCleanupTarget::Prefix(SourceKind::Document, format!("{prefix}:{connection_id}:")),
        MemoryCleanupTarget::Prefix(SourceKind::Document, format!("{prefix}:{connection_id}/")),
    ]
}

fn notion_synced_page_id(raw_id: &str) -> Option<String> {
    let page_id = raw_id.split_once('@').map_or(raw_id, |(id, _)| id).trim();
    (!page_id.is_empty()).then(|| page_id.to_string())
}

fn dedupe_memory_targets(targets: Vec<MemoryCleanupTarget>) -> Vec<MemoryCleanupTarget> {
    let mut unique = Vec::new();
    for target in targets {
        if !unique.contains(&target) {
            unique.push(target);
        }
    }
    unique
}
