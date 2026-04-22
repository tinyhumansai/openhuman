//! Title formatting shared between core and the Tauri shell.
//!
//! Why the prefix: embedded webviews (Slack, Discord, Gmail) may be
//! open alongside the user's locally-installed native apps for the
//! same service. Both would fire OS toasts for the same DM. Prefixing
//! the title with `OpenHuman:` makes it trivial for the user to tell
//! the two apart and also gives the OS notification centre a distinct
//! grouping key.

/// Prefix applied to every OS notification title fired by a webview
/// event. Trailing space so the separation from the raw title reads
/// naturally (`OpenHuman: New message from …`).
pub const OPENHUMAN_TITLE_PREFIX: &str = "OpenHuman: ";

/// Format the native-toast title for a webview notification.
///
/// `provider_label` is the human-readable provider name (e.g. `Slack`),
/// `raw_title` is the renderer-supplied title (may be empty).
///
/// Layout: `OpenHuman: <Provider> — <raw title>` when both pieces are
/// present, collapsing to `OpenHuman: <Provider>` when the raw title is
/// empty or whitespace-only.
pub fn format_title(provider_label: &str, raw_title: &str) -> String {
    let raw = raw_title.trim();
    if raw.is_empty() {
        format!("{OPENHUMAN_TITLE_PREFIX}{provider_label}")
    } else {
        format!("{OPENHUMAN_TITLE_PREFIX}{provider_label} — {raw}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_empty_title_falls_back_to_provider_only() {
        assert_eq!(format_title("Slack", ""), "OpenHuman: Slack");
        assert_eq!(format_title("Slack", "   "), "OpenHuman: Slack");
    }

    #[test]
    fn prefix_with_title_joins_with_em_dash() {
        assert_eq!(
            format_title("Gmail", "New mail from Alice"),
            "OpenHuman: Gmail — New mail from Alice"
        );
    }

    #[test]
    fn prefix_trims_raw_title_whitespace() {
        assert_eq!(
            format_title("Discord", "  ping  "),
            "OpenHuman: Discord — ping"
        );
    }
}
