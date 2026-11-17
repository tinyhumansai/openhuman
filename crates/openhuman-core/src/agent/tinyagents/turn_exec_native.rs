use tinyagents_harness::events::{AgentEvent, EventSink};

use crate::agent::tinyagents::journal;

/// Native identity retained by OpenHuman's neutral sub-agent adapter.
#[derive(Clone)]
pub(crate) struct NativeSubagentRunContext {
    pub config: tinyagents_harness::context::RunConfig,
    pub parent_events: EventSink,
    pub child_events: std::sync::Arc<std::sync::Mutex<Option<EventSink>>>,
    pub child_journal: std::sync::Arc<std::sync::Mutex<Option<journal::TurnJournal>>>,
    pub name: String,
    pub depth: usize,
}

impl NativeSubagentRunContext {
    pub(crate) fn finalize_failed_boundary(&self) {
        let events = self
            .child_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(events) = events else {
            return;
        };
        events.emit(AgentEvent::RunFinalized {
            run_id: self.config.run_id.clone(),
        });
        if let Some(journal) = self
            .child_journal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            journal.flush_events();
        }
    }
}
