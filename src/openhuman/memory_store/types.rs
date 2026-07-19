//! Stable host path for tinycortex-owned namespace memory contracts.

pub use tinycortex::memory::{
    GraphRelationRecord, MemoryItemKind, MemoryKvRecord, NamespaceDocumentInput,
    NamespaceMemoryHit, NamespaceQueryResult, NamespaceRetrievalContext, RetrievalScoreBreakdown,
    StoredMemoryDocument,
};

pub(crate) use tinycortex::memory::types::GLOBAL_NAMESPACE;
