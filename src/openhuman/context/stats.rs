//! Context utilisation and session-memory state for [`ContextManager`].
//!
//! Live context reduction is owned by the TinyAgents middleware stack. This
//! module keeps only OpenHuman-specific bookkeeping: last provider usage,
//! context-window utilisation for the UI/footer, and session-memory extraction
//! trigger state.

use std::sync::{Arc, Mutex};

use crate::openhuman::inference::provider::UsageInfo;

use super::session_memory::{SessionMemoryConfig, SessionMemoryState};

/// Shared handle to a [`SessionMemoryState`] so a detached background archivist
/// task can mark extraction complete/failed after the foreground turn releases
/// its borrow.
pub(crate) type SessionMemoryHandle = Arc<Mutex<SessionMemoryState>>;

#[derive(Debug)]
pub(crate) struct ContextStatsState {
    last_input_tokens: u64,
    last_output_tokens: u64,
    context_window: u64,
    session_memory_config: SessionMemoryConfig,
    session_memory: SessionMemoryHandle,
}

impl ContextStatsState {
    pub(crate) fn new(session_memory_config: SessionMemoryConfig) -> Self {
        Self {
            last_input_tokens: 0,
            last_output_tokens: 0,
            context_window: 0,
            session_memory_config,
            session_memory: Arc::new(Mutex::new(SessionMemoryState::default())),
        }
    }

    pub(crate) fn record_usage(&mut self, usage: &UsageInfo) {
        self.last_input_tokens = usage.input_tokens;
        self.last_output_tokens = usage.output_tokens;
        if usage.context_window > 0 {
            self.context_window = usage.context_window;
        }
        let total = usage.input_tokens + usage.output_tokens;
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.record_usage(total);
        }
    }

    pub(crate) fn tick_turn(&mut self) {
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.tick_turn();
        }
    }

    pub(crate) fn record_tool_calls(&mut self, n: usize) {
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.record_tool_calls(n);
        }
    }

    pub(crate) fn should_extract_session_memory(&self) -> bool {
        self.session_memory
            .lock()
            .map(|sm| sm.should_extract(&self.session_memory_config))
            .unwrap_or(false)
    }

    pub(crate) fn mark_session_memory_started(&mut self) {
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.mark_extraction_started();
        }
    }

    pub(crate) fn mark_session_memory_complete(&mut self) {
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.mark_extraction_complete();
        }
    }

    pub(crate) fn mark_session_memory_failed(&mut self) {
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.mark_extraction_failed();
        }
    }

    pub(crate) fn session_memory_snapshot(&self) -> SessionMemoryState {
        self.session_memory
            .lock()
            .map(|sm| sm.clone())
            .unwrap_or_default()
    }

    pub(crate) fn session_memory_handle(&self) -> SessionMemoryHandle {
        Arc::clone(&self.session_memory)
    }

    pub(crate) fn utilization_pct(&self) -> Option<u8> {
        if self.context_window == 0 {
            return None;
        }
        let total_used = self.last_input_tokens + self.last_output_tokens;
        let pct = (total_used as f64 / self.context_window as f64 * 100.0).round();
        Some(pct as u8)
    }

    pub(crate) fn last_input_tokens(&self) -> u64 {
        self.last_input_tokens
    }

    pub(crate) fn last_output_tokens(&self) -> u64 {
        self.last_output_tokens
    }

    pub(crate) fn context_window(&self) -> u64 {
        self.context_window
    }
}
