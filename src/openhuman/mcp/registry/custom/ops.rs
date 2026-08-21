//! Custom-server RPC operations: add and update hand-entered installs.
//!
//! The `RpcOutcome` handlers `mcp_registry::schemas` delegates to. Business
//! logic + persistence live here; the pure validation/transport helpers are in
//! `super::validate`.

use std::collections::HashMap;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::mcp::registry::types::{InstalledServer, ServerProvenance};
use crate::openhuman::mcp::registry::{connections, store};
use crate::rpc::RpcOutcome;

use super::validate::{
    base_slug, build_custom_transport, clean_description, credential_scope, env_key_list,
    resolve_env, resolve_env_for_transport, validate_env,
};
use super::{CustomServerInput, CUSTOM_QUALIFIED_PREFIX, MAX_SLUG_ATTEMPTS};

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
