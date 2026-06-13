//! High-level memory capture / remember orchestration.
//!
//! `memory_store` owns persistence, `memory_sync` owns upstream pulls, and
//! `memory_tree` owns summarisation / traversal mechanics. This module is where
//! the `memory` domain decides how an incoming "remember this" request should be
//! classified before delegating to those backends.

use serde::{Deserialize, Serialize};

/// Origin of a remember request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RememberSourceKind {
    ChatHistory,
    UploadedData,
    LlmThought,
}

impl RememberSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatHistory => "chat_history",
            Self::UploadedData => "uploaded_data",
            Self::LlmThought => "llm_thought",
        }
    }
}
