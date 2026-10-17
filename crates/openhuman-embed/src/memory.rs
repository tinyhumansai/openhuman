//! Memory sub-facade — one tenant's memory, read and managed by the host.
//!
//! An agent's turns already run TinyMemory's lifecycle on their own (the
//! pre-turn pack, the post-turn log, compaction recall, the `memory` tool),
//! under the memory binding its [`AgentSpec`](crate::AgentSpec) carries. What
//! a host still needs beside its agents is the operator's view of the same
//! memory: which agents have logged turns, what one of them remembers, the
//! shared learnings and brain documents, and a way to file or forget them.
//!
//! Before this existed a host had two routes, both wrong for a multi-tenant
//! host: reach into `openhuman_core::memory::*` with a hand-built config, or
//! invoke the `openhuman.memory_*` RPCs, which run against the ambient config
//! and so against whichever root that config happens to name. [`Memory`] is
//! bound to one layout root ([`Runtime::memory`](crate::Runtime::memory)) and
//! every call it makes is confined to that root's subtree:
//!
//! ```text
//! <root>                      shared learnings
//! ├── source:<kind>           the brain: documents by source type
//! └── agent:<memory agent>    one agent's conversations
//! ```
//!
//! - every read carries a reach of the root's subtree, so a filter cannot
//!   widen it to a sibling tenant;
//! - [`Memory::forget`] forgets only ids the root's subtree holds — an id from
//!   another tenant is left alone as if it named nothing;
//! - [`Memory::learn`] writes at the root, whatever the caller's metadata says,
//!   and [`Memory::store`] writes at the root or one agent's node below it;
//! - [`Memory::search`] ranks only items the root's subtree holds, narrowed by
//!   kind and tag.
//!
//! The engine is the one every agent uses: the configured `[memory]` engine,
//! or one the host installed with [`install_host_engine`]. With neither, memory
//! is off and every call but [`Memory::status`] answers
//! [`MemoryError::Off`].

use openhuman_core::config::Config;
use openhuman_core::memory::engine::{self, Binding};
use openhuman_core::memory::scope::MemoryIdentity;
use openhuman_core::memory::{brain, explore, lifecycle, ops, types};
use tinymemory_api::{
    ItemKind, MemoryMeta, MetaFilter, Namespace, Reach, StoreItem, StoreReceipt, WriteOptions,
};
use tinymemory_tools::MemoryLayout;

pub use openhuman_core::memory::brain::{
    BrainForgetView, BrainIngestParams, BrainIngestView, BrainSearchView, BrainSourceCount,
    BrainSourcesView,
};
pub use openhuman_core::memory::engine::{clear_host_engine, install_host_engine};
pub use openhuman_core::memory::lifecycle::views::{AgentCount, AgentsView};
pub use openhuman_core::memory::types::{
    FetchView, ForgetView, ItemsListView, LearnParams, LearnView, RecallView,
};
pub use openhuman_core::memory::{MemoryError, MemoryResult};

/// The TinyMemory engine contract, for a host that implements or installs an
/// engine ([`install_host_engine`]) or reads the items this facade returns.
pub use tinymemory_api as api;

/// Largest page [`Memory::forget_agent`] lists at once.
const FORGET_PAGE: usize = 100;

/// Whether memory is on for a root, and with which engine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MemoryStatus {
    /// The layout root.
    pub root: String,
    /// Whether an engine is bound.
    pub on: bool,
    /// The engine id, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// The engine endpoint, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Why memory is off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Which items [`Memory::list`] pages through.
#[derive(Debug, Clone, Default)]
pub struct ItemsQuery {
    /// One memory agent's node (its conversations and the beliefs built
    /// there); `None` lists the whole root.
    pub agent_id: Option<String>,
    /// Item kinds to include; empty means every kind.
    pub kinds: Vec<ItemKind>,
    /// Match an item carrying any one of these tags; empty means no constraint.
    pub tags_any: Vec<String>,
    /// Page size.
    pub limit: Option<usize>,
    /// Engine cursor of the next page.
    pub cursor: Option<String>,
}

/// Which items [`Memory::search`] ranks.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// What to rank the items against.
    pub query: String,
    /// One memory agent's node plus the root's items it inherits, as
    /// [`Memory::recall`] reads it; `None` searches the whole root.
    pub agent_id: Option<String>,
    /// Item kinds to include; empty means every kind.
    pub kinds: Vec<ItemKind>,
    /// Match an item carrying any one of these tags; empty means no constraint.
    pub tags_any: Vec<String>,
    /// Most hits returned.
    pub limit: Option<usize>,
}

/// One tenant's memory: every call confined to one layout root. See the
/// module docs.
#[derive(Clone)]
pub struct Memory {
    config: Config,
    layout: MemoryLayout,
}

impl std::fmt::Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memory")
            .field("root", &self.layout.root().to_string())
            .finish_non_exhaustive()
    }
}

impl Memory {
    /// Binds `base` to the layout root `root` (`team:acme`).
    pub(crate) fn bind(mut base: Config, root: &str) -> MemoryResult<Self> {
        let root = root.trim();
        openhuman_core::memory::scope::validate_root(root).map_err(MemoryError::invalid)?;
        let namespace: Namespace = root
            .parse()
            .map_err(|error: tinymemory_api::Error| MemoryError::invalid(error.to_string()))?;
        if namespace.is_root() {
            return Err(MemoryError::invalid(
                "a tenant's memory needs a root below the store root",
            ));
        }
        let layout = MemoryLayout::new(namespace)?;
        // The host binding outranks every identity, and the facade acts as the
        // root one, so a call made from inside an agent's turn still lands on
        // this root.
        base.memory.root = Some(root.to_string());
        base.memory.agent_id = None;
        log::debug!("[embed][memory] bound root={root}");
        Ok(Self {
            config: base,
            layout,
        })
    }

    /// The layout root.
    pub fn root(&self) -> String {
        self.layout.root().to_string()
    }

    /// The node `agent_id`'s conversations live at.
    pub fn agent_node(&self, agent_id: &str) -> MemoryResult<String> {
        Ok(self.layout.conversations(agent_id)?.to_string())
    }

    /// Whether memory is on, and with what.
    pub fn status(&self) -> MemoryStatus {
        match engine::resolve(&self.config) {
            Binding::On(bound) => MemoryStatus {
                root: self.root(),
                on: true,
                engine: Some(bound.id),
                endpoint: Some(bound.endpoint),
                reason: None,
            },
            Binding::Off {
                engine,
                endpoint,
                reason,
            } => MemoryStatus {
                root: self.root(),
                on: false,
                engine,
                endpoint,
                reason: Some(reason),
            },
        }
    }

    /// Whether an engine is bound.
    pub fn is_on(&self) -> bool {
        engine::is_on(&self.config)
    }

    /// The agents with logged turns under the root, most first.
    pub async fn agents(&self) -> MemoryResult<AgentsView> {
        lifecycle::views::agents_list(&self.config, &MemoryIdentity::root()).await
    }

    /// A page of items, newest first.
    pub async fn list(&self, query: ItemsQuery) -> MemoryResult<ItemsListView> {
        let reach = self.reach_of(query.agent_id.as_deref())?;
        ops::items_list(
            &self.config,
            types::ItemsListParams {
                filter: Some(MetaFilter {
                    reach: Some(reach),
                    kinds: query.kinds,
                    tags_any: query.tags_any,
                    ..MetaFilter::default()
                }),
                limit: query.limit,
                cursor: query.cursor,
                path: Vec::new(),
                preview: false,
            },
        )
        .await
    }

    /// The items among `ids` the root holds.
    pub async fn get(&self, ids: Vec<String>) -> MemoryResult<Vec<api::Hit>> {
        let view = explore::items_get(
            &self.config,
            explore::ItemsGetParams {
                ids,
                reach: Some(self.subtree()),
            },
        )
        .await?;
        Ok(view.items)
    }

    /// Answers `question` from the root's memory, or from one agent's node
    /// plus the shared learnings at the root above it.
    pub async fn recall(
        &self,
        question: impl Into<String>,
        agent_id: Option<&str>,
        limit: Option<usize>,
    ) -> MemoryResult<RecallView> {
        ops::recall(
            &self.config,
            types::RecallParams {
                refers_to: None,
                question: question.into(),
                filter: Some(self.reading(agent_id)?),
                limit,
            },
        )
        .await
    }

    /// Ranked items matching `query` under the root, optionally of `kinds`.
    pub async fn fetch(
        &self,
        query: impl Into<String>,
        kinds: Vec<ItemKind>,
        limit: Option<usize>,
    ) -> MemoryResult<FetchView> {
        ops::fetch(
            &self.config,
            types::FetchParams {
                refers_to: None,
                query: query.into(),
                mode: None,
                filter: Some(MetaFilter {
                    reach: Some(self.subtree()),
                    kinds,
                    ..MetaFilter::default()
                }),
                limit,
                cursor: None,
            },
        )
        .await
    }

    /// Ranked items matching `query.query` under the root, narrowed by kind
    /// and tag: [`Memory::fetch`] for a host that files what it captures
    /// under tags of its own and reads only some of them back.
    pub async fn search(&self, query: SearchQuery) -> MemoryResult<FetchView> {
        let mut filter = self.reading(query.agent_id.as_deref())?;
        filter.kinds = query.kinds;
        filter.tags_any = query.tags_any;
        ops::fetch(
            &self.config,
            types::FetchParams {
                refers_to: None,
                query: query.query,
                mode: None,
                filter: Some(filter),
                limit: query.limit,
                cursor: None,
            },
        )
        .await
    }

    /// Stores an item of any kind the host captured itself (a document, a
    /// conversation, a learning) at the root, shared by every agent bound
    /// there, or at `agent_id`'s node. The node is authoritative: a
    /// namespace in the item's meta is overwritten. Everything else in the
    /// meta (tags, source, `observed_at`, `observed_actor`) is kept, so an
    /// identical item stored again is a replay
    /// ([`StoreItem::fingerprint`](api::StoreItem::fingerprint)).
    ///
    /// [`WriteOptions::accepted`] returns once the engine has durably
    /// accepted the item, before it is readable: a capture on a live path,
    /// or a bulk import that reads nothing back until it ends.
    pub async fn store(
        &self,
        mut item: StoreItem,
        agent_id: Option<&str>,
        options: WriteOptions,
    ) -> MemoryResult<StoreReceipt> {
        let node = match agent_id.map(str::trim).filter(|agent| !agent.is_empty()) {
            Some(agent) => self.layout.conversations(agent)?,
            None => self.layout.learnings().clone(),
        };
        item.meta_mut().namespace = node;
        ops::store_item_with(&self.config, item, options).await
    }

    /// Stores a shared learning at the root. The root is authoritative: a
    /// namespace in `params.meta` is overwritten.
    pub async fn learn(&self, params: LearnParams) -> MemoryResult<LearnView> {
        let host = MemoryMeta {
            namespace: self.layout.learnings().clone(),
            ..MemoryMeta::default()
        };
        ops::learn(&self.config, params, Some(host)).await
    }

    /// Forgets the items among `ids` the root holds; any other id is left
    /// alone.
    pub async fn forget(&self, ids: Vec<String>) -> MemoryResult<ForgetView> {
        ops::forget(
            &self.config,
            types::ForgetParams {
                ids,
                reach: Some(self.subtree()),
            },
        )
        .await
    }

    /// Forgets everything stored at and below one agent's node: its logged
    /// turns and the beliefs built from them. Returns how many were removed.
    pub async fn forget_agent(&self, agent_id: &str) -> MemoryResult<usize> {
        let mut forgotten = 0;
        loop {
            let page = self
                .list(ItemsQuery {
                    agent_id: Some(agent_id.to_string()),
                    limit: Some(FORGET_PAGE),
                    ..ItemsQuery::default()
                })
                .await?;
            if page.items.is_empty() {
                break;
            }
            let ids = page.items.into_iter().map(|hit| hit.id.0).collect();
            let removed = self.forget(ids).await?.forgotten;
            forgotten += removed;
            // Nothing removed means the engine keeps listing items it will not
            // forget; stop rather than loop on them.
            if removed == 0 {
                break;
            }
        }
        log::debug!(
            "[embed][memory] forgot agent root={} agent={agent_id} items={forgotten}",
            self.root()
        );
        Ok(forgotten)
    }

    /// The brain's sources under the root and their sizes.
    pub async fn brain_sources(&self) -> MemoryResult<BrainSourcesView> {
        brain::sources(&self.config, &MemoryIdentity::root()).await
    }

    /// Brain documents matching `query`, optionally of one source.
    pub async fn brain_search(
        &self,
        query: impl Into<String>,
        source: Option<String>,
        limit: Option<usize>,
    ) -> MemoryResult<BrainSearchView> {
        brain::search(
            &self.config,
            &MemoryIdentity::root(),
            brain::BrainSearchParams {
                query: query.into(),
                source,
                limit,
            },
        )
        .await
    }

    /// Files a document (a file on disk, or text) in the root's brain.
    pub async fn brain_ingest(&self, params: BrainIngestParams) -> MemoryResult<BrainIngestView> {
        brain::ingest(&self.config, &MemoryIdentity::root(), params).await
    }

    /// Forgets every brain document of one source under the root.
    pub async fn brain_forget(&self, source: impl Into<String>) -> MemoryResult<BrainForgetView> {
        brain::forget(
            &self.config,
            &MemoryIdentity::root(),
            brain::BrainForgetParams {
                source: source.into(),
            },
        )
        .await
    }

    fn subtree(&self) -> Reach {
        Reach::subtree(self.layout.root().clone())
    }

    /// The reach a listing reads: the root's subtree, or one agent's node
    /// and everything below it.
    fn reach_of(&self, agent_id: Option<&str>) -> MemoryResult<Reach> {
        match agent_id.map(str::trim).filter(|agent| !agent.is_empty()) {
            Some(agent) => Ok(Reach {
                at: self.layout.conversations(agent)?,
                inherit: false,
                descendants: true,
            }),
            None => Ok(self.subtree()),
        }
    }

    /// What a recall reads: the whole root, or one agent's node with the
    /// root's shared learnings it inherits — never a sibling agent's turns.
    fn reading(&self, agent_id: Option<&str>) -> MemoryResult<MetaFilter> {
        match agent_id.map(str::trim).filter(|agent| !agent.is_empty()) {
            Some(agent) => Ok(MetaFilter {
                reach: Some(Reach {
                    at: self.layout.conversations(agent)?,
                    inherit: true,
                    descendants: true,
                }),
                ..MetaFilter::default()
            }),
            None => Ok(self.layout.holistic_filter()),
        }
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
