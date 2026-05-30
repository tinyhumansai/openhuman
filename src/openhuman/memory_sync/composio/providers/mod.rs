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
mod scope_lookup;
pub mod tool_scope;
mod traits;
mod types;
pub mod user_scopes;

pub mod catalogs;
pub mod catalogs_business;
pub mod catalogs_google;
pub mod catalogs_messaging;
pub mod catalogs_microsoft;
pub mod catalogs_productivity;
pub mod catalogs_social_media;
pub mod clickup;
pub mod github;
pub mod gmail;
pub mod linear;
pub mod notion;
pub mod profile;
pub mod profile_md;
pub mod registry;
pub mod slack;
pub mod sync_state;

use crate::openhuman::composio::types::ComposioCapability;

const CAPABILITY_TOOLKITS: &[&str] = &[
    "gmail",
    "notion",
    "slack",
    "clickup",
    "github",
    "discord",
    "googlecalendar",
    "googledrive",
    "googledocs",
    "googlesheets",
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
    "shopify",
    "stripe",
    "hubspot",
    "salesforce",
    "airtable",
    "figma",
    "youtube",
    "one_drive",
    "excel",
    "todoist",
];

fn native_provider_sync_interval(toolkit: &str) -> Option<u64> {
    match toolkit {
        "gmail" => Some(gmail::GmailProvider::new().sync_interval_secs()),
        "notion" => Some(notion::NotionProvider::new().sync_interval_secs()),
        "slack" => Some(slack::SlackProvider::new().sync_interval_secs()),
        "clickup" => Some(clickup::ClickUpProvider::new().sync_interval_secs()),
        "github" => Some(github::GitHubProvider::new().sync_interval_secs()),
        "linear" => Some(linear::LinearProvider::new().sync_interval_secs()),
        _ => None,
    }
    .flatten()
}

fn has_native_provider(toolkit: &str) -> bool {
    matches!(
        toolkit,
        "gmail" | "notion" | "slack" | "clickup" | "github" | "linear"
    )
}

/// Static overview of the Composio integrations supported by this core build.
///
/// This deliberately does not consult the live Composio backend/direct tenant:
/// it is an observability surface for OpenHuman's own capability tiers. Use
/// `composio_list_toolkits` / `composio_list_connections` when callers need
/// the currently signed-in user's allowlist or OAuth state.
pub fn capability_matrix() -> Vec<ComposioCapability> {
    CAPABILITY_TOOLKITS
        .iter()
        .map(|toolkit| {
            let native_provider = has_native_provider(toolkit);
            let catalog = catalog_for_toolkit(toolkit);
            let sync_interval_secs = native_provider_sync_interval(toolkit);
            ComposioCapability {
                toolkit: (*toolkit).to_string(),
                description: toolkit_description(toolkit).to_string(),
                native_provider,
                curated_tools: catalog.is_some(),
                curated_tool_count: catalog.map_or(0, <[CuratedTool]>::len),
                tool_execution: catalog.is_some(),
                user_profile: native_provider,
                initial_sync: native_provider,
                periodic_sync: sync_interval_secs.is_some(),
                sync_interval_secs,
                trigger_webhooks: native_provider,
                memory_ingest: native_provider,
            }
        })
        .collect()
}

/// Static toolkit → curated catalog map.
///
/// This is consulted by the meta-tool layer alongside any registered
/// provider's [`ComposioProvider::curated_tools`]. It lets toolkits
/// without a full native provider still benefit from curated
/// whitelisting.
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
        "linear" => Some(linear::LINEAR_CURATED),
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
        "jira" => Some(catalogs::JIRA_CURATED),
        "trello" => Some(catalogs::TRELLO_CURATED),
        "asana" => Some(catalogs::ASANA_CURATED),
        "clickup" => Some(clickup::CLICKUP_CURATED),
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
        // ONE_DRIVE_* slugs extract to "one" via toolkit_from_slug;
        // alias both the prefix and the canonical UI/backend slugs.
        "one" | "one_drive" | "onedrive" => Some(catalogs::ONE_DRIVE_CURATED),
        "excel" => Some(catalogs::EXCEL_CURATED),
        "todoist" => Some(catalogs::TODOIST_CURATED),
        _ => None,
    }
}

/// All toolkit slugs that have a curated agent-ready catalog.
///
/// Source of truth for the UI "preview / agent integration coming
/// soon" badge: any connected toolkit whose slug is NOT in this list
/// can be authorized but lacks a curated tool surface, so the agent
/// can't use it productively.
///
/// Returned in sorted order to keep the RPC response stable across
/// builds.
pub fn agent_ready_toolkits() -> Vec<&'static str> {
    let mut slugs: Vec<&'static str> = vec![
        // Native providers
        "gmail",
        "notion",
        "github",
        // Catalog-only toolkits
        "slack",
        "discord",
        "googlecalendar",
        "googledrive",
        "googledocs",
        "googlesheets",
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
        "shopify",
        "stripe",
        "hubspot",
        "salesforce",
        "airtable",
        "figma",
        "youtube",
        "one_drive",
        "excel",
        "todoist",
    ];
    slugs.sort_unstable();
    slugs
}

pub use descriptions::toolkit_description;
pub(crate) use helpers::{first_array_str, merge_extra, pick_str};
pub use registry::{
    all_providers, get_provider, init_default_providers, register_provider, ProviderArc,
};
pub use scope_lookup::{curated_scope_for, toolkit_has_scope};
pub use tool_scope::{classify_unknown, find_curated, toolkit_from_slug, CuratedTool, ToolScope};
pub use traits::ComposioProvider;
pub use types::{
    NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason, TaskFetchFilter,
};
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

    // Note: `toolkit_has_scope` tests now live in `scope_lookup.rs`
    // alongside the implementation.

    #[test]
    fn catalog_for_toolkit_resolves_new_microsoft_and_todoist_slugs() {
        // Newly added catalogs (#2283): OneDrive, Excel, Todoist must be
        // discoverable both by their canonical UI slug AND by the
        // prefix that `toolkit_from_slug` extracts from action slugs.
        assert!(catalog_for_toolkit("one_drive").is_some());
        assert!(catalog_for_toolkit("onedrive").is_some());
        // ONE_DRIVE_GET_FILE → toolkit_from_slug() → "one"
        assert!(catalog_for_toolkit("one").is_some());
        assert!(catalog_for_toolkit("excel").is_some());
        assert!(catalog_for_toolkit("todoist").is_some());
    }

    #[test]
    fn agent_ready_toolkits_includes_new_catalogs_and_is_sorted() {
        let slugs = agent_ready_toolkits();
        assert!(slugs.contains(&"one_drive"));
        assert!(slugs.contains(&"excel"));
        assert!(slugs.contains(&"todoist"));
        // Spot-check legacy entries still present.
        assert!(slugs.contains(&"gmail"));
        assert!(slugs.contains(&"slack"));
        // Uncurated toolkit must NOT appear — guarantees the UI badge
        // logic can rely on this set to flag "preview" toolkits.
        assert!(!slugs.contains(&"sharepoint"));
        assert!(!slugs.contains(&"clickup"));
        // Stable order across builds — the RPC consumer caches it.
        let mut expected = slugs.clone();
        expected.sort_unstable();
        assert_eq!(slugs, expected);
    }

    #[test]
    fn capability_matrix_includes_new_catalog_only_toolkits() {
        let matrix = capability_matrix();
        for slug in ["one_drive", "excel", "todoist"] {
            let row = matrix
                .iter()
                .find(|entry| entry.toolkit == slug)
                .unwrap_or_else(|| panic!("{slug} capability row missing"));
            assert!(!row.native_provider, "{slug} should not be native");
            assert!(row.curated_tools, "{slug} should be catalogued");
            assert!(
                row.curated_tool_count > 0,
                "{slug} catalog should be non-empty"
            );
            assert!(
                row.tool_execution,
                "{slug} tool execution should be enabled"
            );
            // No profile/sync/memory ingest — catalog-only.
            assert!(!row.user_profile);
            assert!(!row.initial_sync);
            assert!(!row.periodic_sync);
            assert!(!row.memory_ingest);
        }
    }

    #[test]
    fn capability_matrix_distinguishes_native_from_catalog_only_toolkits() {
        let matrix = capability_matrix();

        let gmail = matrix
            .iter()
            .find(|entry| entry.toolkit == "gmail")
            .expect("gmail capability row");
        assert!(gmail.native_provider);
        assert!(gmail.curated_tools);
        assert!(gmail.curated_tool_count > 0);
        assert!(gmail.user_profile);
        assert!(gmail.initial_sync);
        assert!(gmail.periodic_sync);
        assert_eq!(gmail.sync_interval_secs, Some(15 * 60));
        assert!(gmail.trigger_webhooks);
        assert!(gmail.memory_ingest);

        let google_calendar = matrix
            .iter()
            .find(|entry| entry.toolkit == "googlecalendar")
            .expect("googlecalendar capability row");
        assert!(!google_calendar.native_provider);
        assert!(google_calendar.curated_tools);
        assert!(google_calendar.curated_tool_count > 0);
        assert!(google_calendar.tool_execution);
        assert!(!google_calendar.user_profile);
        assert!(!google_calendar.initial_sync);
        assert!(!google_calendar.periodic_sync);
        assert_eq!(google_calendar.sync_interval_secs, None);
        assert!(!google_calendar.memory_ingest);
    }

    #[test]
    fn capability_matrix_includes_clickup_as_native_memory_provider() {
        // Locks in the per-issue #2288 registration: a ClickUp row must
        // appear in the capability matrix with the same native-provider
        // flags Gmail/Notion/Slack already carry (`memory_ingest`,
        // `periodic_sync`, non-zero `sync_interval_secs`). If a future
        // change drops one of the four registration touchpoints
        // (CAPABILITY_TOOLKITS, has_native_provider,
        // native_provider_sync_interval, catalog_for_toolkit) this test
        // fails loud rather than silently degrading the provider to
        // catalog-only status.
        let matrix = capability_matrix();
        let clickup = matrix
            .iter()
            .find(|entry| entry.toolkit == "clickup")
            .expect("clickup capability row");
        assert!(clickup.native_provider, "clickup must be native");
        assert!(clickup.curated_tools, "clickup must have a curated catalog");
        assert!(
            clickup.curated_tool_count > 0,
            "clickup catalog must be non-empty"
        );
        assert!(clickup.user_profile);
        assert!(clickup.initial_sync);
        assert!(clickup.periodic_sync);
        assert_eq!(clickup.sync_interval_secs, Some(30 * 60));
        assert!(clickup.memory_ingest);
    }

    #[test]
    fn capability_matrix_includes_linear_as_native_memory_provider() {
        // Per-issue #2400 registration: a Linear row must appear in
        // the capability matrix as a native memory-ingest provider,
        // matching gmail / notion / slack / clickup. If a future
        // change drops one of the five registration touchpoints
        // (CAPABILITY_TOOLKITS, has_native_provider,
        // native_provider_sync_interval, catalog_for_toolkit,
        // toolkit_description) this test fails loud rather than
        // silently degrading the provider to catalog-only status.
        let matrix = capability_matrix();
        let linear = matrix
            .iter()
            .find(|entry| entry.toolkit == "linear")
            .expect("linear capability row");
        assert!(linear.native_provider, "linear must be native");
        assert!(linear.curated_tools, "linear must have a curated catalog");
        assert!(
            linear.curated_tool_count > 0,
            "linear catalog must be non-empty"
        );
        assert!(linear.user_profile);
        assert!(linear.initial_sync);
        assert!(linear.periodic_sync);
        assert_eq!(linear.sync_interval_secs, Some(30 * 60));
        assert!(linear.memory_ingest);
    }

    #[test]
    fn capability_matrix_includes_github_as_native_memory_provider() {
        let matrix = capability_matrix();
        let github = matrix
            .iter()
            .find(|entry| entry.toolkit == "github")
            .expect("github capability row");
        assert!(github.native_provider, "github must be native");
        assert!(github.curated_tools, "github must have a curated catalog");
        assert!(
            github.curated_tool_count > 0,
            "github catalog must be non-empty"
        );
        assert!(github.user_profile);
        assert!(github.initial_sync);
        assert!(github.periodic_sync);
        assert_eq!(github.sync_interval_secs, Some(30 * 60));
        assert!(github.memory_ingest);
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
