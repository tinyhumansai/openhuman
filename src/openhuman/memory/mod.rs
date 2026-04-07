//! Memory system for OpenHuman.
//!
//! This module provides the core abstractions and implementations for the memory system,
//! including semantic search, ingestion pipelines, document management, and knowledge graph
//! operations. It integrates vector search, keyword search, and relational data to provide
//! a unified memory interface for AI agents.

pub mod chunker;
pub mod embeddings;
pub mod global;
pub mod ingestion;
pub mod ingestion_queue;
pub mod ops;
pub(crate) mod relex;
pub mod rpc_models;
pub mod schemas;
pub mod store;
pub mod traits;

pub use ingestion::{
    ExtractedEntity, ExtractedRelation, ExtractionMode, MemoryIngestionConfig,
    MemoryIngestionRequest, MemoryIngestionResult, DEFAULT_GLINER_RELEX_MODEL,
};
pub use ingestion_queue::{IngestionJob, IngestionQueue};
pub use ops as rpc;
pub use ops::*;
pub use rpc_models::*;
pub use schemas::{
    all_controller_schemas as all_memory_controller_schemas,
    all_registered_controllers as all_memory_registered_controllers,
};
pub use store::{
    create_memory, create_memory_for_migration, create_memory_with_storage,
    create_memory_with_storage_and_routes, effective_memory_backend_name, MemoryClient,
    MemoryClientRef, MemoryItemKind, MemoryState, NamespaceDocumentInput, NamespaceMemoryHit,
    NamespaceQueryResult, NamespaceRetrievalContext, RetrievalScoreBreakdown, UnifiedMemory,
};
pub use traits::{Memory, MemoryCategory, MemoryEntry};
