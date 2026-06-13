//! Drop tool-call payloads from a conversation.
//!
//! Pure transform — no IO, no allocation beyond the result vec.

use crate::openhuman::memory_archivist::types::Turn;

/// Return a new conversation with every turn's `tool_calls_json` stripped
/// to `None`. Also drops tool-role turns entirely (their content is the
/// tool result, which is noisy and rarely useful out of context).
pub fn clean_conversation(turns: &[Turn]) -> Vec<Turn> {
    turns
        .iter()
        .filter(|t| t.role != "tool")
        .map(|t| Turn {
            role: t.role.clone(),
            content: t.content.clone(),
            tool_calls_json: None,
            timestamp: t.timestamp,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn t(role: &str, content: &str, tool_calls: Option<&str>) -> Turn {
        Turn {
            role: role.into(),
            content: content.into(),
            tool_calls_json: tool_calls.map(|s| s.into()),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn drops_tool_calls_json_on_assistant_turns() {
        let convo = vec![
            t("user", "what's the time?", None),
            t("assistant", "let me check", Some(r#"[{"name":"clock"}]"#)),
        ];
        let cleaned = clean_conversation(&convo);
        assert_eq!(cleaned.len(), 2);
        assert!(cleaned[1].tool_calls_json.is_none());
        assert_eq!(cleaned[1].content, "let me check");
    }

    #[test]
    fn drops_tool_role_turns_entirely() {
        let convo = vec![
            t("user", "list files", None),
            t("assistant", "running ls", Some(r#"[{"name":"bash"}]"#)),
            t("tool", "a.txt\nb.txt", None),
            t("assistant", "two files", None),
        ];
        let cleaned = clean_conversation(&convo);
        assert_eq!(cleaned.len(), 3);
        assert!(cleaned.iter().all(|t| t.role != "tool"));
    }

    #[test]
    fn preserves_user_and_system_turns_unchanged() {
        let convo = vec![t("system", "be brief", None), t("user", "hi", None)];
        let cleaned = clean_conversation(&convo);
        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned[0].role, "system");
        assert_eq!(cleaned[1].content, "hi");
    }
}
