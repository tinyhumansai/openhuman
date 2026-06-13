//! Compose a cleaned conversation into a single markdown blob.
//!
//! The output is the body of one tree leaf — newline-separated `## role`
//! sections with the turn content underneath. Plain markdown; no YAML
//! front-matter (the tree leaf already carries timestamps + provenance).

use crate::openhuman::memory_archivist::types::Turn;

pub fn compose_conversation_md(turns: &[Turn]) -> String {
    let mut out = String::new();
    for (idx, turn) in turns.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str("## ");
        out.push_str(&turn.role);
        out.push('\n');
        out.push_str(&turn.content);
        if !turn.content.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn t(role: &str, content: &str) -> Turn {
        Turn {
            role: role.into(),
            content: content.into(),
            tool_calls_json: None,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn empty_input_gives_empty_string() {
        assert_eq!(compose_conversation_md(&[]), "");
    }

    #[test]
    fn role_headings_separate_turns() {
        let md = compose_conversation_md(&[t("user", "hi"), t("assistant", "hello")]);
        assert!(md.contains("## user\nhi\n"));
        assert!(md.contains("## assistant\nhello\n"));
    }

    #[test]
    fn turns_separated_by_blank_line() {
        let md = compose_conversation_md(&[t("user", "a"), t("user", "b")]);
        // turn boundaries get one blank line between them
        assert!(md.contains("a\n\n## user\nb"));
    }
}
