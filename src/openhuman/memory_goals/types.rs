//! Domain types for the agent's long-term goals list.
//!
//! Goals are a small, ordered list of durable objectives the agent holds
//! when interacting with the user. They are persisted as a compact markdown
//! document (`MEMORY_GOALS.md`) — see [`super::store`] — and surfaced over
//! RPC + agent tools. Each item carries a stable short id so edit/delete
//! operations can address a specific line without depending on ordering.

use serde::{Deserialize, Serialize};

/// A single long-term goal item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalItem {
    /// Stable short id (e.g. `g1`). Used as the dedupe/address key for
    /// `edit`/`delete`. Rendered inline in the markdown as `- [g1] …`.
    pub id: String,
    /// The goal text — one concise sentence.
    pub text: String,
}

impl GoalItem {
    /// Construct a goal item from an id + text, trimming surrounding
    /// whitespace from the text.
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into().trim().to_string(),
        }
    }
}

/// The full goals document — an ordered list of [`GoalItem`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalsDoc {
    /// Ordered goal items. Order is meaningful for rendering and cap
    /// trimming (oldest = front).
    pub items: Vec<GoalItem>,
}

/// Markdown header rendered at the top of `MEMORY_GOALS.md`.
const HEADER: &str = "# Long-term Goals";

impl GoalsDoc {
    /// Parse a `MEMORY_GOALS.md` body into a [`GoalsDoc`].
    ///
    /// Recognised item lines look like `- [g1] do the thing`. Lines that
    /// don't match (the header, blank lines, free prose) are ignored so a
    /// hand-edited file degrades gracefully rather than erroring.
    pub fn parse(body: &str) -> Self {
        let mut items = Vec::new();
        for line in body.lines() {
            let trimmed = line.trim();
            // Strip the leading list marker, if present.
            let rest = match trimmed.strip_prefix("- ") {
                Some(r) => r.trim(),
                None => continue,
            };
            // Expect `[id] text`.
            let Some(after_open) = rest.strip_prefix('[') else {
                continue;
            };
            let Some(close_idx) = after_open.find(']') else {
                continue;
            };
            let id = after_open[..close_idx].trim();
            let text = after_open[close_idx + 1..].trim();
            if id.is_empty() || text.is_empty() {
                continue;
            }
            items.push(GoalItem::new(id, text));
        }
        Self { items }
    }

    /// Render the document back to markdown suitable for `MEMORY_GOALS.md`.
    pub fn render(&self) -> String {
        let mut out = String::from(HEADER);
        out.push_str("\n\n");
        for item in &self.items {
            out.push_str(&format!("- [{}] {}\n", item.id, item.text));
        }
        out
    }

    /// Whether the list currently has no items. Used to drive the
    /// "first run / initial population" enrichment behaviour.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Allocate the next free `g<N>` id not already used in the list.
    pub fn next_id(&self) -> String {
        let mut n = self.items.len() + 1;
        loop {
            let candidate = format!("g{n}");
            if !self.items.iter().any(|i| i.id == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// Append a new goal, returning the assigned id. Text is trimmed; an
    /// empty text is rejected.
    pub fn add(&mut self, text: &str) -> Result<String, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("goal text must not be empty".to_string());
        }
        if text.contains('\n') || text.contains('\r') {
            return Err("goal text must be a single line".to_string());
        }
        let id = self.next_id();
        self.items.push(GoalItem::new(&id, text));
        Ok(id)
    }

    /// Replace the text of the goal with `id`. Returns an error if the id
    /// is unknown or the new text is empty.
    pub fn edit(&mut self, id: &str, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("goal text must not be empty".to_string());
        }
        if text.contains('\n') || text.contains('\r') {
            return Err("goal text must be a single line".to_string());
        }
        let item = self
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| format!("no goal with id '{id}'"))?;
        item.text = text.to_string();
        Ok(())
    }

    /// Delete the goal with `id`. Returns an error if the id is unknown.
    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        let before = self.items.len();
        self.items.retain(|i| i.id != id);
        if self.items.len() == before {
            return Err(format!("no goal with id '{id}'"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_render() {
        let mut doc = GoalsDoc::default();
        doc.add("ship the desktop app").unwrap();
        doc.add("keep the rust core authoritative").unwrap();
        let rendered = doc.render();
        let reparsed = GoalsDoc::parse(&rendered);
        assert_eq!(doc, reparsed);
    }

    #[test]
    fn parse_ignores_non_item_lines() {
        let body = "# Long-term Goals\n\nsome stray prose\n- [g1] real goal\n- malformed line\n";
        let doc = GoalsDoc::parse(body);
        assert_eq!(doc.items.len(), 1);
        assert_eq!(doc.items[0].id, "g1");
        assert_eq!(doc.items[0].text, "real goal");
    }

    #[test]
    fn add_assigns_unique_ids() {
        let mut doc = GoalsDoc::default();
        let a = doc.add("a").unwrap();
        let b = doc.add("b").unwrap();
        assert_ne!(a, b);
        assert_eq!(doc.items.len(), 2);
    }

    #[test]
    fn add_rejects_empty_text() {
        let mut doc = GoalsDoc::default();
        assert!(doc.add("   ").is_err());
    }

    #[test]
    fn add_and_edit_reject_multiline_text() {
        let mut doc = GoalsDoc::default();
        // A newline-bearing goal would inject extra "- [..]" list lines on
        // reload, corrupting the stored shape — reject it outright.
        assert!(doc.add("line one\n- [x] injected").is_err());
        let id = doc.add("legit goal").unwrap();
        assert!(doc.edit(&id, "still\rinjected").is_err());
    }

    #[test]
    fn edit_updates_known_id_and_rejects_unknown() {
        let mut doc = GoalsDoc::default();
        let id = doc.add("old").unwrap();
        doc.edit(&id, "new").unwrap();
        assert_eq!(doc.items[0].text, "new");
        assert!(doc.edit("nope", "x").is_err());
    }

    #[test]
    fn delete_removes_known_id_and_rejects_unknown() {
        let mut doc = GoalsDoc::default();
        let id = doc.add("x").unwrap();
        doc.delete(&id).unwrap();
        assert!(doc.is_empty());
        assert!(doc.delete("nope").is_err());
    }

    #[test]
    fn next_id_avoids_collision_with_custom_ids() {
        let mut doc = GoalsDoc {
            items: vec![GoalItem::new("g1", "a"), GoalItem::new("g2", "b")],
        };
        let id = doc.add("c").unwrap();
        assert_eq!(id, "g3");
    }
}
