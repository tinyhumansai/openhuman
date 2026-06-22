//! Content-type detection and tool-name routing for compaction.
//!
//! Clean-room port of the routing behavior in headroom's `content_detector`
//! (Apache-2.0): cheap structural heuristics that classify a tool-output blob
//! so [`super::compact_tool_output`] can pick the right compressor. The tool
//! name gives a strong prior ([`ContentHint`]); detection validates/overrides
//! it for the `Auto` case.

/// The kind of content a tool produced, after detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    /// grep / ripgrep style `path:line:content` matches.
    Search,
    /// Build / test / lint output (compiler logs, test runners).
    Log,
    /// Unified git diff / patch.
    Diff,
    /// JSON array of objects (handled by the Phase-2 crusher).
    JsonArray,
    /// Nothing we compress — pass through unchanged.
    PlainText,
}

/// A prior derived from the tool name at the call site, so we don't have to
/// detect from scratch for the common case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentHint {
    Search,
    Log,
    Diff,
    Json,
    Auto,
}

/// Map a tool name to its content prior. Mirrors the target-tool table in the
/// compaction plan. Unknown tools fall through to [`ContentHint::Auto`].
pub fn hint_for_tool(tool_name: &str) -> ContentHint {
    match tool_name {
        "grep" | "glob_search" => ContentHint::Search,
        // Tools whose output is *reliably* a build/test/lint log.
        "run_tests" | "run_linter" | "npm_exec" | "node_exec" | "install_tool" | "lsp" => {
            ContentHint::Log
        }
        "read_diff" | "git_operations" => ContentHint::Diff,
        // `shell` is generic — its output is often NOT a log (find, seq, cat,
        // generated CSV, a script printing a list). Route it through detection
        // so only output that actually looks like a log gets log-compressed;
        // anything else passes through. See the log compressor's no-signal guard.
        _ => ContentHint::Auto,
    }
}

/// Resolve the content type to compress as. A non-`Auto` hint is trusted
/// unless detection strongly disagrees (e.g. a `shell` call that printed a
/// diff). For `Auto`, run full detection.
pub fn resolve(hint: ContentHint, content: &str) -> ContentType {
    match hint {
        ContentHint::Search => {
            // Trust the hint, but if the body is clearly a diff/log prefer that.
            if looks_like_diff(content) {
                ContentType::Diff
            } else {
                ContentType::Search
            }
        }
        ContentHint::Log => {
            if looks_like_diff(content) {
                ContentType::Diff
            } else if search_line_ratio(content) >= 0.6 {
                ContentType::Search
            } else {
                ContentType::Log
            }
        }
        ContentHint::Diff => {
            if looks_like_diff(content) {
                ContentType::Diff
            } else {
                detect(content)
            }
        }
        ContentHint::Json => {
            if looks_like_json_array(content) {
                ContentType::JsonArray
            } else {
                detect(content)
            }
        }
        ContentHint::Auto => detect(content),
    }
}

/// Full structural detection, in priority order: JSON → diff → search → log →
/// plain text. Thresholds mirror headroom's detector.
pub fn detect(content: &str) -> ContentType {
    let trimmed = content.trim_start();
    if trimmed.is_empty() {
        return ContentType::PlainText;
    }
    if looks_like_json_array(content) {
        return ContentType::JsonArray;
    }
    if looks_like_diff(content) {
        return ContentType::Diff;
    }
    if search_line_ratio(content) >= 0.6 {
        return ContentType::Search;
    }
    if log_line_ratio(content) >= 0.5 {
        return ContentType::Log;
    }
    ContentType::PlainText
}

/// True if `content` parses as a JSON array of objects (the crusher's input).
pub fn looks_like_json_array(content: &str) -> bool {
    let trimmed = content.trim_start();
    if !trimmed.starts_with('[') {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Array(items)) => {
            items.len() >= 2 && items.iter().any(|v| v.is_object())
        }
        _ => false,
    }
}

/// True if `content` looks like a unified diff: a `diff --git` header or at
/// least one hunk header (`@@ ... @@`).
pub fn looks_like_diff(content: &str) -> bool {
    let mut hunks = 0usize;
    for line in content.lines().take(400) {
        if line.starts_with("diff --git ") || line.starts_with("Index: ") {
            return true;
        }
        if line.starts_with("@@ ") && line[3..].contains("@@") {
            hunks += 1;
            if hunks >= 1 {
                return true;
            }
        }
    }
    false
}

/// Fraction of non-empty lines that look like `path:line:...` search hits.
/// Handles a leading Windows drive letter (`C:\...`) so those paths aren't
/// mistaken for the line-number separator.
fn search_line_ratio(content: &str) -> f32 {
    let mut total = 0usize;
    let mut hits = 0usize;
    for line in content.lines().take(2000) {
        if line.trim().is_empty() {
            continue;
        }
        total += 1;
        if parse_search_line(line).is_some() {
            hits += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        hits as f32 / total as f32
    }
}

/// Fraction of lines carrying an error/warning indicator — the log signal.
fn log_line_ratio(content: &str) -> f32 {
    use super::signals::{severity, Severity};
    let mut total = 0usize;
    let mut hits = 0usize;
    for line in content.lines().take(2000) {
        if line.trim().is_empty() {
            continue;
        }
        total += 1;
        if severity(line) != Severity::Other {
            hits += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        hits as f32 / total as f32
    }
}

/// Parse a single grep/ripgrep line into `(path, line_number, content)`.
///
/// Anchors on the earliest `:<digits>:` marker, skipping a leading Windows
/// drive prefix (`C:`), so paths may contain `:` (drive), `-`, and spaces.
/// Returns `None` for context lines (`rg` `-` separators) and non-matches.
pub fn parse_search_line(line: &str) -> Option<(&str, u64, &str)> {
    // Skip an optional Windows drive prefix like `C:` before scanning for the
    // line-number marker.
    let scan_from = if line.len() >= 2 {
        let bytes = line.as_bytes();
        if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            2
        } else {
            0
        }
    } else {
        0
    };

    let rest = &line[scan_from..];
    // Find the first ':' that is followed by digits and then another ':'.
    let mut search_start = 0usize;
    while let Some(rel) = rest[search_start..].find(':') {
        let colon = search_start + rel;
        let after = &rest[colon + 1..];
        let digits_len = after.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits_len > 0 && after.as_bytes().get(digits_len) == Some(&b':') {
            let path = &line[..scan_from + colon];
            let num: u64 = after[..digits_len].parse().ok()?;
            let body = &after[digits_len + 1..];
            if path.is_empty() {
                return None;
            }
            return Some((path, num, body));
        }
        search_start = colon + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_map_known_tools() {
        assert_eq!(hint_for_tool("grep"), ContentHint::Search);
        assert_eq!(hint_for_tool("run_tests"), ContentHint::Log);
        assert_eq!(hint_for_tool("read_diff"), ContentHint::Diff);
        assert_eq!(hint_for_tool("file_read"), ContentHint::Auto);
        // `shell` is generic → Auto (detection decides), not forced to Log.
        assert_eq!(hint_for_tool("shell"), ContentHint::Auto);
    }

    #[test]
    fn detect_search_results() {
        let c =
            "src/main.rs:42:fn process() {\nsrc/lib.rs:7:pub use foo;\nsrc/x.rs:99:    let y = 1;";
        assert_eq!(detect(c), ContentType::Search);
    }

    #[test]
    fn parse_unix_and_windows_paths() {
        assert_eq!(
            parse_search_line("src/main.rs:42:fn process() {"),
            Some(("src/main.rs", 42, "fn process() {"))
        );
        assert_eq!(
            parse_search_line(r"C:\Users\me\a.rs:10:let x = 1;"),
            Some((r"C:\Users\me\a.rs", 10, "let x = 1;"))
        );
        // dashes in filename must survive
        assert_eq!(
            parse_search_line("pre-commit-config.yaml:3:foo"),
            Some(("pre-commit-config.yaml", 3, "foo"))
        );
        assert_eq!(parse_search_line("just a sentence"), None);
    }

    #[test]
    fn detect_diff() {
        let c = "diff --git a/x.rs b/x.rs\n@@ -1,3 +1,4 @@\n+added\n-removed";
        assert_eq!(detect(c), ContentType::Diff);
    }

    #[test]
    fn detect_log() {
        let c =
            "Compiling foo\nwarning: unused\nerror[E0382]: borrow of moved value\nerror: aborting";
        assert_eq!(detect(c), ContentType::Log);
    }

    #[test]
    fn detect_json_array() {
        let c = r#"[{"id":1,"name":"a"},{"id":2,"name":"b"}]"#;
        assert_eq!(detect(c), ContentType::JsonArray);
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(
            detect("just some prose about a topic"),
            ContentType::PlainText
        );
    }

    #[test]
    fn log_hint_with_search_body_routes_search() {
        let body = "a.rs:1:x\nb.rs:2:y\nc.rs:3:z\nd.rs:4:w";
        assert_eq!(resolve(ContentHint::Log, body), ContentType::Search);
    }
}
