//! Custom (user-entered) MCP server installs.
//!
//! The paths in `ops.rs` resolve a server's command/args/url by fetching an
//! upstream catalog listing keyed by `qualified_name`. This module covers the
//! servers that have no listing: the user types the launch command (stdio) or
//! the endpoint URL (http_remote) in directly.
//!
//! Only the *provenance* differs. The record written here is an ordinary
//! [`InstalledServer`] carrying [`ServerProvenance::Custom`], so
//! [`super::connections`], the supervisor, boot spawn, and the agent tool
//! surface treat it exactly like a catalog install — which is what keeps
//! OAuth, redirect resolution, and the tool safety filter working here without
//! a second transport implementation.

use std::collections::HashMap;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::connections;
use super::store;
use super::types::{CommandKind, InstalledServer, ServerProvenance, Transport};

/// Namespace for generated `qualified_name`s. Keeps hand-entered servers from
/// ever colliding with a catalog name (no registry publishes under `custom/`).
const CUSTOM_QUALIFIED_PREFIX: &str = "custom/";

/// Env keys starting with this are reserved for internal connection state —
/// `__oauth__` holds the OAuth refresh bundle. `connections::build_http_auth`
/// filters them out of outgoing headers, so a user-created one would silently
/// do nothing on http_remote while risking a collision with OAuth storage.
const RESERVED_ENV_PREFIX: &str = "__";

/// Upper bound on slug de-duplication attempts before giving up. Only reached
/// if a user really has this many servers sharing one display name.
const MAX_SLUG_ATTEMPTS: usize = 100;

/// The user-editable half of a custom server record, as submitted by the add /
/// edit form. Identity (`server_id`, `qualified_name`, `installed_at`) and
/// provenance are owned by this module, never by the caller.
#[derive(Debug, Clone, Default)]
pub struct CustomServerInput {
    pub display_name: String,
    /// `"stdio"` or `"http_remote"` — matches [`Transport::dispatch_kind`].
    pub transport: String,
    /// Launch binary for stdio servers (`npx`, `uvx`, an absolute path, …).
    pub command: Option<String>,
    /// Arguments passed to `command`. Ignored for http_remote.
    pub args: Vec<String>,
    /// Endpoint for http_remote servers. Ignored for stdio.
    pub url: Option<String>,
    /// stdio: environment variables for the subprocess.
    /// http_remote: request headers (key = header name) — the convention
    /// `connections::build_http_auth` already reads for catalog installs.
    pub env: HashMap<String, String>,
    pub description: Option<String>,
}

// ── validation ───────────────────────────────────────────────────────────────

/// Resolve the form payload into the persisted transport fields.
///
/// Mirrors `setup_ops::build_install_transport` so both install paths produce
/// identically-shaped rows; the difference is only that the values come from
/// user input rather than a catalog connection spec.
fn build_custom_transport(
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
fn resolve_env(
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
enum CredentialScope {
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

fn credential_scope(transport: &Transport) -> CredentialScope {
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
fn resolve_env_for_transport(
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
fn env_key_list(env: &HashMap<String, String>) -> Vec<String> {
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
fn validate_env(env: &HashMap<String, String>, is_http_remote: bool) -> Result<(), String> {
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
fn base_slug(display_name: &str, server_id: &str) -> String {
    // `char`-based, not a byte slice: the production server_id is an ASCII uuid,
    // but a byte slice would panic on a char boundary if that ever changes. This
    // is only a label — uniqueness comes from the DB-checked suffix loop.
    slugify(display_name)
        .unwrap_or_else(|| format!("server-{}", server_id.chars().take(8).collect::<String>()))
}

fn allocate_qualified_name(
    config: &Config,
    display_name: &str,
    server_id: &str,
) -> Result<String, String> {
    let base = format!(
        "{CUSTOM_QUALIFIED_PREFIX}{}",
        base_slug(display_name, server_id)
    );
    for attempt in 0..MAX_SLUG_ATTEMPTS {
        let candidate = if attempt == 0 {
            base.clone()
        } else {
            format!("{base}-{}", attempt + 1)
        };
        let taken = store::find_server_by_qualified_name(config, &candidate)
            .map_err(|e| e.to_string())?
            .is_some();
        if !taken {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not allocate a unique name for `{display_name}` after {MAX_SLUG_ATTEMPTS} attempts"
    ))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn clean_description(raw: Option<String>) -> Option<String> {
    raw.map(|d| d.trim().to_string()).filter(|d| !d.is_empty())
}

// ── add_custom ───────────────────────────────────────────────────────────────

/// Create an install record from hand-entered connection details.
///
/// The row is written disconnected (`last_connected_at: None`); the caller
/// dials it with the existing `mcp_clients_connect` so that adding a server and
/// connecting an existing one share one code path and one set of failure
/// messages.
pub async fn mcp_clients_add_custom(
    config: &Config,
    input: CustomServerInput,
) -> Result<RpcOutcome<Value>, String> {
    let display_name = input.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err("display_name must not be empty".to_string());
    }
    let (transport, command_kind, command, args) = build_custom_transport(&input)?;
    validate_env(&input.env, transport.is_http_remote())?;

    // Nothing is stored yet, so this only drops blank-valued keys.
    let env = resolve_env(&input.env, &HashMap::new(), transport.is_http_remote());

    let server_id = Uuid::new_v4().to_string();
    let qualified_name = allocate_qualified_name(config, &display_name, &server_id)?;

    tracing::debug!(
        "[mcp-custom] add display_name={} qualified_name={} transport={} env_keys={:?}",
        display_name,
        qualified_name,
        transport.dispatch_kind(),
        env.keys().collect::<Vec<_>>()
    );

    let server = InstalledServer {
        server_id: server_id.clone(),
        qualified_name: qualified_name.clone(),
        display_name,
        description: clean_description(input.description.clone()),
        icon_url: None,
        command_kind,
        command,
        args,
        env_keys: env_key_list(&env),
        config: None,
        installed_at: now_ms(),
        last_connected_at: None,
        transport,
        enabled: true,
        provenance: ServerProvenance::Custom,
    };

    // `allocate_qualified_name` and this insert are separate statements, so a
    // concurrent add can take the slug in between. Surface that instead of
    // refreshing onto the winner the way `mcp_clients_install` does: two custom
    // rows sharing a name are *different servers*, so silently attaching this
    // request's command and credentials to someone else's row would be wrong.
    // Row and env commit together: a row that lands without its env is a server
    // the caller was told did not save, holding the name and relaunched by the
    // supervisor every tick with no credentials.
    if !store::insert_custom_server_with_env(config, &server, &env).map_err(|e| e.to_string())? {
        return Err(format!(
            "the name `{qualified_name}` was taken by a concurrent add; please retry"
        ));
    }

    tracing::debug!(
        "[mcp-custom] add ok server_id={} qualified_name={}",
        server_id,
        server.qualified_name
    );

    publish_global(DomainEvent::McpServerInstalled {
        server_id: server_id.clone(),
        qualified_name: server.qualified_name.clone(),
    });

    Ok(RpcOutcome::new(
        json!({ "server": server }),
        vec![format!("added custom server_id={server_id}")],
    ))
}

// ── update_custom ────────────────────────────────────────────────────────────

/// Replace the connection details of an existing custom server.
///
/// Refuses registry installs: their command/url are re-derived from the
/// catalog listing, so an edit here would be silently reverted by the next
/// re-resolve.
pub async fn mcp_clients_update_custom(
    config: &Config,
    server_id: String,
    input: CustomServerInput,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = server_id.trim().to_string();
    if server_id.is_empty() {
        return Err("server_id must not be empty".to_string());
    }
    let display_name = input.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err("display_name must not be empty".to_string());
    }

    let (transport, command_kind, command, args) = build_custom_transport(&input)?;
    validate_env(&input.env, transport.is_http_remote())?;

    // Read the current record, resolve env, and write both tables as one
    // serializable transaction (see `store::update_custom_server_rmw`).
    // Provenance, the previous transport (for credential-scope), and the stored
    // env are ALL read inside the lock: reading any of them outside races a
    // concurrent edit — an OAuth refresh could rotate a token, or another
    // `update_custom` could switch transport and store new-scope credentials —
    // and a stale snapshot could revert the token or mis-scope and carry the
    // new credentials across origins.
    let updated = store::update_custom_server_rmw(config, &server_id, |current, stored_env| {
        if current.provenance != ServerProvenance::Custom {
            anyhow::bail!(
                "server `{server_id}` was installed from a registry; its command and endpoint come from the catalog listing and cannot be edited here"
            );
        }
        let scope_changed = credential_scope(&current.transport) != credential_scope(&transport);
        let env =
            resolve_env_for_transport(&input.env, &stored_env, &current.transport, &transport);
        tracing::debug!(
            "[mcp-custom] update server_id={} transport={}{} env_keys={:?}",
            server_id,
            transport.dispatch_kind(),
            if scope_changed {
                " (re-scoped — stored env dropped)"
            } else {
                ""
            },
            env.keys().collect::<Vec<_>>()
        );
        let record = InstalledServer {
            // Identity and provenance survive an edit untouched — re-deriving
            // `qualified_name` from the new display name would orphan this row's
            // env values.
            server_id: current.server_id.clone(),
            qualified_name: current.qualified_name.clone(),
            installed_at: current.installed_at,
            provenance: current.provenance,
            icon_url: current.icon_url.clone(),
            config: current.config.clone(),
            enabled: current.enabled,
            last_connected_at: current.last_connected_at,

            display_name: display_name.clone(),
            description: clean_description(input.description.clone()),
            command_kind,
            command: command.clone(),
            args: args.clone(),
            env_keys: env_key_list(&env),
            transport: transport.clone(),
        };
        Ok((record, env))
    })
    .map_err(|e| e.to_string())?;

    // Only now drop the live connection: it was dialed with the previous command
    // or URL, so leaving it up would keep serving tools from the old
    // configuration. Disconnecting *before* the write (as this used to) races the
    // supervisor, which redials from a snapshot taken before the write landed and
    // pins the server to the pre-edit command indefinitely — it stays healthy, so
    // no later tick reconnects it. `update_env` persists first for the same
    // reason.
    connections::disconnect(&server_id).await;

    tracing::debug!("[mcp-custom] update ok server_id={}", server_id);

    Ok(RpcOutcome::new(
        json!({ "server": updated }),
        vec![format!("updated custom server_id={server_id}")],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_input(command: &str) -> CustomServerInput {
        CustomServerInput {
            display_name: "Local Server".to_string(),
            transport: "stdio".to_string(),
            command: Some(command.to_string()),
            args: vec!["-y".to_string(), "pkg".to_string()],
            ..Default::default()
        }
    }

    fn http_input(url: &str) -> CustomServerInput {
        CustomServerInput {
            display_name: "Remote Server".to_string(),
            transport: "http_remote".to_string(),
            url: Some(url.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn stdio_input_resolves_command_and_args() {
        let (transport, kind, command, args) =
            build_custom_transport(&stdio_input("npx")).expect("stdio resolves");
        assert_eq!(transport, Transport::Stdio);
        assert_eq!(kind, CommandKind::Node);
        assert_eq!(command, "npx");
        assert_eq!(args, vec!["-y".to_string(), "pkg".to_string()]);
    }

    /// Blank args are form noise (an empty row in the args editor), not
    /// arguments — passing them through would hand the subprocess an empty
    /// argv entry.
    #[test]
    fn stdio_input_drops_blank_args() {
        let mut input = stdio_input("npx");
        input.args = vec!["-y".to_string(), "   ".to_string(), String::new()];
        let (_, _, _, args) = build_custom_transport(&input).expect("stdio resolves");
        assert_eq!(args, vec!["-y".to_string()]);
    }

    #[test]
    fn stdio_input_requires_command() {
        let err = build_custom_transport(&stdio_input("   ")).expect_err("blank command rejected");
        assert!(err.contains("command must not be empty"), "got: {err}");
    }

    #[test]
    fn http_input_resolves_url() {
        let (transport, _, command, args) =
            build_custom_transport(&http_input("https://x.io/mcp")).expect("http resolves");
        assert_eq!(
            transport,
            Transport::HttpRemote {
                url: "https://x.io/mcp".to_string()
            }
        );
        assert!(command.is_empty(), "http_remote stores no command");
        assert!(args.is_empty(), "http_remote stores no args");
    }

    #[test]
    fn http_input_requires_url() {
        let err = build_custom_transport(&http_input("  ")).expect_err("blank url rejected");
        assert!(err.contains("url must not be empty"), "got: {err}");
    }

    #[test]
    fn http_input_rejects_relative_url() {
        let err = build_custom_transport(&http_input("/mcp")).expect_err("relative url rejected");
        assert!(err.contains("not a valid absolute URL"), "got: {err}");
    }

    /// A non-http scheme must never reach the HTTP client.
    #[test]
    fn http_input_rejects_non_http_scheme() {
        let err = build_custom_transport(&http_input("file:///etc/passwd"))
            .expect_err("file scheme rejected");
        assert!(err.contains("scheme must be http or https"), "got: {err}");
    }

    #[test]
    fn unknown_transport_is_rejected() {
        let mut input = stdio_input("npx");
        input.transport = "carrier-pigeon".to_string();
        let err = build_custom_transport(&input).expect_err("unknown transport rejected");
        assert!(
            err.contains("must be `stdio` or `http_remote`"),
            "got: {err}"
        );
    }

    /// A URL is stored verbatim and echoed back in `InstalledServer`, so a
    /// credential in the userinfo would be persisted in cleartext and returned.
    #[test]
    fn url_with_embedded_credentials_is_rejected() {
        for url in [
            "https://user:pass@host.example/mcp",
            "https://user@host.example/mcp",
            "http://alice:secret@127.0.0.1:8080/mcp",
        ] {
            let err = build_custom_transport(&http_input(url))
                .expect_err(&format!("embedded-credential URL `{url}` must be rejected"));
            assert!(err.contains("must not embed credentials"), "got: {err}");
        }
    }

    /// A credential-free URL still passes.
    #[test]
    fn url_without_credentials_is_accepted() {
        assert!(build_custom_transport(&http_input("https://host.example/mcp")).is_ok());
    }

    #[test]
    fn command_kind_is_inferred_from_launcher() {
        assert_eq!(infer_command_kind("npx"), CommandKind::Node);
        assert_eq!(infer_command_kind("uvx"), CommandKind::Python);
        assert_eq!(
            infer_command_kind("/usr/local/bin/python3"),
            CommandKind::Python
        );
        assert_eq!(infer_command_kind(r"C:\tools\npx.exe"), CommandKind::Node);
        assert_eq!(
            infer_command_kind("/opt/my-mcp-server"),
            CommandKind::Binary
        );
    }

    /// `__oauth__` holds the OAuth refresh bundle; a user-supplied `__` key
    /// would be dropped from outgoing headers anyway and could collide with it.
    #[test]
    fn reserved_env_keys_are_rejected() {
        let env = HashMap::from([("__oauth__".to_string(), "{}".to_string())]);
        let err = validate_env(&env, false).expect_err("reserved key rejected");
        assert!(err.contains("reserved"), "got: {err}");
    }

    /// Header names are case-insensitive (RFC 9110), so the core rejects a
    /// case-variant duplicate over /rpc even though the form also blocks it.
    #[test]
    fn case_variant_headers_are_rejected_on_http_remote() {
        let env = HashMap::from([
            ("Authorization".to_string(), "a".to_string()),
            ("authorization".to_string(), "b".to_string()),
        ]);
        let err = validate_env(&env, true).expect_err("case-variant header rejected");
        assert!(err.contains("more than once"), "got: {err}");
    }

    /// Env var names are case-sensitive on Unix, so stdio keeps both.
    #[cfg(not(windows))]
    #[test]
    fn case_variant_env_vars_are_allowed_on_stdio_unix() {
        let env = HashMap::from([
            ("Path".to_string(), "a".to_string()),
            ("PATH".to_string(), "b".to_string()),
        ]);
        assert!(validate_env(&env, false).is_ok());
    }

    /// Env var names are case-insensitive on Windows (`Path` == `PATH`), so a
    /// case-only stdio duplicate would collapse in the spawned process.
    #[cfg(windows)]
    #[test]
    fn case_variant_env_vars_are_rejected_on_stdio_windows() {
        let env = HashMap::from([
            ("Path".to_string(), "a".to_string()),
            ("PATH".to_string(), "b".to_string()),
        ]);
        let err = validate_env(&env, false).expect_err("case-variant env var rejected on Windows");
        assert!(err.contains("more than once"), "got: {err}");
    }

    /// The rows mean subprocess env on stdio and request headers on
    /// http_remote, so nothing stored survives a switch — a blank submitted key
    /// resolves to nothing rather than to the stored secret.
    #[test]
    fn transport_change_drops_stored_env() {
        let stored = HashMap::from([
            ("GITHUB_TOKEN".to_string(), "ghp_live".to_string()),
            (
                "__oauth__".to_string(),
                "{\"refresh_token\":\"r\"}".to_string(),
            ),
        ]);
        let submitted = HashMap::from([("GITHUB_TOKEN".to_string(), String::new())]);

        let resolved = resolve_env_for_transport(
            &submitted,
            &stored,
            &Transport::Stdio,
            &Transport::HttpRemote {
                url: "https://x.io/mcp".to_string(),
            },
        );

        assert!(
            resolved.is_empty(),
            "a stdio secret must not become a header on the new endpoint: {resolved:?}"
        );
    }

    /// `__oauth__` is a refresh bundle for one endpoint. `McpStdioClient` hands
    /// the whole env to the child process with no `__` filter, so carrying it
    /// into stdio would give a user-typed command the refresh token.
    #[test]
    fn transport_change_drops_the_oauth_bundle() {
        let stored = HashMap::from([(
            "__oauth__".to_string(),
            "{\"refresh_token\":\"r\",\"client_secret\":\"s\"}".to_string(),
        )]);

        let resolved = resolve_env_for_transport(
            &HashMap::new(),
            &stored,
            &Transport::HttpRemote {
                url: "https://x.io/mcp".to_string(),
            },
            &Transport::Stdio,
        );

        assert!(
            !resolved.contains_key("__oauth__"),
            "the OAuth bundle must not reach a subprocess: {resolved:?}"
        );
    }

    /// Same transport: the blank-means-keep contract holds, so an unrelated
    /// rename doesn't wipe the credentials.
    #[test]
    fn same_transport_keeps_stored_env() {
        let stored = HashMap::from([
            ("API_KEY".to_string(), "secret".to_string()),
            ("__oauth__".to_string(), "{}".to_string()),
        ]);
        let submitted = HashMap::from([("API_KEY".to_string(), String::new())]);

        let resolved =
            resolve_env_for_transport(&submitted, &stored, &Transport::Stdio, &Transport::Stdio);

        assert_eq!(resolved.get("API_KEY"), Some(&"secret".to_string()));
        assert_eq!(resolved.get("__oauth__"), Some(&"{}".to_string()));
    }

    /// A URL edit that keeps the same origin (path/query only) keeps the env —
    /// the bearer token and OAuth bundle are still valid for that origin.
    #[test]
    fn same_origin_url_change_keeps_stored_env() {
        let stored = HashMap::from([("Authorization".to_string(), "Bearer t".to_string())]);
        let submitted = HashMap::from([("Authorization".to_string(), String::new())]);

        let resolved = resolve_env_for_transport(
            &submitted,
            &stored,
            &Transport::HttpRemote {
                url: "https://svc.io/mcp".to_string(),
            },
            &Transport::HttpRemote {
                url: "https://svc.io/mcp/v2".to_string(),
            },
        );

        assert_eq!(resolved.get("Authorization"), Some(&"Bearer t".to_string()));
    }

    /// Pointing the server at a *different origin* re-scopes the credentials: a
    /// token minted for one service must not be sent to another. `__oauth__`
    /// would otherwise re-mint against the old endpoint and ship the result to
    /// the new host.
    #[test]
    fn cross_origin_url_change_drops_stored_env() {
        let stored = HashMap::from([
            ("Authorization".to_string(), "Bearer for-a".to_string()),
            (
                "__oauth__".to_string(),
                "{\"refresh_token\":\"r\"}".to_string(),
            ),
        ]);
        let submitted = HashMap::from([("Authorization".to_string(), String::new())]);

        let resolved = resolve_env_for_transport(
            &submitted,
            &stored,
            &Transport::HttpRemote {
                url: "https://a.com/mcp".to_string(),
            },
            &Transport::HttpRemote {
                url: "https://b.com/mcp".to_string(),
            },
        );

        assert!(
            resolved.is_empty(),
            "a token for a.com must not carry to b.com: {resolved:?}"
        );
    }

    /// A different port is a different origin.
    #[test]
    fn port_change_drops_stored_env() {
        let stored = HashMap::from([("Authorization".to_string(), "Bearer t".to_string())]);
        let resolved = resolve_env_for_transport(
            &HashMap::new(),
            &stored,
            &Transport::HttpRemote {
                url: "https://svc.io:8443/mcp".to_string(),
            },
            &Transport::HttpRemote {
                url: "https://svc.io:9443/mcp".to_string(),
            },
        );
        assert!(
            resolved.is_empty(),
            "different port re-scopes: {resolved:?}"
        );
    }

    #[test]
    fn ordinary_env_keys_are_accepted() {
        let env = HashMap::from([("Authorization".to_string(), "Bearer t".to_string())]);
        assert!(validate_env(&env, true).is_ok());
    }

    /// `resolve_env` trims before storing, so validating the raw key would let a
    /// padded `"  __oauth__"` through and land it as `__oauth__` — a caller
    /// could then plant a refresh bundle pointing at a token endpoint of their
    /// choosing. Validate what actually gets stored.
    #[test]
    fn reserved_env_keys_are_rejected_despite_padding() {
        for padded in ["  __oauth__", "__oauth__  ", "\t__oauth__"] {
            let env = HashMap::from([(padded.to_string(), "{}".to_string())]);
            let err = validate_env(&env, false)
                .expect_err(&format!("padded reserved key `{padded}` must be rejected"));
            assert!(err.contains("reserved"), "got: {err}");
        }
    }

    #[test]
    fn empty_env_key_is_rejected() {
        let env = HashMap::from([("  ".to_string(), "v".to_string())]);
        assert!(validate_env(&env, false).is_err());
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A retyped value wins over the stored one.
    #[test]
    fn resolve_env_takes_supplied_values() {
        let resolved = resolve_env(&env(&[("KEY", "new")]), &env(&[("KEY", "old")]), false);
        assert_eq!(resolved.get("KEY").map(String::as_str), Some("new"));
    }

    /// The edit form cannot render stored secrets (they are never returned), so
    /// an untouched row arrives blank. Blank must mean "keep" — dropping it
    /// would erase the credential on an unrelated rename.
    #[test]
    fn resolve_env_blank_value_keeps_stored_secret() {
        let resolved = resolve_env(&env(&[("KEY", "")]), &env(&[("KEY", "stored")]), false);
        assert_eq!(resolved.get("KEY").map(String::as_str), Some("stored"));
    }

    /// Removing a row must actually delete the key — this is why the edit path
    /// resolves against the submitted key set rather than merging like
    /// `mcp_clients_update_env` does.
    #[test]
    fn resolve_env_omitted_key_is_removed() {
        let resolved = resolve_env(
            &env(&[("KEEP", "v")]),
            &env(&[("KEEP", "v"), ("GONE", "x")]),
            false,
        );
        assert!(resolved.contains_key("KEEP"));
        assert!(
            !resolved.contains_key("GONE"),
            "omitted key must be dropped"
        );
    }

    /// The form never submits `__oauth__`, so without an explicit carry-over an
    /// edit would drop the refresh bundle and sign the user out of an
    /// OAuth-authenticated server.
    #[test]
    fn resolve_env_preserves_reserved_internal_state() {
        let resolved = resolve_env(
            &env(&[("Authorization", "Bearer new")]),
            &env(&[("__oauth__", "{\"refresh_token\":\"r\"}")]),
            true,
        );
        assert_eq!(
            resolved.get("__oauth__").map(String::as_str),
            Some("{\"refresh_token\":\"r\"}")
        );
        assert_eq!(
            resolved.get("Authorization").map(String::as_str),
            Some("Bearer new")
        );
    }

    /// http_remote header names are case-insensitive, so re-casing a header and
    /// leaving the value blank still means "keep" — a case-sensitive lookup would
    /// miss the stored value and silently erase the credential.
    #[test]
    fn resolve_env_blank_keeps_stored_header_across_case_change() {
        let resolved = resolve_env(
            &env(&[("authorization", "")]),
            &env(&[("Authorization", "Bearer keep")]),
            true,
        );
        assert_eq!(
            resolved.get("authorization").map(String::as_str),
            Some("Bearer keep"),
            "a re-cased header with a blank value must keep the stored secret"
        );
    }

    /// stdio env var names are case-sensitive, so the same blank re-cased key is
    /// a *different* key with nothing stored — not a keep.
    #[test]
    fn resolve_env_blank_recased_key_is_dropped_on_stdio() {
        let resolved = resolve_env(&env(&[("path", "")]), &env(&[("PATH", "/usr/bin")]), false);
        assert!(
            resolved.is_empty(),
            "a case-different env var is a new key, not a keep: {resolved:?}"
        );
    }

    /// On add there is nothing stored, so a blank row is simply not a value.
    #[test]
    fn resolve_env_drops_blank_value_with_nothing_stored() {
        let resolved = resolve_env(&env(&[("KEY", "  ")]), &HashMap::new(), false);
        assert!(resolved.is_empty());
    }

    #[test]
    fn env_key_list_is_sorted() {
        assert_eq!(
            env_key_list(&env(&[("b", "1"), ("a", "2")])),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn slugify_normalises_punctuation_and_case() {
        assert_eq!(slugify("My Cool Server").as_deref(), Some("my-cool-server"));
        assert_eq!(slugify("  @scope/thing!  ").as_deref(), Some("scope-thing"));
        assert_eq!(slugify("a---b").as_deref(), Some("a-b"));
    }

    /// A name with no ASCII alphanumerics yields no slug; the caller substitutes
    /// a per-server fragment so distinct non-Latin names don't collide.
    #[test]
    fn slugify_is_none_for_non_ascii_names() {
        assert_eq!(slugify("한글 서버"), None);
        assert_eq!(slugify("日本語"), None);
        assert_eq!(slugify(""), None);
    }

    /// The whole point of the `Option` return: two differently-named all-CJK
    /// servers get *different* base slugs from their unique server_ids, instead
    /// of both collapsing onto one constant and racing the collision suffix. The
    /// suffix loop itself needs the DB and is covered in `json_rpc_e2e`.
    #[test]
    fn base_slug_is_distinct_for_distinct_non_ascii_names() {
        let a = base_slug("한글 서버", "11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let b = base_slug("日本語サーバー", "22222222-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        assert_ne!(a, b, "distinct non-Latin names collided on `{a}`");
        assert_eq!(a, "server-11111111");
        assert_eq!(b, "server-22222222");
    }

    /// An ASCII name ignores the server_id and uses its own slug.
    #[test]
    fn base_slug_prefers_the_display_name_slug() {
        assert_eq!(base_slug("My Server", "unused-id"), "my-server");
    }
}
