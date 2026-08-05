//! Validation, transport resolution, and env-scoping helpers for custom MCP
//! servers. Pure functions consumed by `super::ops`; no persistence or IO.

use std::collections::HashMap;

use crate::openhuman::mcp::registry::types::{CommandKind, Transport};

use super::CustomServerInput;
use super::RESERVED_ENV_PREFIX;

// ── validation ───────────────────────────────────────────────────────────────

/// Resolve the form payload into the persisted transport fields.
///
/// Mirrors `setup_ops::build_install_transport` so both install paths produce
/// identically-shaped rows; the difference is only that the values come from
/// user input rather than a catalog connection spec.
pub(super) fn build_custom_transport(
    input: &CustomServerInput,
) -> Result<(Transport, CommandKind, String, Vec<String>), String> {
    match input.transport.trim() {
        "http_remote" => {
            let url = input.url.as_deref().unwrap_or("").trim().to_string();
            if url.is_empty() {
                return Err("url must not be empty for an http_remote server".to_string());
            }
            // Reject a non-http(s) URL here rather than letting it through to
            // become an opaque dial failure much later. `file://` and friends
            // have no business reaching the HTTP client at all.
            let parsed = url::Url::parse(&url)
                .map_err(|e| format!("url is not a valid absolute URL: {e}"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(format!(
                    "url scheme must be http or https, got `{}`",
                    parsed.scheme()
                ));
            }
            // Reject `https://user:pass@host/…`. Unlike env values (write-only),
            // the URL is stored verbatim in `Transport::HttpRemote { url }` and
            // echoed back in `InstalledServer`, so a credential in the userinfo
            // would be persisted in cleartext and returned to the client. Auth
            // belongs in write-only headers or OAuth.
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(
                    "url must not embed credentials (user:password@) — add auth as a header or use OAuth".to_string(),
                );
            }
            Ok((
                Transport::HttpRemote { url },
                // Unused for HTTP — matches what build_install_transport stores.
                CommandKind::Node,
                String::new(),
                Vec::new(),
            ))
        }
        "stdio" => {
            let command = input.command.as_deref().unwrap_or("").trim().to_string();
            if command.is_empty() {
                return Err("command must not be empty for a stdio server".to_string());
            }
            let args: Vec<String> = input
                .args
                .iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect();
            Ok((
                Transport::Stdio,
                infer_command_kind(&command),
                command,
                args,
            ))
        }
        other => Err(format!(
            "transport must be `stdio` or `http_remote`, got `{other}`"
        )),
    }
}

/// Classify a hand-entered launcher for the persisted `command_kind`.
///
/// Metadata only — `connections::connect` spawns `command` + `args` verbatim
/// and never reads this. It is populated anyway because every registry install
/// carries it, and a column that is accurate for some rows and a blind `node`
/// default for others is worse than one that is right everywhere.
fn infer_command_kind(command: &str) -> CommandKind {
    let file_name = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    let base = file_name.strip_suffix(".exe").unwrap_or(file_name.as_str());
    match base {
        "npx" | "npm" | "node" | "bun" | "bunx" | "pnpm" | "pnpx" | "yarn" => CommandKind::Node,
        "uvx" | "uv" | "pipx" | "python" | "python3" => CommandKind::Python,
        _ => CommandKind::Binary,
    }
}

/// Resolve the submitted env map against what is already stored.
///
/// Two rules, both forced by the fact that env *values* are write-only (the
/// core never returns them, so the edit form cannot round-trip a secret):
///
/// - The submitted **key set is authoritative** — removing a row deletes that
///   key. This is why `update_custom` cannot just merge the way
///   `mcp_clients_update_env` does; merging has no way to express a removal.
/// - A submitted key with a **blank value keeps the stored value**. An
///   untouched row necessarily arrives blank, and a blank is not meaningful on
///   either transport anyway (`build_http_auth` skips empty headers; an empty
///   env var is noise), so nothing is lost by giving it that meaning.
///
/// Reserved `__`-prefixed entries are carried over unconditionally: the form
/// never submits them, so without this an unrelated rename would drop
/// `__oauth__` and silently sign the user out of an OAuth'd server.
///
/// Both rules are scoped to *one* transport — see [`resolve_env_for_transport`],
/// which is what callers should use.
pub(super) fn resolve_env(
    submitted: &HashMap<String, String>,
    stored: &HashMap<String, String>,
    is_http_remote: bool,
) -> HashMap<String, String> {
    let mut resolved: HashMap<String, String> = HashMap::new();
    for (key, value) in submitted {
        let key = key.trim().to_string();
        if !value.trim().is_empty() {
            resolved.insert(key, value.clone());
        } else if let Some(existing) = lookup_stored(stored, &key, is_http_remote) {
            resolved.insert(key, existing.clone());
        }
        // Blank value with nothing stored: there is no secret to keep, so the
        // key is simply not set.
    }
    for (key, value) in stored {
        if key.starts_with(RESERVED_ENV_PREFIX) {
            resolved.insert(key.clone(), value.clone());
        }
    }
    resolved
}

/// Look up a submitted key against the stored map for the blank-means-keep rule.
///
/// On `http_remote` the keys are HTTP header names (case-insensitive, RFC 9110),
/// so a user who only re-cased a header (`Authorization` → `authorization`) and
/// left the value blank still means "keep" — a case-sensitive `HashMap::get`
/// would miss the stored value and silently drop the credential. stdio keys are
/// subprocess env vars, case-sensitive on Unix, so they match exactly.
fn lookup_stored<'a>(
    stored: &'a HashMap<String, String>,
    key: &str,
    is_http_remote: bool,
) -> Option<&'a String> {
    if let Some(v) = stored.get(key) {
        return Some(v);
    }
    if is_http_remote {
        let lower = key.to_ascii_lowercase();
        return stored
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == lower)
            .map(|(_, v)| v);
    }
    None
}

/// Where a stored credential is authorised to go. Two edits share a scope only
/// if the credential still means the same thing afterwards.
///
/// - stdio: the credentials are subprocess env vars; the process is spawned from
///   the launch command, but env is not command-specific, so all stdio shares
///   one scope. (A command change is not a credential re-scope.)
/// - http_remote: the credentials are request headers and, for OAuth, a refresh
///   bundle minted against a *specific* endpoint. The scope is the endpoint's
///   **origin** (scheme + host + port). A different origin is a different service.
pub(super) enum CredentialScope {
    Stdio,
    HttpOrigin(String),
    /// The URL didn't parse. `build_custom_transport` rejects such URLs before a
    /// `Transport` is built, so this is unreachable via the real callers — but it
    /// must still *never* compare equal (see the manual `PartialEq`), so a
    /// corrupt stored URL can only force re-entry, never keep credentials by
    /// accident.
    Unparseable,
}

// Hand-written rather than derived so `Unparseable` is never equal to anything,
// including another `Unparseable` — the derived `Eq` would make two corrupt URLs
// compare equal and *keep* the stored credentials, the opposite of the intent.
impl PartialEq for CredentialScope {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Stdio, Self::Stdio) => true,
            (Self::HttpOrigin(a), Self::HttpOrigin(b)) => a == b,
            _ => false,
        }
    }
}

pub(super) fn credential_scope(transport: &Transport) -> CredentialScope {
    match transport {
        Transport::Stdio => CredentialScope::Stdio,
        Transport::HttpRemote { url } => match url::Url::parse(url) {
            Ok(u) => CredentialScope::HttpOrigin(u.origin().ascii_serialization()),
            Err(_) => CredentialScope::Unparseable,
        },
    }
}

/// Resolve the env for an edit that may also re-scope the credentials.
///
/// Nothing stored carries across a scope change, reserved keys included. Both of
/// [`resolve_env`]'s rules assume the map keeps its meaning, and a scope change
/// is exactly where it doesn't:
///
/// - Ordinary keys are the subprocess environment on stdio and request headers
///   on `http_remote` ([`connections::build_http_auth`]). Carrying them across a
///   transport switch ships a token that only ever reached a local process to a
///   remote host; carrying them to a different **origin** ships one service's
///   bearer token to another. Values are write-only, so blank-means-keep makes
///   either invisible.
/// - `__oauth__` is a refresh bundle (refresh token + client secret + token
///   endpoint) minted for one origin. On a subprocess `McpStdioClient` hands the
///   whole env to the child with no `__` filter; on a new origin the next
///   `refresh_if_expired` mints a token against the *old* endpoint and sends it
///   to the *new* host. Either way the secret crosses the scope it was issued
///   for.
///
/// A re-scoped credential must be re-entered. This lives here, not in the form:
/// `/rpc`, the CLI and the iOS client all reach this function without touching
/// the React layer.
pub(super) fn resolve_env_for_transport(
    submitted: &HashMap<String, String>,
    stored: &HashMap<String, String>,
    previous: &Transport,
    next: &Transport,
) -> HashMap<String, String> {
    // The submitted keys are interpreted under the *new* transport.
    let is_http_remote = next.is_http_remote();
    if credential_scope(previous) != credential_scope(next) {
        return resolve_env(submitted, &HashMap::new(), is_http_remote);
    }
    resolve_env(submitted, stored, is_http_remote)
}

/// Sorted key list for the install record, matching what `update_env` persists.
pub(super) fn env_key_list(env: &HashMap<String, String>) -> Vec<String> {
    let mut keys: Vec<String> = env.keys().cloned().collect();
    keys.sort();
    keys
}

/// Reject env keys that are empty, claim the reserved `__` namespace, or collide
/// case-insensitively where the target treats names case-insensitively.
///
/// Validates the key **as `resolve_env` will store it** — i.e. trimmed. Checking
/// the raw key would let `"  __oauth__"` through here and then land as
/// `__oauth__` after the trim, defeating the guard by the normalisation applied
/// downstream.
///
/// The case-duplicate check fires when a case-only duplicate would collapse to
/// one value in the target:
/// - **http_remote**: the keys are HTTP header names, case-insensitive by
///   RFC 9110, so `Authorization` and `authorization` are the same header.
/// - **stdio on Windows**: subprocess env var names are case-insensitive there
///   (`Path` == `PATH`); Rust's `Command` dedups them case-insensitively when
///   spawning, so two rows would silently collapse. On Unix env var names are
///   case-sensitive, so stdio keys are compared exactly.
///
/// `cfg!(windows)` reflects the OS this core (and the subprocess it spawns) runs
/// on. The rule has to live here, not just the UI, for `/rpc` / CLI / iOS.
pub(super) fn validate_env(
    env: &HashMap<String, String>,
    is_http_remote: bool,
) -> Result<(), String> {
    let case_insensitive = is_http_remote || cfg!(windows);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw_key in env.keys() {
        let key = raw_key.trim();
        if key.is_empty() {
            return Err("env keys must not be empty".to_string());
        }
        if key.starts_with(RESERVED_ENV_PREFIX) {
            return Err(format!(
                "env key `{key}` is reserved — keys starting with `{RESERVED_ENV_PREFIX}` hold internal connection state"
            ));
        }
        if case_insensitive && !seen.insert(key.to_ascii_lowercase()) {
            let kind = if is_http_remote {
                "header"
            } else {
                "environment variable"
            };
            return Err(format!(
                "{kind} `{key}` is listed more than once (names are case-insensitive here)"
            ));
        }
    }
    Ok(())
}

/// ASCII slug for the `custom/<slug>` identity, or `None` when the name has no
/// ASCII alphanumerics to build one from.
///
/// Deliberately ASCII-only: `qualified_name` is an internal identifier that
/// flows into logs and dedupe lookups, while the UI always renders
/// `display_name`. Returning `None` rather than a constant `"server"` lets the
/// caller fall back to a per-server unique fragment — otherwise every server
/// named purely in a non-Latin script (CJK, Cyrillic, Arabic, …) would collapse
/// to the *same* slug, so a user with such names would exhaust the collision
/// suffix at 100 *distinct* servers, and every log line would read
/// `custom/server-N` with no diagnostic value.
fn slugify(raw: &str) -> Option<String> {
    let mut slug = String::new();
    let mut pending_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

/// Allocate the stable catalog-style identity for a new custom server.
///
/// `qualified_name` is what `find_server_by_qualified_name` dedupes on, so it
/// must be unique. It is derived once here and never re-derived on rename —
/// re-slugging an edited display name would change the row's identity and
/// orphan its stored env values.
///
/// A collision gets a numeric suffix rather than an error: two servers can
/// legitimately share a display name (same tool, different args or
/// credentials), and the user should not have to invent a unique label.
/// The slug for a server's base `qualified_name`: the display name's slug, or —
/// when the name has no ASCII to build one from — a fragment of the (unique)
/// server_id. The fallback is per-server so two differently-named non-Latin
/// servers get different slugs instead of both collapsing to one constant.
pub(super) fn base_slug(display_name: &str, server_id: &str) -> String {
    // `char`-based, not a byte slice: the production server_id is an ASCII uuid,
    // but a byte slice would panic on a char boundary if that ever changes. This
    // is only a label — uniqueness comes from the DB-checked suffix loop.
    slugify(display_name)
        .unwrap_or_else(|| format!("server-{}", server_id.chars().take(8).collect::<String>()))
}

pub(super) fn clean_description(raw: Option<String>) -> Option<String> {
    raw.map(|d| d.trim().to_string()).filter(|d| !d.is_empty())
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
