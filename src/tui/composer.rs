//! Keyboard-first, terminal-independent chat composer.

use unicode_width::UnicodeWidthChar;

/// Commands implemented by the terminal client.  The list is deliberately
/// OpenHuman-native: commands either map to a stable core RPC or a local view.
pub const COMMANDS: &[(&str, &str)] = &[
    ("help", "show commands and keyboard shortcuts"),
    ("new", "start a new thread"),
    ("resume", "browse saved threads"),
    ("rename", "rename the current thread"),
    ("delete", "delete the current thread"),
    ("model", "choose the model for subsequent turns"),
    ("permissions", "inspect or change agent access"),
    ("status", "show session, core, and Git status"),
    ("usage", "show token and cost usage"),
    ("goal", "view or manage the thread goal"),
    ("tasks", "show the thread task board"),
    ("agents", "browse and switch agent profiles"),
    ("skills", "browse installed skills and recent runs"),
    ("mcp", "show installed MCP servers"),
    ("artifacts", "browse generated artifacts"),
    ("approvals", "show pending approval requests"),
    (
        "diff",
        "show the working-tree diff when action_dir is a Git repo",
    ),
    ("review", "ask OpenHuman to review the working tree"),
    ("copy", "copy the latest completed answer"),
    ("export", "export this transcript as Markdown"),
    ("clear", "clear the visible transcript"),
    ("logs", "open Logs"),
    ("config", "open Config"),
    ("settings", "open Settings"),
    ("logout", "log out"),
    ("quit", "exit the TUI"),
];

#[derive(Debug, Clone, Default)]
pub struct Composer {
    text: String,
    /// Character index, never a byte offset.
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    saved_draft: String,
}

impl Composer {
    pub fn text(&self) -> &str {
        &self.text
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
        self.history_index = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.history_index = None;
        self.saved_draft.clear();
    }

    pub fn insert_char(&mut self, ch: char) {
        let byte = char_to_byte(&self.text, self.cursor);
        self.text.insert(byte, ch);
        self.cursor += 1;
        self.history_index = None;
    }

    pub fn insert_str(&mut self, value: &str) {
        let byte = char_to_byte(&self.text, self.cursor);
        self.text.insert_str(byte, value);
        self.cursor += value.chars().count();
        self.history_index = None;
    }

    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = char_to_byte(&self.text, self.cursor - 1);
        let end = char_to_byte(&self.text, self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.text.chars().count() {
            return;
        }
        let start = char_to_byte(&self.text, self.cursor);
        let end = char_to_byte(&self.text, self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    pub fn move_home(&mut self) {
        let before: String = self.text.chars().take(self.cursor).collect();
        self.cursor = before
            .rfind('\n')
            .map_or(0, |idx| before[..=idx].chars().count());
    }

    pub fn move_end(&mut self) {
        let after: String = self.text.chars().skip(self.cursor).collect();
        self.cursor += after
            .find('\n')
            .map_or_else(|| after.chars().count(), |idx| after[..idx].chars().count());
    }

    pub fn delete_word_back(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let start_byte = char_to_byte(&self.text, start);
        let end_byte = char_to_byte(&self.text, self.cursor);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor = start;
    }

    pub fn take_for_send(&mut self) -> Option<String> {
        let value = self.text.trim().to_string();
        if value.is_empty() {
            return None;
        }
        if self.history.last() != Some(&value) {
            self.history.push(value.clone());
        }
        self.clear();
        Some(value)
    }

    pub fn history_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index.is_none() {
            self.saved_draft = self.text.clone();
        }
        let next = self
            .history_index
            .map_or(self.history.len() - 1, |i| i.saturating_sub(1));
        self.history_index = Some(next);
        self.text = self.history[next].clone();
        self.cursor = self.text.chars().count();
    }

    pub fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.text = self.history[index + 1].clone();
        } else {
            self.history_index = None;
            self.text = std::mem::take(&mut self.saved_draft);
        }
        self.cursor = self.text.chars().count();
    }

    pub fn history_search(&self, query: &str) -> Vec<String> {
        let query = query.to_ascii_lowercase();
        self.history
            .iter()
            .rev()
            .filter(|item| item.to_ascii_lowercase().contains(&query))
            .take(20)
            .cloned()
            .collect()
    }

    pub fn command_query(&self) -> Option<&str> {
        let line = self.text.lines().next().unwrap_or_default();
        line.strip_prefix('/')
            .filter(|s| !s.contains(char::is_whitespace))
    }

    pub fn command_matches(&self) -> Vec<(&'static str, &'static str)> {
        let Some(query) = self.command_query() else {
            return Vec::new();
        };
        COMMANDS
            .iter()
            .copied()
            .filter(|(name, _)| fuzzy_match(name, query))
            .take(12)
            .collect()
    }

    pub fn complete_command(&mut self) -> bool {
        let Some((name, _)) = self.command_matches().first().copied() else {
            return false;
        };
        self.set_text(format!("/{name} "));
        true
    }

    pub fn file_query(&self) -> Option<String> {
        let before: String = self.text.chars().take(self.cursor).collect();
        before
            .split_whitespace()
            .last()
            .and_then(|token| token.strip_prefix('@'))
            .map(str::to_string)
    }

    pub fn replace_current_token(&mut self, replacement: &str) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut start = self.cursor;
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let start_byte = char_to_byte(&self.text, start);
        let end_byte = char_to_byte(&self.text, self.cursor);
        self.text.replace_range(start_byte..end_byte, replacement);
        self.cursor = start + replacement.chars().count();
    }

    /// Visible rows and cursor position for a width-constrained composer.
    pub fn display(&self, width: usize) -> (Vec<String>, usize, usize) {
        let width = width.max(1);
        let mut rows = vec![String::new()];
        let mut row = 0usize;
        let mut col = 0usize;
        let mut cursor_row = 0usize;
        let mut cursor_col = 0usize;
        for (idx, ch) in self.text.chars().enumerate() {
            if idx == self.cursor {
                cursor_row = row;
                cursor_col = col;
            }
            if ch == '\n' {
                rows.push(String::new());
                row += 1;
                col = 0;
                continue;
            }
            let cw = ch.width().unwrap_or(0).max(1);
            if col + cw > width {
                rows.push(String::new());
                row += 1;
                col = 0;
            }
            rows[row].push(ch);
            col += cw;
        }
        if self.cursor == self.text.chars().count() {
            cursor_row = row;
            cursor_col = col;
        }
        (rows, cursor_row, cursor_col)
    }
}

fn char_to_byte(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map_or(value.len(), |(idx, _)| idx)
}

fn fuzzy_match(candidate: &str, query: &str) -> bool {
    if query.is_empty() || candidate.contains(query) {
        return true;
    }
    let mut chars = candidate.chars();
    query
        .chars()
        .all(|needle| chars.by_ref().any(|ch| ch == needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_editing_uses_character_indices() {
        let mut c = Composer::default();
        c.insert_str("a🦀b");
        c.move_left();
        c.backspace();
        assert_eq!(c.text(), "ab");
        assert_eq!(c.cursor(), 1);
    }

    #[test]
    fn multiline_home_end_and_wrapping() {
        let mut c = Composer::default();
        c.set_text("one\ntwo long");
        c.move_home();
        assert_eq!(c.cursor(), 4);
        c.move_end();
        assert_eq!(c.cursor(), 12);
        let (rows, row, col) = c.display(5);
        assert_eq!(rows, vec!["one", "two l", "ong"]);
        assert_eq!((row, col), (2, 3));
    }

    #[test]
    fn history_preserves_draft_and_deduplicates() {
        let mut c = Composer::default();
        c.set_text("first");
        assert_eq!(c.take_for_send().as_deref(), Some("first"));
        c.set_text("first");
        c.take_for_send();
        c.set_text("draft");
        c.history_previous();
        assert_eq!(c.text(), "first");
        c.history_next();
        assert_eq!(c.text(), "draft");
    }

    #[test]
    fn slash_commands_filter_fuzzily() {
        let mut c = Composer::default();
        c.set_text("/perm");
        assert_eq!(c.command_matches()[0].0, "permissions");
        c.set_text("/rsm");
        assert_eq!(c.command_matches()[0].0, "resume");
    }

    #[test]
    fn file_mentions_replace_only_the_current_token() {
        let mut composer = Composer::default();
        composer.set_text("review @src/ma");
        assert_eq!(composer.file_query().as_deref(), Some("src/ma"));
        composer.replace_current_token("@src/main.rs");
        assert_eq!(composer.text(), "review @src/main.rs");
    }
}
