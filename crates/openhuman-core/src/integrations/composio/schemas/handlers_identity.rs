//! User-profile, identity-sync, and per-toolkit scope-preference handlers:
//! `get_user_profile`, `refresh_all_identities`, `sync`, `get_user_scopes`,
//! `set_user_scopes`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::integrations::composio::{contract, ops};

use super::util::{read_required, read_required_non_empty, to_json};

pub(super) fn handle_get_user_profile(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        to_json(ops::composio_get_user_profile(&ctx, &config, &connection_id).await?)
    })
}

pub(super) fn handle_refresh_all_identities(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(ops::composio_refresh_all_identities(&ctx, &config).await?)
    })
}

pub(super) fn handle_get_user_scopes(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
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
        // Fails closed: a pref the module cannot read is an error, not the
        // default. The default is `read+write`, so showing it for a user who
        // turned `write` off would misstate what the agent may run; the
        // Connections panel renders the error instead.
        let config = ctx.load_config().await?;
        let pref = match ops::load_user_scope_pref(&ctx, &config, &toolkit).await {
            Ok(pref) => pref,
            Err(error) => {
                tracing::warn!(
                    method = "composio.get_user_scopes",
                    toolkit = %toolkit,
                    %error,
                    "[composio:scopes] pref unavailable"
                );
                return Err(format!(
                    "scope preference for `{toolkit}` is unavailable: {error}"
                ));
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

pub(super) fn handle_set_user_scopes(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
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
        let config = ctx.load_config().await?;
        if let Err(e) = ops::save_user_scope_pref(&ctx, &config, &toolkit, pref).await {
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
