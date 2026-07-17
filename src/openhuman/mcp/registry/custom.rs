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
fn resolve_env(
    submitted: &HashMap<String, String>,
    stored: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut resolved: HashMap<String, String> = HashMap::new();
    for (key, value) in submitted {
        let key = key.trim().to_string();
        if !value.trim().is_empty() {
            resolved.insert(key, value.clone());
        } else if let Some(existing) = stored.get(&key) {
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

/// Sorted key list for the install record, matching what `update_env` persists.
fn env_key_list(env: &HashMap<String, String>) -> Vec<String> {
    let mut keys: Vec<String> = env.keys().cloned().collect();
    keys.sort();
    keys
}

/// Reject env keys that are empty or claim the reserved `__` namespace.
///
/// Validates the key **as `resolve_env` will store it** — i.e. trimmed. Checking
/// the raw key would let `"  __oauth__"` through here and then land as
/// `__oauth__` after the trim, defeating the guard by the normalisation applied
/// downstream.
fn validate_env(env: &HashMap<String, String>) -> Result<(), String> {
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
    }
    Ok(())
}

/// ASCII slug for the `custom/<slug>` identity.
///
/// Deliberately ASCII-only: `qualified_name` is an internal identifier that
/// flows into logs and dedupe lookups, while the UI always renders
/// `display_name`. A name with no ASCII alphanumerics (e.g. all-CJK) collapses
/// to `server`, and the caller's collision suffix makes it unique.
fn slugify(raw: &str) -> String {
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
        "server".to_string()
    } else {
        slug
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
fn allocate_qualified_name(config: &Config, display_name: &str) -> Result<String, String> {
    let base = format!("{CUSTOM_QUALIFIED_PREFIX}{}", slugify(display_name));
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
    validate_env(&input.env)?;
    let (transport, command_kind, command, args) = build_custom_transport(&input)?;

    // Nothing is stored yet, so this only drops blank-valued keys.
    let env = resolve_env(&input.env, &HashMap::new());

    let qualified_name = allocate_qualified_name(config, &display_name)?;
    let server_id = Uuid::new_v4().to_string();

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
    if !store::insert_server_if_absent(config, &server).map_err(|e| e.to_string())? {
        return Err(format!(
            "the name `{qualified_name}` was taken by a concurrent add; please retry"
        ));
    }
    store::set_env_values(config, &server_id, &env).map_err(|e| e.to_string())?;

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
    validate_env(&input.env)?;

    let existing = store::get_server(config, &server_id).map_err(|e| e.to_string())?;
    if existing.provenance != ServerProvenance::Custom {
        return Err(format!(
            "server `{server_id}` was installed from a registry; its command and endpoint come from the catalog listing and cannot be edited here"
        ));
    }

    let (transport, command_kind, command, args) = build_custom_transport(&input)?;

    // Propagate a failed env read rather than treating it as an empty base, the
    // same rule `refresh_existing_install` follows. It matters more here: this
    // path treats the submitted key set as authoritative, so an empty base does
    // not merely fail to merge — every blank-valued key resolves to nothing, the
    // reserved carry-over finds nothing to carry, and the `set_env_values` below
    // deletes the row's entire env (secrets and the `__oauth__` bundle) while
    // still returning Ok.
    let stored_env = store::load_env_values(config, &server_id).map_err(|e| {
        format!("failed to read stored env for `{server_id}`; refusing to update: {e}")
    })?;
    let env = resolve_env(&input.env, &stored_env);

    tracing::debug!(
        "[mcp-custom] update server_id={} transport={} env_keys={:?}",
        server_id,
        transport.dispatch_kind(),
        env.keys().collect::<Vec<_>>()
    );

    // Drop the live connection first: it was dialed with the previous command
    // or URL, so leaving it up would keep serving tools from the old
    // configuration until something unrelated happened to reconnect.
    connections::disconnect(&server_id).await;

    let updated = InstalledServer {
        // Identity and provenance survive an edit untouched — re-deriving
        // `qualified_name` from the new display name would orphan this row's
        // env values.
        server_id: existing.server_id.clone(),
        qualified_name: existing.qualified_name.clone(),
        installed_at: existing.installed_at,
        provenance: existing.provenance,
        icon_url: existing.icon_url.clone(),
        config: existing.config.clone(),
        enabled: existing.enabled,
        last_connected_at: existing.last_connected_at,

        display_name,
        description: clean_description(input.description.clone()),
        command_kind,
        command,
        args,
        env_keys: env_key_list(&env),
        transport,
    };

    store::update_server_custom_fields(config, &server_id, &updated).map_err(|e| e.to_string())?;
    store::set_env_values(config, &server_id, &env).map_err(|e| e.to_string())?;

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
        let err = validate_env(&env).expect_err("reserved key rejected");
        assert!(err.contains("reserved"), "got: {err}");
    }

    #[test]
    fn ordinary_env_keys_are_accepted() {
        let env = HashMap::from([("Authorization".to_string(), "Bearer t".to_string())]);
        assert!(validate_env(&env).is_ok());
    }

    /// `resolve_env` trims before storing, so validating the raw key would let a
    /// padded `"  __oauth__"` through and land it as `__oauth__` — a caller
    /// could then plant a refresh bundle pointing at a token endpoint of their
    /// choosing. Validate what actually gets stored.
    #[test]
    fn reserved_env_keys_are_rejected_despite_padding() {
        for padded in ["  __oauth__", "__oauth__  ", "\t__oauth__"] {
            let env = HashMap::from([(padded.to_string(), "{}".to_string())]);
            let err = validate_env(&env)
                .expect_err(&format!("padded reserved key `{padded}` must be rejected"));
            assert!(err.contains("reserved"), "got: {err}");
        }
    }

    #[test]
    fn empty_env_key_is_rejected() {
        let env = HashMap::from([("  ".to_string(), "v".to_string())]);
        assert!(validate_env(&env).is_err());
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
        let resolved = resolve_env(&env(&[("KEY", "new")]), &env(&[("KEY", "old")]));
        assert_eq!(resolved.get("KEY").map(String::as_str), Some("new"));
    }

    /// The edit form cannot render stored secrets (they are never returned), so
    /// an untouched row arrives blank. Blank must mean "keep" — dropping it
    /// would erase the credential on an unrelated rename.
    #[test]
    fn resolve_env_blank_value_keeps_stored_secret() {
        let resolved = resolve_env(&env(&[("KEY", "")]), &env(&[("KEY", "stored")]));
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

    /// On add there is nothing stored, so a blank row is simply not a value.
    #[test]
    fn resolve_env_drops_blank_value_with_nothing_stored() {
        let resolved = resolve_env(&env(&[("KEY", "  ")]), &HashMap::new());
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
        assert_eq!(slugify("My Cool Server"), "my-cool-server");
        assert_eq!(slugify("  @scope/thing!  "), "scope-thing");
        assert_eq!(slugify("a---b"), "a-b");
    }

    /// A name with no ASCII alphanumerics still needs an identity; the caller's
    /// collision suffix keeps repeats unique.
    #[test]
    fn slugify_falls_back_for_non_ascii_names() {
        assert_eq!(slugify("한글 서버"), "server");
        assert_eq!(slugify(""), "server");
    }
}
