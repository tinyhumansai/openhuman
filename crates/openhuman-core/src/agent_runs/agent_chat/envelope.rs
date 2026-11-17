use crate::core::Outcome;

/// The log line every successful agent chat turn carries.
const AGENT_CHAT_COMPLETED_LOG: &str = "agent chat completed";

/// What one agent chat turn produced: its reply and whether it stopped at the
/// tool-iteration cap rather than finishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentChatReply {
    /// The reply text. When `hit_cap` is set this is the turn's checkpoint.
    pub text: String,
    /// The turn paused at its iteration cap (see
    /// [`OpenHumanSessionHost::last_turn_hit_cap`](crate::agent::OpenHumanSessionHost::last_turn_hit_cap).
    pub hit_cap: bool,
}

impl AgentChatReply {
    /// The historical `Outcome<String>` shape, for library callers.
    pub fn into_outcome(self) -> Outcome<String> {
        Outcome::single_log(self.text, AGENT_CHAT_COMPLETED_LOG)
    }

    /// The `inference.agent_chat` wire shape: the historical
    /// `{"result": <text>, "logs": [...]}` envelope plus `hit_cap`, and, when
    /// the turn was capped, `checkpoint` (the same text, named for what it is).
    ///
    /// Additive on purpose: existing clients keep reading the reply from
    /// `result` (directly or through `unwrap_rpc`), and a headless caller can
    /// now tell a capped turn from a finished one and continue it (#6958).
    pub fn into_rpc_json(self) -> Result<serde_json::Value, String> {
        let hit_cap = self.hit_cap;
        let checkpoint = hit_cap.then(|| self.text.clone());
        let mut value = self.into_outcome().into_cli_compatible_json()?;
        let serde_json::Value::Object(map) = &mut value else {
            return Err("agent chat envelope is not a JSON object".to_string());
        };
        map.insert("hit_cap".to_string(), serde_json::Value::Bool(hit_cap));
        if let Some(checkpoint) = checkpoint {
            map.insert(
                "checkpoint".to_string(),
                serde_json::Value::String(checkpoint),
            );
        }
        Ok(value)
    }
}

#[cfg(test)]
#[path = "envelope_tests.rs"]
mod tests;
