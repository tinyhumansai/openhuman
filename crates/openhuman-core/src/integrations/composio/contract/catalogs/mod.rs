//! The curated Composio tool catalogs, and the lookups over them.
//!
//! Composio publishes 60+ actions per toolkit; most are noise for an agent's
//! planning loop. Each toolkit gets a hand-curated `&'static [CuratedTool]`
//! slice that pares the surface down to a useful subset and tags every action
//! with a [`ToolScope`], so per-user scope preferences can gate execution.
//!
//! The host is their only reader: it filters the agent's visible tool list,
//! renders the `gated_tools` unlock hints, and decides which connected
//! toolkits get the "agent-ready" badge.

pub mod business;
pub mod clickup;
pub mod descriptions;
pub mod github;
pub mod gmail;
pub mod google;
pub mod linear;
pub mod messaging;
pub mod microsoft;
pub mod notion;
pub mod productivity;
pub mod social_media;

use super::scopes::{
    classify_unknown, find_curated, toolkit_from_slug, CuratedTool, ToolScope, UserScopePref,
};

pub use descriptions::toolkit_description;

/// Toolkits the connector module can sync into memory (`tinyconnectors-sync`'s
/// provider tree).
pub const NATIVE_PROVIDERS: &[&str] = &["gmail", "notion", "slack", "clickup", "github", "linear"];

/// Does `toolkit` have a native sync provider?
#[must_use]
pub fn has_native_provider(toolkit: &str) -> bool {
    NATIVE_PROVIDERS.contains(&toolkit)
}

/// Static toolkit → curated catalog map.
///
/// The lookup key is the lowercased prefix [`toolkit_from_slug`] returns for an
/// action slug — `GOOGLECALENDAR_CREATE_EVENT` → `"googlecalendar"`.
/// Multi-segment prefixes like `MICROSOFT_TEAMS_*` return their known toolkit
/// slug.
#[must_use]
pub fn catalog_for_toolkit(toolkit: &str) -> Option<&'static [CuratedTool]> {
    match toolkit.trim().to_ascii_lowercase().as_str() {
        // Toolkits with a native provider. Each provider's `curated_tools()`
        // returns this same slice.
        "gmail" => Some(gmail::GMAIL_CURATED),
        "notion" => Some(notion::NOTION_CURATED),
        "github" => Some(github::GITHUB_CURATED),
        "linear" => Some(linear::LINEAR_CURATED),
        "clickup" => Some(clickup::CLICKUP_CURATED),
        "slack" => Some(messaging::SLACK_CURATED),
        // Catalog-only toolkits.
        "discord" => Some(messaging::DISCORD_CURATED),
        "googlecalendar" | "google_calendar" => Some(google::GOOGLECALENDAR_CURATED),
        "googledrive" | "google_drive" => Some(google::GOOGLEDRIVE_CURATED),
        "googledocs" | "google_docs" => Some(google::GOOGLEDOCS_CURATED),
        "googlesheets" | "google_sheets" => Some(google::GOOGLESHEETS_CURATED),
        "outlook" => Some(productivity::OUTLOOK_CURATED),
        // The legacy "microsoft" alias stays while `toolkit_from_slug` returns
        // the precise "microsoft_teams" slug for Teams actions.
        "microsoft" | "microsoft_teams" => Some(messaging::MICROSOFT_TEAMS_CURATED),
        "jira" => Some(productivity::JIRA_CURATED),
        "trello" => Some(productivity::TRELLO_CURATED),
        "asana" => Some(productivity::ASANA_CURATED),
        "dropbox" => Some(productivity::DROPBOX_CURATED),
        "twitter" => Some(social_media::TWITTER_CURATED),
        "spotify" => Some(social_media::SPOTIFY_CURATED),
        "telegram" => Some(messaging::TELEGRAM_CURATED),
        "whatsapp" => Some(messaging::WHATSAPP_CURATED),
        "shopify" => Some(business::SHOPIFY_CURATED),
        "stripe" => Some(business::STRIPE_CURATED),
        "hubspot" => Some(business::HUBSPOT_CURATED),
        "salesforce" => Some(business::SALESFORCE_CURATED),
        "airtable" => Some(business::AIRTABLE_CURATED),
        "figma" => Some(business::FIGMA_CURATED),
        "youtube" => Some(social_media::YOUTUBE_CURATED),
        // `ONE_DRIVE_*` slugs extract to "one" via `toolkit_from_slug`; alias
        // both the prefix and the canonical UI/backend slugs.
        "one" | "one_drive" | "onedrive" => Some(microsoft::ONE_DRIVE_CURATED),
        "excel" => Some(microsoft::EXCEL_CURATED),
        "todoist" => Some(productivity::TODOIST_CURATED),
        _ => None,
    }
}

/// Should this action slug appear in the agent's tool surface, given an
/// already-loaded user scope preference?
///
/// `true` when the action is in its toolkit's curated whitelist (or the toolkit
/// has no curation) **and** the preference allows its classification. Falls
/// back to [`classify_unknown`] for uncurated toolkits.
///
/// Takes a pre-loaded preference because the typical caller loops over
/// toolkits, where awaiting once per toolkit is cheaper than once per action.
#[must_use]
pub fn is_action_visible_with_pref(slug: &str, pref: &UserScopePref) -> bool {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        return true;
    };
    match catalog_for_toolkit(&toolkit) {
        Some(catalog) => match find_curated(catalog, slug) {
            Some(curated) => pref.allows(curated.scope),
            None => false,
        },
        None => pref.allows(classify_unknown(slug)),
    }
}

/// The curated scope `slug` requires, if it appears in any catalog.
///
/// `None` for a genuinely uncurated slug — a caller wanting a defensible
/// heuristic for those should reach for [`classify_unknown`] explicitly.
///
/// Sibling of [`is_action_visible_with_pref`]: that one answers "visible?",
/// this one answers "what scope is required?", so a caller can render an unlock
/// hint without redoing the catalog walk.
#[must_use]
pub fn curated_scope_for(slug: &str) -> Option<ToolScope> {
    let toolkit = toolkit_from_slug(slug)?;
    let catalog = catalog_for_toolkit(&toolkit)?;
    find_curated(catalog, slug).map(|c| c.scope)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
