//! Who the user is on a connected account: the identifier kinds, how each one
//! is canonicalised, and what a loaded set of identities looks like.
//!
//! A provider hands back one [`ProviderUserProfile`] per connection. The engine
//! crate expands that into one facet row per identifier, so the self-identity
//! matcher can answer "is this message *from the user*?" with a
//! `(toolkit, kind, canonical value)` lookup rather than a fuzzy comparison.
//! [`IdentityKind`] is that matching axis and [`canonicalize`] is the routine
//! both sides of the comparison run.
//!
//! # Why canonicalisation is contract vocabulary
//!
//! Because equality of canonical forms is the matcher's *only* test. The value
//! is canonicalised once when a profile is persisted and again when a candidate
//! identifier is checked against it — no `COLLATE NOCASE`, no per-call
//! lowercasing. If the writer and the reader ran two different implementations
//! of that routine, the matcher would fail open: a user's own Slack messages
//! would stop being recognised as theirs, silently, with nothing to catch it.
//! Those two calls are on opposite sides of the module boundary, which is what
//! puts [`canonicalize`] here and not in the engine.
//!
//! The same argument covers [`normalize_connection_identifier`]: it produces
//! the key segment a facet row is *stored under*, so writer and reader must
//! spell it identically or a disconnect leaves rows behind and the removed
//! account keeps being treated as the user.
//!
//! # What is not here
//!
//! Persisting a profile, loading the identities back and deleting a
//! connection's rows: those are `identity_store`'s, over a JSON file in the
//! workspace.

use serde::{Deserialize, Serialize};

/// Normalized user profile shape returned by every provider.
///
/// The shared fields (`display_name`, `email`, `username`, `avatar_url`,
/// `profile_url`) cover what a desktop UI needs to render a connected-account
/// card. Anything provider-specific — Gmail's `messagesTotal`, Notion's
/// workspace ids — goes into [`extras`](Self::extras), so callers do not widen
/// the shape every time a new toolkit lands, and so an identifier a provider
/// only exposes there (a Slack screen name, for instance) is still available to
/// the row expansion.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderUserProfile {
    /// Composio toolkit slug the profile was fetched from, e.g. `"gmail"`.
    pub toolkit: String,
    /// The connection the profile belongs to; `None` on toolkit-wide fetches.
    pub connection_id: Option<String>,
    /// Human display label, when the provider exposes one.
    pub display_name: Option<String>,
    /// Primary email address on the connected account.
    pub email: Option<String>,
    /// Platform username or screen name, without any leading `@`.
    pub username: Option<String>,
    /// URL of the account's avatar image.
    pub avatar_url: Option<String>,
    /// URL of the account's public profile page.
    pub profile_url: Option<String>,
    /// Provider-specific extras (raw JSON object).
    #[serde(default)]
    pub extras: serde_json::Value,
}

/// Shape of an identifier persisted against a connection.
///
/// A closed set of identifier dimensions, so the
/// self-check is a direct `(toolkit, kind, value)` lookup. The string form is
/// the last segment of the stored facet key, which makes every variant name a
/// durable value rather than a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKind {
    /// Platform-canonical immutable id — a Slack `U123ABC`, a Notion UUID.
    UserId,
    /// Email address.
    Email,
    /// An `@`-style screen name, canonicalised without the leading `@`.
    Handle,
    /// E.164 phone number.
    Phone,
    /// Human display label. A weak signal — never auto-promotes to "is self".
    DisplayName,
    /// Not for matching; kept for UI and prompt rendering.
    AvatarUrl,
    /// Not for matching; kept for UI and prompt rendering.
    ProfileUrl,
}

impl IdentityKind {
    /// The stored key segment for this kind.
    ///
    /// Durable: it is the last segment of a persisted facet key, so renaming a
    /// variant's string orphans every row filed under the old one.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserId => "user_id",
            Self::Email => "email",
            Self::Handle => "handle",
            Self::Phone => "phone",
            Self::DisplayName => "display_name",
            Self::AvatarUrl => "avatar_url",
            Self::ProfileUrl => "profile_url",
        }
    }

    /// Parse a stored key segment back into a kind.
    ///
    /// Returns `None` for anything unrecognised — including the legacy
    /// `username` segment written before the identifier rewrite. Callers skip
    /// those rows rather than failing the load, so one stale row cannot make a
    /// user's whole identity set unreadable.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user_id" => Self::UserId,
            "email" => Self::Email,
            "handle" => Self::Handle,
            "phone" => Self::Phone,
            "display_name" => Self::DisplayName,
            "avatar_url" => Self::AvatarUrl,
            "profile_url" => Self::ProfileUrl,
            _ => return None,
        })
    }

    /// Confidence the matcher records on a row of this kind.
    ///
    /// Hard kinds auto-promote a chunk to "is self"; weak kinds require
    /// corroboration. A display name is deliberately low — two people share a
    /// name far more often than they share a user id.
    pub fn confidence(self) -> f64 {
        match self {
            Self::UserId | Self::Phone => 1.00,
            Self::Email => 0.95,
            Self::Handle => 0.70,
            Self::DisplayName => 0.40,
            Self::AvatarUrl | Self::ProfileUrl => 0.50,
        }
    }

    /// Whether this kind is a real identity signal worth running through the
    /// matcher, as opposed to a UI-only field.
    pub fn is_matchable(self) -> bool {
        matches!(
            self,
            Self::UserId | Self::Email | Self::Handle | Self::Phone | Self::DisplayName
        )
    }
}

/// Canonicalize a raw identifier for storage and lookup.
///
/// The same routine runs on the entity side at match time, so equality of
/// canonical forms is the matcher's only test. Returns `None` for an empty or
/// whitespace-only value — storing one would match every chunk that happened to
/// carry a blank sender.
pub fn canonicalize(kind: IdentityKind, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(match kind {
        IdentityKind::Email => trimmed.to_lowercase(),
        IdentityKind::Handle => trimmed.trim_start_matches('@').to_lowercase(),
        IdentityKind::Phone => trimmed
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect(),
        IdentityKind::DisplayName => trimmed.split_whitespace().collect::<Vec<_>>().join(" "),
        IdentityKind::UserId | IdentityKind::AvatarUrl | IdentityKind::ProfileUrl => {
            trimmed.to_string()
        }
    })
}

/// Every identifier known for one `(source, connection)` pair, collapsed into
/// one row.
///
/// This is the read shape: the store holds one facet per identifier, and a
/// loader groups them back into this so a caller does not have to reassemble an
/// account from seven rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectedIdentity {
    /// Toolkit slug the identity came from, normalised.
    pub source: String,
    /// Connection identifier, normalised — see
    /// [`normalize_connection_identifier`].
    pub identifier: String,
    /// Human display label, when one was stored.
    pub display_name: Option<String>,
    /// Canonicalised email address.
    pub email: Option<String>,
    /// Canonicalised screen name, without the leading `@`.
    pub handle: Option<String>,
    /// Canonicalised phone number.
    pub phone: Option<String>,
    /// Platform-canonical immutable id.
    pub user_id: Option<String>,
    /// Avatar image URL.
    pub avatar_url: Option<String>,
    /// Public profile URL.
    pub profile_url: Option<String>,
}

/// Render a compact prompt section for a set of identities.
///
/// Skips `user_id` (not human-readable) and prefixes a handle with `@`. Returns
/// an empty string — rather than a bare heading — when there is nothing worth
/// showing, so a caller can concatenate the result unconditionally.
///
/// Every value is flattened onto one line and has `|` replaced before it is
/// joined with `|` separators: an identifier is user-controlled text arriving
/// from a third-party provider, and a display name containing a newline would
/// otherwise let it forge additional prompt lines.
pub fn render_connected_identities_section(identities: &[ConnectedIdentity]) -> String {
    if identities.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Connected Identities\n\n");
    for id in identities {
        let mut fields = Vec::<String>::new();
        if let Some(v) = id.display_name.as_deref() {
            let v = sanitize_prompt_value(v);
            if !v.is_empty() {
                fields.push(v);
            }
        }
        if let Some(v) = id.email.as_deref() {
            let v = sanitize_prompt_value(v);
            if !v.is_empty() {
                fields.push(v);
            }
        }
        if let Some(v) = id.handle.as_deref() {
            let v = sanitize_prompt_value(v);
            if !v.is_empty() {
                fields.push(format!("@{v}"));
            }
        }
        if let Some(v) = id.profile_url.as_deref() {
            let v = sanitize_prompt_value(v);
            if !v.is_empty() {
                fields.push(v);
            }
        }
        if fields.is_empty() {
            continue;
        }
        let identifier = sanitize_prompt_value(&id.identifier);
        out.push_str(&format!(
            "- {} ({}): {}\n",
            title_case(&id.source),
            identifier,
            fields.join(" | ")
        ));
    }
    if out.trim() == "## Connected Identities" {
        return String::new();
    }
    out
}

/// Normalize a raw toolkit slug or connection id into the form facet keys are
/// stored under.
///
/// Lowercases, replaces every character outside `[a-z0-9_-]` with `_`, and
/// trims leading and trailing underscores. Writer and reader must both call
/// this: a caller passing a raw connection id to a delete would otherwise match
/// no rows, and the disconnected account would keep being treated as the user.
pub fn normalize_connection_identifier(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '-' || lower == '_' {
            out.push(lower);
        } else {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn title_case(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn sanitize_prompt_value(raw: &str) -> String {
    let replaced = raw.replace(['\n', '\r', '\t'], " ").replace('|', "/");
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
