use super::*;

impl OpenHumanSessionHost {
    /// Discards this host's cached session composition.
    ///
    /// This synchronous compatibility method does not reset durable history;
    /// callers that serve a user-visible reset command must use
    /// `reset_bound_history` so the transcript generation changes atomically.
    pub fn clear_history(&mut self) {
        // Dropping the composition only affects this host instance. Durable
        // transcript state remains untouched.
        self.runtime_session = None;
        self.runtime_state = Arc::new(std::sync::Mutex::new(
            super::super::super::runtime_session::OpenHumanSessionState::default(),
        ));
    }

    /// Set the overrides applied to the **next** [`Self::turn`] call.
    ///
    /// The overrides are consumed at the top of the next turn (they apply to a
    /// single turn, then reset to the default), so a caller running a chat /
    /// small-talk turn calls this immediately before [`Self::turn`]. Callers
    /// that never touch this get the unchanged full-agentic behaviour. See
    /// [`TurnOverrides`](super::super::super::types::TurnOverrides) for the fields and the
    /// motivating case (#1725: a bare greeting must not run the task loop nor
    /// inherit a prior task's goal / tools / memory).
    pub fn set_next_turn_overrides(
        &mut self,
        overrides: super::super::super::types::TurnOverrides,
    ) {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending_turn_overrides = overrides;
    }

    /// Ask the **next** turn to shape every model call (response format,
    /// output cap) and report its final call into `scope`. Consumed at the
    /// start of that turn, like [`Self::set_next_turn_overrides`].
    pub fn set_next_turn_response_shape(
        &mut self,
        scope: Option<Arc<tinyagents_harness::middleware::ResponseShapeScope>>,
    ) {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending_response_shape = scope;
    }

    /// Set the reply-language instruction every following user message
    /// carries (next to the `Current Date & Time` line), or clear it with
    /// `None`. Sent per message rather than baked into the system prompt, so
    /// the cached prompt prefix stays byte-stable and a changed interface
    /// locale applies from the next turn.
    pub fn set_reply_language_directive(&mut self, directive: Option<String>) {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reply_language_directive = directive;
    }

    /// The user's IANA time zone, which the date line on each later user
    /// message is stamped in (`None`: the device's).
    pub fn set_time_zone(&mut self, zone: Option<String>) {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .time_zone = zone;
    }

    /// The reply-language instruction the next user message will carry.
    pub fn reply_language_directive(&self) -> Option<String> {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reply_language_directive
            .clone()
    }
}
