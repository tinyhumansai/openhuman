//! Durable agent session database.
//!
//! SQLite-backed store (WAL + FTS5) for sessions, messages, tool calls,
//! cost metadata, and parent/child lineage. Complements the existing
//! `session_raw/*.jsonl` transcript files — those remain the source of
//! truth for KV-cache resume; this module provides queryable indexing,
//! cross-session search, and orchestration recovery.
//!
//! Database path: `{workspace}/session_db/sessions.db`.

mod ops;
mod schemas;
mod store;
pub mod types;

pub use ops::{
    get_session, import_transcript, list_sessions, record_message, record_session_end,
    record_session_start, record_tool_call, search_sessions,
};
pub use schemas::{
    all_controller_schemas as all_session_db_controller_schemas,
    all_registered_controllers as all_session_db_registered_controllers,
};
pub use store::with_connection;
pub use types::{
    SessionMessage, SessionRecord, SessionSearchParams, SessionSearchResult, SessionStatus,
    SessionToolCall,
};
