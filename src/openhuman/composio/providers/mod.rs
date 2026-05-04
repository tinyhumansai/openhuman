//! Provider-specific code for Composio toolkits.
//!
//! Each Composio toolkit (gmail, notion, slack, …) can register a
//! [`ComposioProvider`] implementation that knows how to:
//!
//!   * Fetch a normalized **user profile** for a connected account.
//!   * Run an **initial / periodic sync** that pulls fresh data from the
//!     upstream service via the backend-proxied
//!     [`ComposioClient`](super::client::ComposioClient).
//!   * React to **trigger webhooks** that arrive over the
//!     `composio:trigger` Socket.IO bridge.
//!   * React to **OAuth handoff completion** so the very first sync can
//!     run as soon as a user connects an account.
//!
//! Providers are pure Rust — there is no JS sandbox involved. They are
//! the native counterpart to the QuickJS skill bundles in
//! `tinyhumansai/openhuman-skills`, but specialized for Composio's API
//! surface and run inside the core process directly.
//!
//! ## Registry & dispatch
//!
//! The [`registry`] module owns a process-global `HashMap<toolkit_slug,
//! Arc<dyn ComposioProvider>>`. The composio event bus subscriber
//! ([`super::bus::ComposioTriggerSubscriber`]) and the periodic sync
//! task both look up providers by toolkit slug and call into them.
//!
//! ## Why a trait, not a giant `match`
//!
//! Each provider has provider-specific shapes (gmail returns
//! emailAddress + messagesTotal, notion returns workspaces + pages, …)
//! and a different idea of what "sync" means. A trait keeps each
//! provider's implementation isolated, individually testable, and
//! easy to add without touching the dispatch layer.

mod descriptions;
pub(crate) mod helpers;
pub mod tool_scope;
mod traits;
mod types;
pub mod user_scopes;

pub mod catalogs;
pub mod catalogs_business;
pub mod catalogs_google;
pub mod catalogs_messaging;
pub mod catalogs_productivity;
pub mod catalogs_social_media;
pub mod github;
pub mod gmail;
pub mod notion;
pub mod profile;
pub mod registry;
pub mod slack;
pub mod sync_state;

/// Static toolkit → curated catalog map.
///
/// This is consulted by the meta-tool layer alongside any registered
/// provider's [`ComposioProvider::curated_tools`]. It lets toolkits
/// without a full native provider (e.g. `github`, which has no sync
/// logic yet) still benefit from curated whitelisting.
///
/// Lookup key is the lowercased prefix returned by
/// [`toolkit_from_slug`] applied to the action slug — e.g.
/// `GOOGLECALENDAR_CREATE_EVENT` → `"googlecalendar"`. Multi-segment
/// prefixes like `MICROSOFT_TEAMS_*` are matched via their first
/// segment with an extra arm.
/// Synchronous visibility check for a Composio action slug given a
/// pre-loaded user scope preference.
///
/// Returns `true` if the action should appear in the agent's tool
/// surface — i.e. it's in the toolkit's curated whitelist (or the
/// toolkit has no curation) **and** the user's scope pref allows its
/// classification. Falls back to [`classify_unknown`] for un-curated
/// toolkits.
///
/// Use this when the user pref has already been loaded for the
/// toolkit (typical inside a `for slug in toolkits {...}` loop where
/// awaiting once per toolkit is cheaper than once per action).
pub fn is_action_visible_with_pref(slug: &str, pref: &UserScopePref) -> bool {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        return true;
    };
    let catalog = get_provider(&toolkit)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(&toolkit));
    match catalog {
        Some(catalog) => match find_curated(catalog, slug) {
            Some(curated) => pref.allows(curated.scope),
            None => false,
        },
        None => pref.allows(classify_unknown(slug)),
    }
}

pub fn catalog_for_toolkit(toolkit: &str) -> Option<&'static [CuratedTool]> {
    match toolkit.trim().to_ascii_lowercase().as_str() {
        // Native providers
        "gmail" => Some(gmail::GMAIL_CURATED),
        "notion" => Some(notion::NOTION_CURATED),
        "github" => Some(github::GITHUB_CURATED),
        // Catalog-only toolkits
        "slack" => Some(catalogs::SLACK_CURATED),
        "discord" => Some(catalogs::DISCORD_CURATED),
        "googlecalendar" | "google_calendar" => Some(catalogs::GOOGLECALENDAR_CURATED),
        "googledrive" | "google_drive" => Some(catalogs::GOOGLEDRIVE_CURATED),
        "googledocs" | "google_docs" => Some(catalogs::GOOGLEDOCS_CURATED),
        "googlesheets" | "google_sheets" => Some(catalogs::GOOGLESHEETS_CURATED),
        "outlook" => Some(catalogs::OUTLOOK_CURATED),
        // MICROSOFT_TEAMS_* slugs extract to "microsoft" via toolkit_from_slug.
        "microsoft" | "microsoft_teams" => Some(catalogs::MICROSOFT_TEAMS_CURATED),
        "linear" => Some(catalogs::LINEAR_CURATED),
        "jira" => Some(catalogs::JIRA_CURATED),
        "trello" => Some(catalogs::TRELLO_CURATED),
        "asana" => Some(catalogs::ASANA_CURATED),
        "dropbox" => Some(catalogs::DROPBOX_CURATED),
        "twitter" => Some(catalogs::TWITTER_CURATED),
        "spotify" => Some(catalogs::SPOTIFY_CURATED),
        "telegram" => Some(catalogs::TELEGRAM_CURATED),
        "whatsapp" => Some(catalogs::WHATSAPP_CURATED),
        "shopify" => Some(catalogs::SHOPIFY_CURATED),
        "stripe" => Some(catalogs::STRIPE_CURATED),
        "hubspot" => Some(catalogs::HUBSPOT_CURATED),
        "salesforce" => Some(catalogs::SALESFORCE_CURATED),
        "airtable" => Some(catalogs::AIRTABLE_CURATED),
        "figma" => Some(catalogs::FIGMA_CURATED),
        "youtube" => Some(catalogs::YOUTUBE_CURATED),
        _ => None,
    }
}

pub use descriptions::toolkit_description;
pub(crate) use helpers::pick_str;
pub use registry::{
    all_providers, get_provider, init_default_providers, register_provider, ProviderArc,
};
pub use tool_scope::{classify_unknown, find_curated, toolkit_from_slug, CuratedTool, ToolScope};
pub use traits::ComposioProvider;
pub use types::{ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason};
pub use user_scopes::{load_or_default as load_user_scope_or_default, UserScopePref};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pick_str_finds_first_non_empty_match() {
        let v = json!({
            "data": { "user": { "email": "  user@example.com  ", "name": "" } },
            "fallback": "fallback@example.com"
        });
        // first path empty -> falls through
        assert_eq!(
            pick_str(&v, &["data.user.name", "data.user.email"]),
            Some("user@example.com".to_string())
        );
        // missing path -> falls through to fallback
        assert_eq!(
            pick_str(&v, &["data.missing", "fallback"]),
            Some("fallback@example.com".to_string())
        );
        // nothing matches
        assert_eq!(pick_str(&v, &["nope.nope"]), None);
    }

    #[test]
    fn sync_outcome_elapsed_ms_is_safe_when_finish_lt_start() {
        let mut o = SyncOutcome::default();
        o.started_at_ms = 100;
        o.finished_at_ms = 50;
        assert_eq!(o.elapsed_ms(), 0);
        o.finished_at_ms = 250;
        assert_eq!(o.elapsed_ms(), 150);
    }

    #[test]
    fn pick_str_returns_none_for_non_string_values() {
        let v = json!({ "count": 42, "flag": true, "empty": "", "whitespace": "   " });
        assert_eq!(pick_str(&v, &["count"]), None);
        assert_eq!(pick_str(&v, &["flag"]), None);
        assert_eq!(pick_str(&v, &["empty"]), None);
        assert_eq!(pick_str(&v, &["whitespace"]), None);
    }

    #[test]
    fn pick_str_respects_path_order() {
        let v = json!({ "a": "first", "b": "second" });
        assert_eq!(pick_str(&v, &["a", "b"]), Some("first".into()));
        assert_eq!(pick_str(&v, &["b", "a"]), Some("second".into()));
    }

    #[test]
    fn sync_reason_as_str_matches_enum_variant() {
        assert_eq!(SyncReason::ConnectionCreated.as_str(), "connection_created");
        assert_eq!(SyncReason::Periodic.as_str(), "periodic");
        assert_eq!(SyncReason::Manual.as_str(), "manual");
    }

    #[test]
    fn sync_reason_serde_is_snake_case() {
        let s = serde_json::to_string(&SyncReason::ConnectionCreated).unwrap();
        assert_eq!(s, "\"connection_created\"");
        let back: SyncReason = serde_json::from_str(&s).unwrap();
        assert_eq!(back, SyncReason::ConnectionCreated);
    }

    #[test]
    fn toolkit_description_known_slugs_are_distinct_and_non_empty() {
        let known = [
            "gmail",
            "notion",
            "github",
            "slack",
            "discord",
            "google_calendar",
            "google_drive",
            "google_docs",
            "google_sheets",
            "outlook",
            "microsoft_teams",
            "linear",
            "jira",
            "trello",
            "asana",
            "dropbox",
            "twitter",
            "spotify",
            "telegram",
            "whatsapp",
            "twilio",
            "shopify",
            "stripe",
            "hubspot",
            "salesforce",
            "airtable",
            "figma",
            "youtube",
            "calendar",
        ];
        let fallback = toolkit_description("__definitely_unknown_slug__");
        for slug in known {
            let desc = toolkit_description(slug);
            assert!(!desc.is_empty(), "{slug} description must not be empty");
            assert_ne!(
                desc, fallback,
                "known slug `{slug}` must not map to the generic fallback"
            );
        }
    }

    #[test]
    fn toolkit_description_unknown_slug_uses_generic_fallback() {
        assert_eq!(
            toolkit_description("not_a_real_toolkit_123"),
            "Interact with this connected service via its available actions"
        );
        assert_eq!(
            toolkit_description(""),
            "Interact with this connected service via its available actions"
        );
    }

    #[test]
    fn toolkit_description_is_case_sensitive() {
        // The match is lowercase-only by convention; an uppercase slug
        // should fall through to the generic description. Explicitly
        // documenting this guards against accidental case-insensitive
        // matching sneaking in later.
        let fallback = toolkit_description("__fallback__");
        assert_eq!(toolkit_description("GMAIL"), fallback);
        assert_eq!(toolkit_description("Notion"), fallback);
    }

    #[test]
    fn provider_user_profile_default_is_empty() {
        let p = ProviderUserProfile::default();
        assert!(p.toolkit.is_empty());
        assert!(p.connection_id.is_none());
        assert!(p.display_name.is_none());
        assert!(p.email.is_none());
        assert!(p.username.is_none());
        assert!(p.avatar_url.is_none());
        assert!(p.profile_url.is_none());
        assert!(p.extras.is_null());
    }
}
