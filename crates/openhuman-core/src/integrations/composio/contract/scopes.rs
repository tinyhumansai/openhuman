//! How invasive an action is, and how much of that the user has agreed to.
//!
//! Composio publishes sixty-odd actions per toolkit and most of them are noise
//! for an agent's planning loop, so each provider hand-curates a slice of
//! [`CuratedTool`] entries that pares the surface down and tags every action
//! with a [`ToolScope`]. The user's [`UserScopePref`] then gates execution per
//! toolkit: reads and writes on by default, destructive and permission-changing
//! actions off until explicitly opted into.
//!
//! # Why the classification (and now the catalogs) live here
//!
//! Two different consumers ask the same question from opposite sides of the
//! module boundary. The host asks it when it renders the integrations panel and
//! when it filters the agent's visible tool list; the sync pipelines ask it
//! inside the module before firing an action. Both have to reach the same
//! verdict, which makes [`ToolScope`], [`UserScopePref::allows`] and the
//! heuristic fallback [`classify_unknown`] shared vocabulary rather than either
//! side's private policy.
//!
//! The catalogs themselves — thousands of `&'static str` action slugs across
//! thirty toolkits — live alongside this module, in [`super::catalogs`]
//! (moved here from the engine crate by OpenHuman#5560, for the same reason
//! the classification lives here: both sides of the module boundary need the
//! same answer, and a contract-crate lookup is the only way to get it without
//! either side linking the other). They are provider data, they change
//! whenever a provider does, and nothing about them has to cross a frame: what
//! crosses is the verdict.
//!
//! Reading and writing a preference is likewise the engine crate's; this module
//! defines what a preference *is*, not where it is stored.

use serde::{Deserialize, Serialize};

/// Classification of how invasive an action is.
///
/// Used both to filter the agent's visible tool list and to enforce per-user
/// scope preferences at execution time. The serde form is lowercase and is
/// persisted inside a [`UserScopePref`] key/value row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolScope {
    /// Pure reads — `GET` / `FETCH` / `LIST` / `SEARCH` / `GET_PROFILE`.
    Read,
    /// Side-effectful actions that create or mutate user data —
    /// `SEND` / `CREATE` / `UPDATE` / `REPLY` / `APPEND`.
    Write,
    /// Destructive or permission-changing actions — `DELETE` / `TRASH` /
    /// `REMOVE` / `MODIFY_LABELS` / `SHARE`.
    Admin,
}

impl ToolScope {
    /// Stable lowercase tag, matching the serde representation.
    pub fn as_str(self) -> &'static str {
        match self {
            ToolScope::Read => "read",
            ToolScope::Write => "write",
            ToolScope::Admin => "admin",
        }
    }
}

/// One curated entry in a provider's tool catalog.
///
/// `slug` is the Composio action slug as the toolkit listing returns it, e.g.
/// `"GMAIL_SEND_EMAIL"`. `scope` controls whether the action is gated by the
/// user's read / write / admin preference.
///
/// Deliberately `&'static str` and `Copy`: catalogs are `const` slices built at
/// compile time, and giving this owned `String` fields would turn thirty static
/// tables into thirty heap allocations at startup for no gain.
#[derive(Debug, Clone, Copy)]
pub struct CuratedTool {
    /// The Composio action slug, e.g. `"GMAIL_SEND_EMAIL"`.
    pub slug: &'static str,
    /// How invasive the action is, for preference gating.
    pub scope: ToolScope,
}

/// Per-toolkit scope preference.
///
/// Defaults are `read = true`, `write = true`, `admin = false` — the agent can
/// use a connected integration productively out of the box, but destructive and
/// permission-changing actions require an explicit opt-in.
///
/// The two `default_true` helpers matter for decoding: a row written before a
/// field existed must not read back as "denied", which is what a bare
/// `#[serde(default)]` would give a `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserScopePref {
    /// Whether the agent may call [`ToolScope::Read`] actions.
    #[serde(default = "default_true")]
    pub read: bool,
    /// Whether the agent may call [`ToolScope::Write`] actions.
    #[serde(default = "default_true")]
    pub write: bool,
    /// Whether the agent may call [`ToolScope::Admin`] actions.
    #[serde(default)]
    pub admin: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UserScopePref {
    fn default() -> Self {
        Self {
            read: true,
            write: true,
            admin: false,
        }
    }
}

impl UserScopePref {
    /// Whether the given scope is enabled in this preference.
    pub fn allows(&self, scope: ToolScope) -> bool {
        match scope {
            ToolScope::Read => self.read,
            ToolScope::Write => self.write,
            ToolScope::Admin => self.admin,
        }
    }
}

/// Heuristic fallback for gating a tool that is not in any provider's curated
/// list.
///
/// Prefer the curated classification when one exists; only reach for this when
/// a toolkit has no catalog or the catalog does not mention the slug. Admin
/// verbs are checked first so `MODIFY_LABELS` does not slip into the write
/// bucket on the `UPDATE` substring rule — the ordering is the whole point of
/// the function and not an implementation detail.
pub fn classify_unknown(slug: &str) -> ToolScope {
    let upper = slug.to_ascii_uppercase();
    const ADMIN: &[&str] = &[
        "DELETE",
        "TRASH",
        "REMOVE",
        "MODIFY_LABELS",
        "SHARE",
        "REVOKE",
        "DESTROY",
    ];
    const WRITE: &[&str] = &[
        "SEND", "CREATE", "UPDATE", "REPLY", "APPEND", "INSERT", "ADD", "POST", "PATCH", "WRITE",
        "DRAFT",
    ];
    if ADMIN.iter().any(|kw| upper.contains(kw)) {
        return ToolScope::Admin;
    }
    if WRITE.iter().any(|kw| upper.contains(kw)) {
        return ToolScope::Write;
    }
    ToolScope::Read
}

/// Look up a slug inside a curated catalog, case-insensitively.
pub fn find_curated<'a>(catalog: &'a [CuratedTool], slug: &str) -> Option<&'a CuratedTool> {
    catalog.iter().find(|t| t.slug.eq_ignore_ascii_case(slug))
}

/// Extract the toolkit slug from a Composio action slug.
///
/// Most action slugs follow `<TOOLKIT>_<VERB>_…` — `GMAIL_SEND_EMAIL` yields
/// `gmail`. A few toolkit identifiers contain underscores themselves, so those
/// need known-prefix handling or a connected-toolkit check silently drops every
/// action for them (`ZOHO_MAIL_*` would resolve to the non-existent `zoho`).
///
/// Returns `None` only for an empty or whitespace-only slug.
pub fn toolkit_from_slug(slug: &str) -> Option<String> {
    let trimmed = slug.trim();
    if trimmed.is_empty() {
        return None;
    }
    const MULTI_SEGMENT_TOOLKIT_PREFIXES: &[(&str, &str)] = &[
        ("MICROSOFT_TEAMS_", "microsoft_teams"),
        ("ONE_DRIVE_", "one_drive"),
        ("ZOHO_MAIL_", "zoho_mail"),
    ];
    let upper = trimmed.to_ascii_uppercase();
    for (prefix, toolkit) in MULTI_SEGMENT_TOOLKIT_PREFIXES {
        if upper.starts_with(prefix) {
            return Some((*toolkit).to_string());
        }
    }
    let prefix = trimmed.split('_').next()?;
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_ascii_lowercase())
    }
}

/// Every toolkit slug that has a curated, agent-ready catalog.
///
/// This is the source of truth behind the "preview / agent integration coming
/// soon" badge: a connected toolkit whose slug is *not* in this list can be
/// authorized but has no curated tool surface, so the agent cannot use it
/// productively and the UI should say so rather than offering it.
///
/// Returned sorted, so the RPC response is stable across builds.
///
/// The list is here rather than with the catalogs it names because the *host*
/// renders the badge. Keeping it beside the catalogs would mean the host asking
/// the module a question — "is this toolkit worth showing?" — that has no
/// user-visible state behind it and would answer identically forever.
pub fn agent_ready_toolkits() -> Vec<&'static str> {
    let mut slugs: Vec<&'static str> = vec![
        // Native providers.
        "gmail",
        "notion",
        "github",
        // Catalog-only toolkits.
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

#[cfg(test)]
#[path = "scopes_tests.rs"]
mod tests;
