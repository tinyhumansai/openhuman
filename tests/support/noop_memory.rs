//! A [`Memory`] that stores nothing, for integration targets that need one.
//!
//! Three targets build an `Agent` or a session that must be handed *a* memory
//! and never read one back. They used to get it from the engine's factory with
//! `backend: "none"` — an engine call whose entire purpose was to obtain
//! something that does not store, and one of the last things keeping
//! `tinymemory-core` on this crate's test critical path (openhuman#6161).
//!
//! It lives under `tests/support/` and is pulled in with `#[path]` because
//! `memory::test_support::noop_memory` is `pub(crate)`: a `tests/*.rs` target
//! links this crate as an ordinary dependency and cannot see crate-private
//! items, however they are declared.

#![allow(dead_code)]

use std::sync::Arc;

use openhuman_core::openhuman::memory::api::recall::RecallOpts;
use openhuman_core::openhuman::memory::api::types::{
    MemoryCategory, MemoryEntry, NamespaceSummary,
};
use openhuman_core::openhuman::memory::Memory;

/// Accepts every write and answers empty.
#[derive(Debug)]
pub struct NoopMemory;

#[async_trait::async_trait]
impl Memory for NoopMemory {
    fn name(&self) -> &str {
        "none"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

/// The shorthand the `backend: "none"` call sites use.
pub fn noop_memory() -> Arc<dyn Memory> {
    Arc::new(NoopMemory)
}
