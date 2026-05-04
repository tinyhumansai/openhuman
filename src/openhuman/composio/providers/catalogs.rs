//! Curated catalogs for Composio toolkits that don't (yet) have a
//! native [`super::ComposioProvider`] implementation.
//!
//! These slices are consulted by [`super::catalog_for_toolkit`] alongside
//! provider-supplied catalogs (gmail, notion, github), so the meta-tool
//! layer applies the same whitelist + scope filtering.
//!
//! Slugs sourced from `https://docs.composio.dev/toolkits/<id>.md` —
//! best-effort. Slugs that don't exist on the backend simply never
//! appear in `composio_list_tools`, so extras are harmless.
//!
//! Data is split into category submodules:
//! - [`catalogs_messaging`] — Slack, Discord, Telegram, WhatsApp, MS Teams
//! - [`catalogs_google`]    — GoogleCalendar, GoogleDrive, GoogleDocs, GoogleSheets
//! - [`catalogs_productivity`] — Outlook, Linear, Jira, Trello, Asana, Dropbox
//! - [`catalogs_social_media`] — Twitter, Spotify, YouTube
//! - [`catalogs_business`]  — Shopify, Stripe, HubSpot, Salesforce, Airtable, Figma

pub use super::catalogs_business::{
    AIRTABLE_CURATED, FIGMA_CURATED, HUBSPOT_CURATED, SALESFORCE_CURATED, SHOPIFY_CURATED,
    STRIPE_CURATED,
};
pub use super::catalogs_google::{
    GOOGLECALENDAR_CURATED, GOOGLEDOCS_CURATED, GOOGLEDRIVE_CURATED, GOOGLESHEETS_CURATED,
};
pub use super::catalogs_messaging::{
    DISCORD_CURATED, MICROSOFT_TEAMS_CURATED, SLACK_CURATED, TELEGRAM_CURATED, WHATSAPP_CURATED,
};
pub use super::catalogs_productivity::{
    ASANA_CURATED, DROPBOX_CURATED, JIRA_CURATED, LINEAR_CURATED, OUTLOOK_CURATED, TRELLO_CURATED,
};
pub use super::catalogs_social_media::{SPOTIFY_CURATED, TWITTER_CURATED, YOUTUBE_CURATED};
