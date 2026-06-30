//! Tool-name → content-prior mapping for the content router.
//!
//! The producing tool name is a strong prior on what kind of content a blob
//! holds, so the detector doesn't have to work from scratch for the common
//! case. Ported from the compaction router (`hint_for_tool`) and folded into
//! the richer [`ContentHint`] the new router uses.

use crate::openhuman::tokenjuice::types::ContentKind;

/// A coarse prior derived purely from the producing tool name. `Auto` means
/// "no strong prior — run full structural detection".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPrior {
    Search,
    Log,
    Diff,
    Json,
    Auto,
}

/// Map an agent-level tool name to its content prior. Unknown tools fall
/// through to [`ToolPrior::Auto`].
///
/// `shell` is deliberately `Auto` — its output is frequently NOT a log (a
/// `find`, a `seq`, a `cat` of CSV, a script printing a list), so it is routed
/// through detection rather than forced to the log compressor.
pub fn tool_prior(tool_name: &str) -> ToolPrior {
    match tool_name {
        "grep" | "glob_search" | "ripgrep" | "rg" => ToolPrior::Search,
        "run_tests" | "run_linter" | "npm_exec" | "node_exec" | "install_tool" | "lsp" => {
            ToolPrior::Log
        }
        "read_diff" | "git_operations" => ToolPrior::Diff,
        _ => ToolPrior::Auto,
    }
}

/// Translate a strong tool prior straight into a [`ContentKind`] without
/// structural detection. Returns `None` for `Auto` (detector must decide).
pub fn prior_to_kind(prior: ToolPrior) -> Option<ContentKind> {
    match prior {
        ToolPrior::Search => Some(ContentKind::Search),
        ToolPrior::Log => Some(ContentKind::Log),
        ToolPrior::Diff => Some(ContentKind::Diff),
        ToolPrior::Json => Some(ContentKind::Json),
        ToolPrior::Auto => None,
    }
}

/// Map a file extension (no dot, lower-cased by the caller) to a content kind,
/// when it is unambiguous. Returns `None` for extensions that don't pin a kind.
pub fn extension_to_kind(ext: &str) -> Option<ContentKind> {
    match ext {
        "json" | "jsonl" | "ndjson" => Some(ContentKind::Json),
        "html" | "htm" | "xhtml" => Some(ContentKind::Html),
        "diff" | "patch" => Some(ContentKind::Diff),
        "log" => Some(ContentKind::Log),
        // Source-code extensions we recognise for the code compressor. Grammar
        // availability is checked later by the compressor; unknown languages
        // fall back to the brace-depth heuristic.
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "pyi" | "go" | "java"
        | "kt" | "kts" | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "rb" | "php" | "swift"
        | "scala" | "cs" => Some(ContentKind::Code),
        _ => None,
    }
}

/// Map a MIME type to a content kind, when unambiguous. Tolerates a `; charset`
/// suffix.
pub fn mime_to_kind(mime: &str) -> Option<ContentKind> {
    let base = mime.split(';').next().unwrap_or(mime).trim();
    match base {
        "application/json" | "text/json" => Some(ContentKind::Json),
        "text/html" | "application/xhtml+xml" => Some(ContentKind::Html),
        "text/x-diff" | "text/x-patch" => Some(ContentKind::Diff),
        _ => {
            // application/<lang> and text/x-<lang> source types → Code.
            if base.starts_with("text/x-") || base == "application/javascript" {
                Some(ContentKind::Code)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_priors_map_known_tools() {
        assert_eq!(tool_prior("grep"), ToolPrior::Search);
        assert_eq!(tool_prior("run_tests"), ToolPrior::Log);
        assert_eq!(tool_prior("read_diff"), ToolPrior::Diff);
        assert_eq!(tool_prior("file_read"), ToolPrior::Auto);
        assert_eq!(tool_prior("shell"), ToolPrior::Auto);
    }

    #[test]
    fn extensions_map() {
        assert_eq!(extension_to_kind("rs"), Some(ContentKind::Code));
        assert_eq!(extension_to_kind("json"), Some(ContentKind::Json));
        assert_eq!(extension_to_kind("html"), Some(ContentKind::Html));
        assert_eq!(extension_to_kind("patch"), Some(ContentKind::Diff));
        assert_eq!(extension_to_kind("xyz"), None);
    }

    #[test]
    fn mimes_map() {
        assert_eq!(mime_to_kind("application/json"), Some(ContentKind::Json));
        assert_eq!(
            mime_to_kind("text/html; charset=utf-8"),
            Some(ContentKind::Html)
        );
        assert_eq!(mime_to_kind("text/x-rust"), Some(ContentKind::Code));
        assert_eq!(mime_to_kind("text/plain"), None);
    }
}
