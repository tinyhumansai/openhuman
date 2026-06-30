//! Content-kind detection for the TokenJuice content router.
//!
//! Cheap structural heuristics that classify a blob so the router can pick the
//! right compressor. Ported and extended from the compaction router's
//! `detect.rs` (clean-room port of Headroom's `content_detector`, Apache-2.0),
//! adding HTML and source-code detection and a richer [`ContentHint`] that can
//! carry a MIME type, file extension, producing tool, and a hard override.
//!
//! Resolution precedence (cheap → expensive):
//!   1. `hint.explicit`            — hard override, returned verbatim.
//!   2. `hint.mime` / `hint.extension` — unambiguous type tags.
//!   3. `hint.source_tool`         — strong tool prior (grep→Search, …).
//!   4. structural detection       — JSON → Diff → HTML → Search → Code → Log.

use crate::openhuman::tokenjuice::detect::hint::{
    extension_to_kind, mime_to_kind, prior_to_kind, tool_prior, ToolPrior,
};
use crate::openhuman::tokenjuice::types::{ContentHint, ContentKind};

/// Resolve the [`ContentKind`] for `content` given a caller [`ContentHint`].
pub fn detect_content_kind(content: &str, hint: &ContentHint) -> ContentKind {
    // 1. Hard override.
    if let Some(kind) = hint.explicit {
        return kind;
    }
    // 2. MIME / extension tags. For a Code/Json/Html tag we still sanity-check
    //    diff bodies (a `shell` cat of a `.json` that is actually a patch is
    //    rare, but a diff body is unmistakable and cheap to confirm).
    if let Some(kind) = hint.mime.as_deref().and_then(mime_to_kind) {
        return reconcile_tag(kind, content);
    }
    if let Some(kind) = hint
        .extension
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
        .and_then(extension_to_kind)
    {
        return reconcile_tag(kind, content);
    }
    // 3. Tool prior.
    if let Some(tool) = hint.source_tool.as_deref() {
        match tool_prior(tool) {
            // A Search prior is absolute: grep output is never re-routed.
            ToolPrior::Search => return ContentKind::Search,
            ToolPrior::Auto => {}
            other => {
                if let Some(kind) = prior_to_kind(other) {
                    return reconcile_tag(kind, content);
                }
            }
        }
    }
    // 4. Full structural detection.
    detect(content)
}

/// A type tag (from MIME/extension/tool) is trusted unless the body is clearly
/// a diff — diffs are unmistakable and worth preferring even when the file
/// extension says otherwise.
fn reconcile_tag(tagged: ContentKind, content: &str) -> ContentKind {
    if tagged != ContentKind::Diff && looks_like_diff(content) {
        ContentKind::Diff
    } else {
        tagged
    }
}

/// Full structural detection, in priority order: JSON → diff → HTML → search →
/// code → log → plain text.
pub fn detect(content: &str) -> ContentKind {
    let trimmed = content.trim_start();
    if trimmed.is_empty() {
        return ContentKind::PlainText;
    }
    if looks_like_json(content) {
        return ContentKind::Json;
    }
    if looks_like_diff(content) {
        return ContentKind::Diff;
    }
    if looks_like_html(content) {
        return ContentKind::Html;
    }
    if search_line_ratio(content) >= 0.6 {
        return ContentKind::Search;
    }
    if looks_like_code(content) {
        return ContentKind::Code;
    }
    if log_line_ratio(content) >= 0.5 {
        return ContentKind::Log;
    }
    ContentKind::PlainText
}

/// True if `content` parses as a JSON array of objects (crusher input) or a
/// single non-trivial JSON object/array. Scalars and tiny payloads are ignored.
pub fn looks_like_json(content: &str) -> bool {
    let trimmed = content.trim_start();
    let first = trimmed.as_bytes().first().copied();
    if first != Some(b'[') && first != Some(b'{') {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed.trim_end()) {
        Ok(serde_json::Value::Array(items)) => {
            items.len() >= 2 && items.iter().any(|v| v.is_object())
        }
        // A standalone object is JSON worth routing when it has enough keys to
        // be worth pretty-handling (the JSON compressor declines small ones).
        Ok(serde_json::Value::Object(map)) => map.len() >= 2,
        _ => false,
    }
}

/// True if `content` parses as a JSON array of objects specifically (the table
/// crusher's strict input). Kept for callers that need the narrow check.
pub fn looks_like_json_array(content: &str) -> bool {
    let trimmed = content.trim_start();
    if !trimmed.starts_with('[') {
        return false;
    }
    matches!(
        serde_json::from_str::<serde_json::Value>(trimmed.trim_end()),
        Ok(serde_json::Value::Array(items)) if items.len() >= 2 && items.iter().any(|v| v.is_object())
    )
}

/// True if `content` looks like a unified diff: a `diff --git` header or at
/// least one hunk header (`@@ ... @@`).
pub fn looks_like_diff(content: &str) -> bool {
    for line in content.lines().take(400) {
        if line.starts_with("diff --git ") || line.starts_with("Index: ") {
            return true;
        }
        if line.starts_with("@@ ") && line[3..].contains("@@") {
            return true;
        }
    }
    false
}

/// True if `content` looks like an HTML document: a doctype / `<html`/`<body`
/// marker, or a high density of angle-bracket tags over the first lines.
pub fn looks_like_html(content: &str) -> bool {
    let head = &content[..content.len().min(8192)];
    let lower = head.to_ascii_lowercase();
    if lower.contains("<!doctype html")
        || lower.contains("<html")
        || lower.contains("<head>")
        || lower.contains("<body")
    {
        return true;
    }
    // Tag-density fallback: count `<tag ...>` openings over non-blank lines.
    let mut tags = 0usize;
    let mut lines = 0usize;
    for line in head.lines().take(200) {
        if line.trim().is_empty() {
            continue;
        }
        lines += 1;
        tags += count_html_tags(line);
    }
    lines >= 3 && (tags as f32 / lines as f32) >= 1.0
}

/// Cheap count of `<tag` / `</tag` openings in a line (an HTML signal).
fn count_html_tags(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' {
            let next = bytes[i + 1];
            if next.is_ascii_alphabetic() || next == b'/' || next == b'!' {
                count += 1;
            }
        }
        i += 1;
    }
    count
}

/// True if `content` heuristically looks like source code: a meaningful share
/// of lines carry code structure (keywords, braces, semicolons, indentation
/// with operators). Deliberately conservative — detection only fires for `Auto`
/// content with no extension/MIME hint, so false positives are rare.
pub fn looks_like_code(content: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "fn ",
        "function ",
        "class ",
        "def ",
        "impl ",
        "struct ",
        "enum ",
        "trait ",
        "interface ",
        "import ",
        "export ",
        "package ",
        "public ",
        "private ",
        "const ",
        "let ",
        "var ",
        "return ",
        "#include",
        "using ",
        "namespace ",
    ];
    let mut total = 0usize;
    let mut code_like = 0usize;
    let mut brace_lines = 0usize;
    for line in content.lines().take(400) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        total += 1;
        let has_kw = KEYWORDS.iter().any(|kw| line.contains(kw));
        let ends_struct = t.ends_with('{') || t.ends_with(';') || t.ends_with('}');
        if t.contains('{') || t.contains('}') {
            brace_lines += 1;
        }
        if has_kw || ends_struct {
            code_like += 1;
        }
    }
    if total < 5 {
        return false;
    }
    let ratio = code_like as f32 / total as f32;
    // Require both a decent code-signal ratio and some brace structure so prose
    // with the odd "return" or "class" doesn't trip it.
    ratio >= 0.4 && brace_lines >= 2
}

/// Fraction of non-empty lines that look like `path:line:...` search hits.
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
    use crate::openhuman::tokenjuice::compressors::signals::{severity, Severity};
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
/// Returns `None` for context lines and non-matches.
pub fn parse_search_line(line: &str) -> Option<(&str, u64, &str)> {
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

    fn hint() -> ContentHint {
        ContentHint::default()
    }

    #[test]
    fn explicit_override_wins() {
        let h = ContentHint {
            explicit: Some(ContentKind::Html),
            ..Default::default()
        };
        // Body is JSON but the explicit hint forces Html.
        assert_eq!(
            detect_content_kind(r#"[{"a":1},{"b":2}]"#, &h),
            ContentKind::Html
        );
    }

    #[test]
    fn mime_and_extension_route() {
        let h = ContentHint {
            mime: Some("application/json".into()),
            ..Default::default()
        };
        assert_eq!(
            detect_content_kind("{not even json}", &h),
            ContentKind::Json
        );
        let h = ContentHint {
            extension: Some("RS".into()),
            ..Default::default()
        };
        assert_eq!(detect_content_kind("anything", &h), ContentKind::Code);
    }

    #[test]
    fn tool_prior_routes_and_search_is_absolute() {
        let h = ContentHint::for_tool("grep");
        // Even a diff-looking body stays Search under a grep prior.
        assert_eq!(
            detect_content_kind("diff --git a/x b/x\n@@ -1 +1 @@", &h),
            ContentKind::Search
        );
        let h = ContentHint::for_tool("read_diff");
        assert_eq!(
            detect_content_kind("diff --git a/x b/x\n@@ -1 +1 @@\n+a", &h),
            ContentKind::Diff
        );
    }

    #[test]
    fn detect_search_results() {
        let c =
            "src/main.rs:42:fn process() {\nsrc/lib.rs:7:pub use foo;\nsrc/x.rs:99:    let y = 1;";
        assert_eq!(detect_content_kind(c, &hint()), ContentKind::Search);
    }

    #[test]
    fn detect_diff_json_log() {
        assert_eq!(
            detect_content_kind("diff --git a/x.rs b/x.rs\n@@ -1,3 +1,4 @@\n+added", &hint()),
            ContentKind::Diff
        );
        assert_eq!(
            detect_content_kind(r#"[{"id":1,"name":"a"},{"id":2,"name":"b"}]"#, &hint()),
            ContentKind::Json
        );
        assert_eq!(
            detect_content_kind(
                "Compiling foo\nwarning: unused\nerror[E0382]: borrow of moved value\nerror: aborting",
                &hint()
            ),
            ContentKind::Log
        );
    }

    #[test]
    fn detect_html() {
        let c = "<!DOCTYPE html>\n<html><head><title>x</title></head><body><p>hi</p></body></html>";
        assert_eq!(detect_content_kind(c, &hint()), ContentKind::Html);
    }

    #[test]
    fn detect_code() {
        let c = "use std::fmt;\n\npub fn add(a: i32, b: i32) -> i32 {\n    let c = a + b;\n    return c;\n}\n\nstruct Foo {\n    x: i32,\n}";
        assert_eq!(detect_content_kind(c, &hint()), ContentKind::Code);
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(
            detect_content_kind("just some prose about a topic at length here", &hint()),
            ContentKind::PlainText
        );
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
        assert_eq!(
            parse_search_line("pre-commit-config.yaml:3:foo"),
            Some(("pre-commit-config.yaml", 3, "foo"))
        );
        assert_eq!(parse_search_line("just a sentence"), None);
    }
}
