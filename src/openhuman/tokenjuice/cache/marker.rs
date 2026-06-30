//! CCR retrieval markers.
//!
//! When the router offloads an original to the [`super::store`], it embeds a
//! marker carrying the CCR token so the model knows the content is recoverable
//! and how to fetch it. The canonical marker is `⟦tj:<hash>⟧`; for backward
//! compatibility we also parse the legacy `retrieve_tool_output("<hash>")` form
//! that older histories may still contain.

/// The retrieve tool's name, surfaced in footers and used by the harness to
/// keep the tool's own output from being re-compacted and to always advertise it.
pub const RETRIEVE_TOOL_NAME: &str = "tokenjuice_retrieve";

/// The legacy retrieve tool name (kept as an alias during migration).
pub const LEGACY_RETRIEVE_TOOL_NAME: &str = "retrieve_tool_output";

/// All CCR recovery tool names. Both must be (a) always advertised to every
/// agent — any agent that sees a retrieval footer must be able to call the tool
/// — and (b) never re-compacted (their job is to return an original in full).
pub const RECOVERY_TOOL_NAMES: &[&str] = &[RETRIEVE_TOOL_NAME, LEGACY_RETRIEVE_TOOL_NAME];

/// Tools whose output must never be re-compacted. See [`RECOVERY_TOOL_NAMES`].
pub const NEVER_COMPACT_TOOLS: &[&str] = RECOVERY_TOOL_NAMES;

/// True if `tool_name` is one of the CCR recovery tools.
pub fn is_recovery_tool(tool_name: &str) -> bool {
    RECOVERY_TOOL_NAMES.contains(&tool_name)
}

/// Format the canonical inline marker for a CCR `hash`.
pub fn format_marker(hash: &str) -> String {
    format!("⟦tj:{hash}⟧")
}

/// Build the human-facing recovery footer appended to compacted output.
///
/// `lossy` distinguishes a partial view (data dropped) from a faithful reformat
/// (no data lost, layout changed); both offer exact recovery.
pub fn recovery_footer(hash: &str, original_bytes: usize, lossy: bool) -> String {
    let marker = format_marker(hash);
    if lossy {
        format!(
            "\n\n[compacted tool output — this is a PARTIAL view; the full original \
             ({original_bytes} bytes) is available by calling {RETRIEVE_TOOL_NAME} with \
             token \"{hash}\" (marker {marker})]"
        )
    } else {
        format!(
            "\n\n[reformatted tool output — no data lost, but layout changed; the exact \
             original ({original_bytes} bytes) is available by calling {RETRIEVE_TOOL_NAME} \
             with token \"{hash}\" (marker {marker})]"
        )
    }
}

/// Extract all CCR tokens referenced in `text`, from both the canonical
/// `⟦tj:<hash>⟧` markers and the legacy `retrieve_tool_output("<hash>")` form.
/// Order-preserving, de-duplicated.
pub fn parse_markers(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |h: &str| {
        let h = h.trim();
        if !h.is_empty() && !out.iter().any(|e| e == h) {
            out.push(h.to_string());
        }
    };

    // Canonical: ⟦tj:HASH⟧
    let mut rest = text;
    while let Some(start) = rest.find("⟦tj:") {
        let after = &rest[start + "⟦tj:".len()..];
        if let Some(end) = after.find('⟧') {
            push(&after[..end]);
            rest = &after[end..];
        } else {
            break;
        }
    }

    // Legacy: retrieve_tool_output("HASH") or tokenjuice_retrieve("HASH")
    for needle in [
        "retrieve_tool_output(\"",
        "tokenjuice_retrieve(\"",
        "token \"",
    ] {
        let mut rest = text;
        while let Some(start) = rest.find(needle) {
            let after = &rest[start + needle.len()..];
            if let Some(end) = after.find('"') {
                push(&after[..end]);
                rest = &after[end..];
            } else {
                break;
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_parses_canonical() {
        let m = format_marker("ab12cd34");
        assert_eq!(m, "⟦tj:ab12cd34⟧");
        assert_eq!(
            parse_markers(&format!("see {m} for more")),
            vec!["ab12cd34"]
        );
    }

    #[test]
    fn parses_legacy_form() {
        let text = "partial; call retrieve_tool_output(\"deadbeef00\") to recover";
        assert_eq!(parse_markers(text), vec!["deadbeef00"]);
    }

    #[test]
    fn parses_multiple_dedup() {
        let text = "⟦tj:aaa⟧ and ⟦tj:bbb⟧ and again ⟦tj:aaa⟧";
        assert_eq!(parse_markers(text), vec!["aaa", "bbb"]);
    }

    #[test]
    fn footer_carries_token() {
        let f = recovery_footer("c0ffee", 1234, true);
        assert!(f.contains("PARTIAL view"));
        assert!(f.contains("c0ffee"));
        assert_eq!(parse_markers(&f), vec!["c0ffee"]);
    }
}
