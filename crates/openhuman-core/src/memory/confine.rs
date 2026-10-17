//! Confinement of the ambient memory surface: the `openhuman.memory_*` RPCs
//! and the MCP memory tools, which dispatch through them.
//!
//! The agent `memory` tool ([`super::tools`]) always reads and forgets within
//! the acting identity's layout: `Reach::subtree(<root>)`. The RPCs used to
//! take whatever filter the caller sent, and with no `reach` an engine reads
//! every namespace, so an RPC or MCP caller could list, read and forget
//! another root's memory (another team's, or another host-bound agent's).
//!
//! Every RPC resolves the identity the caller passes ([`MemoryIdentity`]; the
//! RPC handlers pass [`MemoryIdentity::root`], the config's own root
//! identity) and:
//!
//! - fills an unset `reach` with that identity's subtree ([`allowed_reach`]);
//! - keeps a caller's reach only when it is [`Reach::within`] the allowed
//!   one, and refuses a wider one with `INVALID_REQUEST`;
//! - places a `learn` with no namespace at the identity's learnings node and
//!   refuses one aimed outside the subtree.

use tinymemory_api::{ExplorePage, MemoryMeta, MetaFilter, Namespace, Reach};

use crate::config::Config;

use super::error::{MemoryError, MemoryResult};
use super::explore::{self, ExploreParams, ItemsGetParams, ItemsGetView};
use super::ops;
use super::scope::MemoryIdentity;
use super::types::{
    FetchParams, FetchView, ForgetParams, ForgetView, ItemsListParams, ItemsListView, LearnParams,
    LearnView, RecallParams, RecallView,
};

/// What `identity` under `config` may read and forget: the subtree of its
/// layout root.
#[must_use]
pub fn allowed_reach(config: &Config, identity: &MemoryIdentity) -> Reach {
    Reach::subtree(identity.resolve(config).root().clone())
}

/// `reach` confined to `allowed`: unset becomes `allowed`, a reach within it
/// is kept.
///
/// # Errors
///
/// [`MemoryError::InvalidRequest`] when `reach` reads outside `allowed`.
pub fn confine_reach(reach: Option<Reach>, allowed: &Reach) -> MemoryResult<Reach> {
    match reach {
        None => Ok(allowed.clone()),
        Some(reach) if reach.within(allowed) => Ok(reach),
        Some(reach) => {
            tracing::warn!(
                asked = %reach.at,
                allowed = %allowed.at,
                "[memory:confine] refused a reach outside the caller's root"
            );
            Err(MemoryError::invalid(format!(
                "the reach `{}` is outside this caller's memory (`{}`)",
                reach.at, allowed.at
            )))
        }
    }
}

/// `filter` with its reach confined to `allowed` ([`confine_reach`]).
///
/// # Errors
///
/// [`MemoryError::InvalidRequest`] when the filter's reach reads outside
/// `allowed`.
pub fn confine_filter(filter: Option<MetaFilter>, allowed: &Reach) -> MemoryResult<MetaFilter> {
    let mut filter = filter.unwrap_or_default();
    filter.reach = Some(confine_reach(filter.reach.take(), allowed)?);
    Ok(filter)
}

/// The metadata a `memory_learn` caller sent, with its namespace inside the
/// `identity`'s layout: an unset namespace (the root) lands at the
/// layout's learnings node when that is not the root itself.
///
/// # Errors
///
/// [`MemoryError::InvalidRequest`] when the namespace is outside the
/// layout's subtree.
pub fn confine_learn_meta(
    config: &Config,
    identity: &MemoryIdentity,
    meta: Option<MemoryMeta>,
) -> MemoryResult<MemoryMeta> {
    let resolved = identity.resolve(config);
    let allowed = Reach::subtree(resolved.root().clone());
    let mut meta = meta.unwrap_or_default();
    if meta.namespace == Namespace::ROOT {
        meta.namespace = resolved.layout.learnings().clone();
    }
    if !allowed.admits(&meta.namespace) {
        tracing::warn!(
            asked = %meta.namespace,
            allowed = %allowed.at,
            "[memory:confine] refused a learning outside the caller's root"
        );
        return Err(MemoryError::invalid(format!(
            "the namespace `{}` is outside this caller's memory (`{}`)",
            meta.namespace, allowed.at
        )));
    }
    Ok(meta)
}

/// `memory_recall`, confined.
///
/// # Errors
///
/// A reach outside the caller's root, or what [`ops::recall`] returns.
pub async fn recall(
    config: &Config,
    identity: &MemoryIdentity,
    mut params: RecallParams,
) -> MemoryResult<RecallView> {
    params.filter = Some(confine_filter(
        params.filter,
        &allowed_reach(config, identity),
    )?);
    ops::recall(config, params).await
}

/// `memory_fetch`, confined.
///
/// # Errors
///
/// A reach outside the caller's root, or what [`ops::fetch`] returns.
pub async fn fetch(
    config: &Config,
    identity: &MemoryIdentity,
    mut params: FetchParams,
) -> MemoryResult<FetchView> {
    params.filter = Some(confine_filter(
        params.filter,
        &allowed_reach(config, identity),
    )?);
    ops::fetch(config, params).await
}

/// `memory_learn`, confined: the learning lands inside the caller's layout.
///
/// # Errors
///
/// A namespace outside the caller's root, or what [`ops::learn`] returns.
pub async fn learn(
    config: &Config,
    identity: &MemoryIdentity,
    mut params: LearnParams,
) -> MemoryResult<LearnView> {
    params.meta = Some(confine_learn_meta(config, identity, params.meta)?);
    ops::learn(config, params, None).await
}

/// `memory_forget`, confined: an id outside the caller's root is not
/// forgotten.
///
/// # Errors
///
/// A reach outside the caller's root, or what [`ops::forget`] returns.
pub async fn forget(
    config: &Config,
    identity: &MemoryIdentity,
    mut params: ForgetParams,
) -> MemoryResult<ForgetView> {
    params.reach = Some(confine_reach(
        params.reach,
        &allowed_reach(config, identity),
    )?);
    ops::forget(config, params).await
}

/// `memory_items_list`, confined. The explorer path narrows first (a
/// namespace step sets the reach), then the narrowed filter is confined.
///
/// # Errors
///
/// A reach outside the caller's root, an invalid path, or what
/// [`ops::items_list`] returns.
pub async fn items_list(
    config: &Config,
    identity: &MemoryIdentity,
    mut params: ItemsListParams,
) -> MemoryResult<ItemsListView> {
    let narrowed = explore::narrowed(params.filter.take(), &params.path)?;
    params.filter = Some(confine_filter(
        Some(narrowed),
        &allowed_reach(config, identity),
    )?);
    params.path = Vec::new();
    ops::items_list(config, params).await
}

/// `memory_explore`, confined like [`items_list`].
///
/// # Errors
///
/// A reach outside the caller's root, an invalid path, or what
/// [`explore::explore`] returns.
pub async fn explore(
    config: &Config,
    identity: &MemoryIdentity,
    mut params: ExploreParams,
) -> MemoryResult<ExplorePage> {
    let narrowed = explore::narrowed(params.filter.take(), &params.path)?;
    params.filter = Some(confine_filter(
        Some(narrowed),
        &allowed_reach(config, identity),
    )?);
    params.path = Vec::new();
    explore::explore(config, params).await
}

/// `memory_items_get`, confined: an id outside the caller's root reads as
/// unknown.
///
/// # Errors
///
/// A reach outside the caller's root, or what [`explore::items_get`]
/// returns.
pub async fn items_get(
    config: &Config,
    identity: &MemoryIdentity,
    mut params: ItemsGetParams,
) -> MemoryResult<ItemsGetView> {
    params.reach = Some(confine_reach(
        params.reach,
        &allowed_reach(config, identity),
    )?);
    explore::items_get(config, params).await
}

#[cfg(test)]
#[path = "confine_tests.rs"]
mod tests;
