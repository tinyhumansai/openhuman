//! Human-readable capability summaries for Composio toolkit slugs.

/// Human-readable capability summary for a Composio toolkit slug.
///
/// Used by the prompt renderer to tell the orchestrator what each connected
/// integration can do. Covers the most common toolkits; unknown slugs get
/// a generic fallback so newly connected services still appear.
pub fn toolkit_description(slug: &str) -> &'static str {
    match slug {
        "gmail" => {
            "Send, read, draft, reply, forward, and search emails; manage labels and threads"
        }
        "notion" => "Create, read, update, and search notion pages and notion databases",
        "github" => "Manage repositories, issues, pull requests on GitHub",
        "slack" => "Send messages, read channels, manage threads, and post updates in Slack",
        "discord" => "Send messages, manage channels, and interact with Discord servers",
        "google_calendar" => "Create, update, and query calendar events; check availability",
        "google_drive" => "Upload, download, search, and share files in Google Drive",
        "google_docs" => "Create, read, and edit Google Docs documents",
        "google_sheets" => "Read, write, and manage Google Sheets spreadsheets",
        "outlook" => "Send, read, and manage emails in Microsoft Outlook",
        "microsoft_teams" => "Send messages and manage channels in Microsoft Teams",
        "linear" => "Create and manage issues, projects, and cycles in Linear",
        "jira" => "Create and manage issues, projects, and sprints in Jira",
        "trello" => "Create and manage cards, lists, and boards in Trello",
        "asana" => "Create and manage tasks, projects, and sections in Asana",
        "clickup" => {
            "Create, read, and manage tasks, lists, and docs in ClickUp; sync \
             assigned tasks into Memory Tree"
        }
        "dropbox" => "Upload, download, and share files in Dropbox",
        "twitter" => "Post tweets, read timelines, and manage Twitter interactions",
        "spotify" => "Control playback, search music, and manage playlists on Spotify",
        "telegram" => "Send and receive messages via Telegram",
        "whatsapp" => "Send and receive messages via WhatsApp",
        "twilio" => "Send SMS, make calls, and manage communications via Twilio",
        "shopify" => "Manage products, orders, and customers in Shopify",
        "stripe" => "Manage payments, subscriptions, and customers in Stripe",
        "hubspot" => "Manage contacts, deals, and marketing in HubSpot",
        "salesforce" => "Manage contacts, leads, and opportunities in Salesforce",
        "airtable" => "Read and write records in Airtable bases",
        "figma" => "Access and manage Figma design files and components",
        "youtube" => "Search videos, manage playlists, and interact with YouTube",
        "calendar" => "Create, update, and query calendar events",
        _ => "Interact with this connected service via its available actions",
    }
}
