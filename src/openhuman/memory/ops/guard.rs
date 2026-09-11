//! [`active_memory_guard`] — how a memory RPC handler reaches the **guarded**
//! driver.
//!
//! This is the read-write twin of
//! [`helpers::active_memory_client`](super::helpers::active_memory_client): the
//! same store, reached through
//! [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard) so the seven
//! policy steps in `docs/specs/kernel.md` §3.4 actually run. A handler that has
//! a typed contract twin for what it does calls this; one that does not stays
//! on `active_memory_client` and is listed, with its reason, in
//! `docs/specs/memory-guard-allowlist.md`.
//!
//! ## Two resolution paths, and why the second one exists
//!
//! The primary path is the ambient [`CoreContext`], exactly as
//! [`ops::provider`](super::provider) already resolves the binding for the
//! health probe. Under RPC dispatch a context is always present and its
//! workspace is always bound, so that is the only path production takes.
//!
//! The fallback path exists for callers that run *before* a context is built —
//! in practice the roughly four thousand pre-boot unit tests, which never build
//! a `CoreContext`. `CoreContext::memory()` cannot serve them:
//! `memory_binding()` goes through `workspace_dir()`, which errors outright
//! when the context has no bound workspace.
//!
//! ### The fallback names its workspace differently in the two builds, on purpose
//!
//! This used to read `tinymemory_core::global::active_workspace_dir()` — "the
//! workspace the in-process engine singleton is currently bound to" — and
//! preferred it over `Config::load_or_init` precisely so a test whose fixtures
//! wrote through that singleton could not be handed a binding over a *different*
//! workspace, which is a silently wrong store rather than a visible failure.
//!
//! #5560 deletes that singleton's boot, so the host no longer has an engine slot
//! to ask. The property still has to hold, so the fallback now names the same
//! workspace by its real source in each build:
//!
//! - **`cfg(test)`** — `test_support::shared_memory_test_workspace()`, the
//!   single leaked temp workspace every `memory::ops` fixture shares and the
//!   exact path `shared_memory_test_workspace()` binds. Asking the fixture
//!   directly is strictly *more* reliable than asking a singleton it happened to
//!   have initialised, because it cannot be re-pointed by an unrelated test.
//!   (Named without a doc link on purpose: the module it lives in is
//!   `#[cfg(test)]`, so a link would dangle in a documentation build.)
//! - **production** — `helpers::current_workspace_dir()`, i.e.
//!   `Config::load_or_init`. This is not a behaviour change: with the engine
//!   singleton no longer initialised at boot, `active_workspace_dir()` answered
//!   `None` on every production call anyway, so `Config::load_or_init` was
//!   already the only reachable answer here.
//!
//! The fallback binds with [`MemorySubsystemConfig::default`] (driver
//! `"tinycortex"`, default hook budgets). That is the right default precisely
//! because it is only reachable with no context: a context always carries the
//! operator's real `[subsystems.memory]` block and takes the first path.

use std::sync::Arc;

use crate::core::runtime::context::CoreContext;
use crate::openhuman::config::schema::MemorySubsystemConfig;
use crate::openhuman::memory::binding;
use crate::openhuman::memory::guard::MemoryGuard;

/// The workspace the pre-boot fallback guards — production build.
///
/// See the module docs for why the two builds resolve it differently. Two
/// definitions rather than one body with a `#[cfg]` block inside it: a bare
/// block in statement position is only the function's value once the *other*
/// arm has been stripped, which is exactly the kind of thing that compiles in
/// one build configuration and not the other.
#[cfg(not(test))]
async fn fallback_workspace_dir() -> Result<std::path::PathBuf, String> {
    super::helpers::current_workspace_dir().await
}

/// The workspace the pre-boot fallback guards — test build.
///
/// The shared fixture's own path, so a test that seeded through
/// `shared_memory_test_workspace()` is guaranteed the binding over the store its
/// fixtures wrote to. `async` to match the production arm; there is nothing to
/// await.
#[cfg(test)]
async fn fallback_workspace_dir() -> Result<std::path::PathBuf, String> {
    Ok(super::test_support::shared_memory_test_workspace())
}

/// The guarded memory driver for this dispatch.
///
/// # Errors
///
/// When neither resolution path can name a workspace — no ambient context and
/// `Config::load_or_init` also failing — or when the binding cache lock is
/// poisoned.
pub(crate) async fn active_memory_guard() -> Result<Arc<MemoryGuard>, String> {
    if let Some(ctx) = CoreContext::current() {
        match ctx.memory() {
            Ok(guard) => return Ok(guard),
            Err(error) => log::debug!(
                "[memory:guard] ambient context has no bound workspace ({error}); \
                 falling back to the configured workspace"
            ),
        }
    }

    let workspace_dir = fallback_workspace_dir().await?;
    log::debug!(
        "[memory:guard] no context binding; guarding workspace={}",
        workspace_dir.display()
    );
    Ok(binding::for_workspace(&workspace_dir, &MemorySubsystemConfig::default())?.guard())
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
