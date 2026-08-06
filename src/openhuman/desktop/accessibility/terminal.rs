//! Terminal app detection and context extraction.

/// Known terminal application name substrings (lowercase).
/// Extend this list to support additional terminal emulators.
pub const TERMINAL_NAMES: &[&str] = &[
    "terminal",
    "iterm",
    "wezterm",
    "warp",
    "alacritty",
    "kitty",
    "ghostty",
    "hyper",
    "rio",
    "tabby",
    "wave",
    "contour",
    "foot",
];

pub fn is_text_role(role: Option<&str>) -> bool {
    matches!(
        role.unwrap_or_default(),
        "AXTextArea" | "AXTextField" | "AXSearchField" | "AXComboBox" | "AXEditableText"
    )
}

pub fn is_terminal_app(app_name: Option<&str>) -> bool {
    let app = app_name.unwrap_or_default().to_ascii_lowercase();
    TERMINAL_NAMES.iter().any(|needle| app.contains(needle))
}

pub fn looks_like_terminal_buffer(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let line_count = text.lines().count();
    line_count >= 5
        && (lower.contains("$ ")
            || lower.contains("# ")
            || lower.contains("❯")
            || lower.contains("[1] 0:")
            || lower.contains("tmux")
            || lower.contains("cargo run")
            || lower.contains("git status"))
}

fn is_terminal_noise_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    trimmed.starts_with('•')
        || trimmed.starts_with('└')
        || trimmed.starts_with('─')
        || trimmed.starts_with('│')
        || (trimmed.starts_with('[')
            && (trimmed.contains(" 0:") || trimmed.contains("[tmux]") || trimmed.contains("\"⠙")))
}

pub fn extract_terminal_input_context(text: &str) -> String {
    let mut fallback = String::new();
    for raw_line in text.lines().rev().take(40) {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if fallback.is_empty() && !is_terminal_noise_line(line) {
            fallback = line.to_string();
        }
        if is_terminal_noise_line(line) {
            continue;
        }
        if line.contains("$ ")
            || line.contains("# ")
            || line.contains("❯")
            || line.contains("➜")
            || line.contains("λ")
        {
            return line.to_string();
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_text_role_accepts_known_roles() {
        assert!(is_text_role(Some("AXTextArea")));
        assert!(is_text_role(Some("AXTextField")));
        assert!(is_text_role(Some("AXSearchField")));
        assert!(is_text_role(Some("AXComboBox")));
        assert!(is_text_role(Some("AXEditableText")));
    }

    #[test]
    fn is_text_role_rejects_other_roles() {
        assert!(!is_text_role(Some("AXButton")));
        assert!(!is_text_role(Some("AXImage")));
        assert!(!is_text_role(None));
        assert!(!is_text_role(Some("")));
    }

    #[test]
    fn is_terminal_app_detects_known_terminals() {
        assert!(is_terminal_app(Some("iTerm2")));
        assert!(is_terminal_app(Some("Terminal")));
        assert!(is_terminal_app(Some("WezTerm")));
        assert!(is_terminal_app(Some("Alacritty")));
        assert!(is_terminal_app(Some("kitty")));
        assert!(is_terminal_app(Some("Warp")));
        assert!(is_terminal_app(Some("Ghostty")));
    }

    #[test]
    fn is_terminal_app_rejects_non_terminals() {
        assert!(!is_terminal_app(Some("Safari")));
        assert!(!is_terminal_app(Some("Slack")));
        assert!(!is_terminal_app(None));
        assert!(!is_terminal_app(Some("")));
    }

    #[test]
    fn looks_like_terminal_buffer_detects_shell_prompts() {
        let buffer = "line1\nline2\nline3\nline4\n$ cargo build\nCompiling...\n";
        assert!(looks_like_terminal_buffer(buffer));
    }

    #[test]
    fn looks_like_terminal_buffer_rejects_short_text() {
        assert!(!looks_like_terminal_buffer("hello"));
        assert!(!looks_like_terminal_buffer("$ cmd"));
    }

    #[test]
    fn looks_like_terminal_buffer_detects_git_status() {
        let buffer = "line1\nline2\nline3\nline4\ngit status\nOn branch main\n";
        assert!(looks_like_terminal_buffer(buffer));
    }

    #[test]
    fn extract_terminal_input_context_finds_prompt_line() {
        let text = "old output\n$ ls -la\ntotal 42\nfile1.txt\nfile2.txt\n";
        let ctx = extract_terminal_input_context(text);
        assert!(ctx.contains("$ ls"), "expected prompt line, got: {ctx}");
    }

    #[test]
    fn extract_terminal_input_context_skips_noise() {
        let text = "actual content\n• bullet point\n└── tree branch\n│ pipe\n";
        let ctx = extract_terminal_input_context(text);
        assert_eq!(ctx, "actual content");
    }

    #[test]
    fn extract_terminal_input_context_empty_returns_empty() {
        assert!(extract_terminal_input_context("").is_empty());
    }

    #[test]
    fn extract_terminal_input_context_all_noise_returns_empty() {
        let text = "\n\n\n";
        assert!(extract_terminal_input_context(text).is_empty());
    }

    #[test]
    fn terminal_names_is_nonempty() {
        assert!(!TERMINAL_NAMES.is_empty());
    }

    #[test]
    fn is_terminal_noise_line_detects_noise() {
        assert!(is_terminal_noise_line(""));
        assert!(is_terminal_noise_line("   "));
        assert!(is_terminal_noise_line("• item"));
        assert!(is_terminal_noise_line("└── branch"));
        assert!(is_terminal_noise_line("─────"));
        assert!(is_terminal_noise_line("│ pipe"));
    }

    #[test]
    fn is_terminal_noise_line_passes_normal_text() {
        assert!(!is_terminal_noise_line("hello world"));
        assert!(!is_terminal_noise_line("$ command"));
    }
}
