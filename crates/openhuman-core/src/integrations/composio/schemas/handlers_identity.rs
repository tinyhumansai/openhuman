//! User-profile, identity-sync, and per-toolkit scope-preference handlers:
//! `get_user_profile`, `refresh_all_identities`, `sync`, `get_user_scopes`,
//! `set_user_scopes`.

use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;
use crate::integrations::composio::{contract, ops};

use super::util::{read_required, read_required_non_empty, to_json};

pub(super) fn handle_get_user_profile(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        to_json(ops::composio_get_user_profile(&config, &connection_id).await?)
    })
}

pub(super) fn handle_refresh_all_identities(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::composio_refresh_all_identities(&config).await?)
    })
}

pub(super) fn handle_get_user_scopes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let toolkit = match read_required_non_empty(&params, "toolkit") {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    method = "composio.get_user_scopes",
                    error = %e,
                    "[composio:scopes] missing required `toolkit` param"
                );
                return Err(e);
            }
        };
        tracing::debug!(
            method = "composio.get_user_scopes",
            toolkit = %toolkit,
            "[composio:scopes] handler entry"
        );
        // Reads through the bound memory driver's `Graph` family, not through
        // the engine's `load_or_default` (which resolved the in-process client
        // itself — openhuman#5560 deleted that client). The read still fails
        // OPEN onto the default pref for every failure mode, deliberately; see
        // `ops::user_scopes`.
        //
        // The config load is `?`-free for the same reason: this handler has
        // never had a failing path, and turning "settings file momentarily
        // unreadable" into an RPC error would make the scopes panel fail where
        // it used to render the defaults.
        let pref = match config_rpc::load_config_with_timeout().await {
            Ok(config) => ops::load_user_scope_pref(&config, &toolkit).await,
            Err(error) => {
                tracing::warn!(
                    method = "composio.get_user_scopes",
                    toolkit = %toolkit,
                    %error,
                    "[composio:scopes] config load failed, using default pref (read+write)"
                );
                contract::UserScopePref::default()
            }
        };
        tracing::debug!(
            method = "composio.get_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler exit"
        );
        to_json(crate::core::Outcome::new(pref, vec![]))
    })
}

pub(super) fn handle_set_user_scopes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let toolkit = match read_required_non_empty(&params, "toolkit") {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    method = "composio.set_user_scopes",
                    error = %e,
                    "[composio:scopes] missing required `toolkit` param"
                );
                return Err(e);
            }
        };
        let read: bool = read_required(&params, "read")?;
        let write: bool = read_required(&params, "write")?;
        let admin: bool = read_required(&params, "admin")?;
        let pref = contract::UserScopePref { read, write, admin };
        tracing::debug!(
            method = "composio.set_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler entry"
        );
        // Writes through the bound memory driver's `Graph` family. This half
        // fails CLOSED, as it did before: the old code refused with "memory
        // client not initialised" rather than reporting a save it had not
        // done, and `ops::user_scopes::save` refuses on the same three
        // grounds (no driver, no `Graph` family, backend write failure).
        let config = config_rpc::load_config_with_timeout().await?;
        if let Err(e) = ops::save_user_scope_pref(&config, &toolkit, pref).await {
            tracing::error!(
                method = "composio.set_user_scopes",
                toolkit = %toolkit,
                error = %e,
                "[composio:scopes] save failed"
            );
            return Err(e);
        }
        tracing::debug!(
            method = "composio.set_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler exit"
        );
        to_json(crate::core::Outcome::new(pref, vec![]))
    })
}
