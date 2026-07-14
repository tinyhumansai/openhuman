//! Local-only egress enforcement (privacy epic S7, #4441).
//!
//! S2 (#4436) made every external transfer *observable*: each egress point
//! builds an [`EgressDescriptor`] and hands it to
//! [`emit_external_transfer`](super::emit::emit_external_transfer). This module
//! makes the restrictive privacy mode (`LocalOnly`, S1 #4435) *enforceable* at
//! those same chokepoints. Before a site discloses-and-sends, it asks whether
//! the live policy permits the transfer and refuses it when not.
//!
//! ## Shape (mirrors the S1 inference block)
//!
//! Like `inference/provider/factory.rs`'s `local_only_violation` /
//! `enforce_local_only_inference` pair, enforcement is split into:
//!
//! - [`local_only_blocks`] — a **pure decision** over `(mode, descriptor)`,
//!   unit-testable without the process-global live policy, and
//! - two thin side-effecting wrappers that read the live mode:
//!   - [`enforce_egress`] for `anyhow`-returning egress sites (composio,
//!     integrations, cloud embeddings) — `Err` with a clean, non-sensitive
//!     message when blocked, mirroring the inference `bail!`.
//!   - [`local_only_tool_block`] for agent tools whose `execute` returns
//!     `Ok(ToolResult::error(..))` on a denied action (the network tools) —
//!     returns the tool-facing, [`POLICY_BLOCKED_MARKER`]-prefixed message.
//!
//! [`current_privacy_mode`](crate::openhuman::security::live_policy::current_privacy_mode)
//! defaults to [`PrivacyMode::Standard`] (= no restriction) when no session
//! policy is installed (CLI / cron / background), so enforcement is a correct
//! no-op in those unmanaged contexts — exactly like the inference block.
//!
//! ## Control-plane exemption (do not brick sign-in)
//!
//! `LocalOnly` blocks the user's *data* from leaving the device — it must not
//! block the backend **control-plane** the app needs to stay usable (session /
//! auth / team / billing round-trips, plus the integration connection-management
//! and catalog reads that carry no user content). See [`is_control_plane`] for
//! the exact boundary and why it is drawn where it is.

use super::types::{EgressDescriptor, EgressReason};
use crate::openhuman::config::PrivacyMode;
use crate::openhuman::security::live_policy::current_privacy_mode;
use crate::openhuman::security::POLICY_BLOCKED_MARKER;

/// Pure decision: under privacy `mode`, is the transfer described by `desc`
/// blocked? `true` only when the mode is [`PrivacyMode::LocalOnly`], the
/// transfer actually leaves the device (`desc.is_external`), and it is **not**
/// an exempt backend control-plane round-trip ([`is_control_plane`]).
///
/// Every other mode (`Standard` / `Sensitive`) permits everything here — the
/// heightened-caution behaviours for `Sensitive` are approval/redaction slices,
/// not an egress block. Local runtimes (`desc.is_external == false`) are always
/// permitted: nothing leaves the device.
///
/// Extracted as a pure fn so the truth table is unit-testable without touching
/// the process-global live policy.
pub fn local_only_blocks(mode: PrivacyMode, desc: &EgressDescriptor) -> bool {
    mode == PrivacyMode::LocalOnly && desc.is_external && !is_control_plane(desc)
}

/// Is `desc` a backend **control-plane** round-trip that must keep flowing even
/// under `LocalOnly` (blocking it would break sign-in / the Connections UI with
/// no privacy benefit, because it carries no user *content* — only auth tokens,
/// ids, and routing metadata)?
///
/// Only [`EgressReason::Integration`] descriptors are ever eligible: inference,
/// composio tool calls, cloud embeddings, and agent network fetches all ship the
/// user's data and are never control-plane.
///
/// For an integration transfer, the descriptor's `service` is the backend
/// endpoint path (query stripped by
/// [`emit_backend_egress`](crate::openhuman::integrations)). Two shapes are
/// exempt:
///
/// 1. **Any path outside the user-data tool namespace** — i.e. it is *not* under
///    `/agent-integrations/`. Today every user-data integration call (composio
///    execute, parallel / tinyfish / financial-apis / google-places / twilio /
///    apify research + actions, file-storage uploads) routes through
///    `IntegrationClient` under `/agent-integrations/…`, while session / team /
///    billing / auth round-trips (`/teams/me/usage`, `/payments/…`, `/auth/…`)
///    go through `api::rest` and never build an egress descriptor at all. Any
///    such path only reaches here if a future caller re-homes a control-plane
///    call onto this descriptor — exempt it defensively so a re-home can never
///    brick login (per the epic's "do not block control-plane" directive).
///
/// 2. **A narrow allow-list of read-only composio connection-management /
///    catalog / OAuth sub-routes** that set up or *describe* an integration
///    without shipping or mutating user content: `connections` (list +
///    per-connection delete), `authorize` (OAuth handoff), `tools` and
///    `toolkits` (catalog), plus `pricing` (billing metadata). These are the
///    routes the Connections UI needs to stay usable under `LocalOnly`.
///
/// Everything else under `composio/` is **blocked**, including:
///   - `execute` — ships the tool arguments (the original user-data path);
///   - `triggers` / `triggers/available` — `create_trigger` / `enable_trigger`
///     POST user-supplied `slug` / `connectionId` / `triggerConfig` (a
///     user-data *write*); the descriptor carries only the path, not the HTTP
///     method, so the read variants on the same path are blocked with them
///     (fail-closed — trigger setup is an integration write surface, not
///     sign-in, so blocking it under `LocalOnly` breaks nothing that matters);
///   - `github/repos` and any future sub-route — reveals user-adjacent data,
///     and an *unknown* composio sub-route is treated as user-data, never
///     exempt.
///
/// The boundary is deliberately **fail-closed** for user data: only the named
/// read-only routes are exempt; anything else under `/agent-integrations/`
/// (composio or otherwise) ships user data and stays blocked. Genuine
/// non-composio control-plane (session / auth / team / billing) is handled by
/// rule (1) so sign-in never breaks.
pub(crate) fn is_control_plane(desc: &EgressDescriptor) -> bool {
    if desc.reason != EgressReason::Integration {
        return false;
    }
    let path = desc.service.trim_start_matches('/');
    // (1) Not under the user-data tool namespace → control-plane (session /
    //     auth / team / billing, or a defensively re-homed control-plane call).
    let Some(subroute) = path.strip_prefix("agent-integrations/") else {
        return true;
    };
    // (2) Pricing metadata carries no user content.
    if subroute == "pricing" {
        return true;
    }
    // (3) Composio: exempt ONLY the read-only connection-management / catalog /
    //     OAuth allow-list. `execute` (tool arguments), `triggers[/available]`
    //     (user-data writes), `github/repos`, and any unknown sub-route stay
    //     blocked — fail-closed for user data.
    if let Some(rest) = subroute.strip_prefix("composio/") {
        let head = rest.split(['/', '?']).next().unwrap_or(rest);
        return matches!(head, "connections" | "authorize" | "tools" | "toolkits");
    }
    // Everything else under /agent-integrations/ ships user data → not exempt.
    false
}

/// Human-readable, non-sensitive reason the transfer was blocked. Names only the
/// destination *service* (already a coarse slug / endpoint / host on the
/// descriptor — never raw payload) and how to lift the block. No marker; callers
/// that need the [`POLICY_BLOCKED_MARKER`] prefix add it (see
/// [`local_only_tool_block`]).
fn block_message(desc: &EgressDescriptor) -> String {
    format!(
        "Local-only privacy mode is active: this action needs external service \
         `{}`. Disable local-only mode in Settings to allow it.",
        desc.service
    )
}

/// Enforce the live privacy policy for an `anyhow`-returning egress site
/// (composio tool calls, backend integrations, cloud embeddings). Reads the live
/// mode (defaulting to `Standard`/allow when no session policy is installed) and
/// returns `Err(block_message)` when [`local_only_blocks`] refuses the transfer,
/// else `Ok(())`. Call this **before** the disclose-and-send (i.e. before
/// [`emit_external_transfer`](super::emit::emit_external_transfer)) so a blocked
/// transfer is neither disclosed as pending nor dispatched.
pub fn enforce_egress(desc: &EgressDescriptor) -> anyhow::Result<()> {
    let mode = current_privacy_mode();
    if local_only_blocks(mode, desc) {
        log::warn!(
            "[privacy][egress-enforce] LocalOnly BLOCK provider={} service={} reason={:?} — refused",
            desc.provider_slug,
            desc.service,
            desc.reason,
        );
        anyhow::bail!("{}", block_message(desc));
    }
    log::debug!(
        "[privacy][egress-enforce] privacy_mode={:?} provider={} service={} reason={:?} — permitted",
        mode,
        desc.provider_slug,
        desc.service,
        desc.reason,
    );
    Ok(())
}

/// Enforce the live privacy policy for an agent tool whose `execute` returns
/// `Ok(ToolResult::error(..))` on a denied action (the network tools). Returns
/// `Some(message)` — prefixed with [`POLICY_BLOCKED_MARKER`] so the agent loop
/// treats it as a hard, cross-turn policy block (no pointless retries) — when the
/// transfer is refused, else `None`. Call it before the disclose-and-send; on
/// `Some`, short-circuit with `Ok(ToolResult::error(message))`.
pub fn local_only_tool_block(desc: &EgressDescriptor) -> Option<String> {
    let mode = current_privacy_mode();
    if local_only_blocks(mode, desc) {
        log::warn!(
            "[privacy][egress-enforce] LocalOnly BLOCK (tool) provider={} service={} reason={:?} — refused",
            desc.provider_slug,
            desc.service,
            desc.reason,
        );
        Some(format!("{POLICY_BLOCKED_MARKER} {}", block_message(desc)))
    } else {
        log::debug!(
            "[privacy][egress-enforce] privacy_mode={:?} (tool) provider={} service={} reason={:?} — permitted",
            mode,
            desc.provider_slug,
            desc.service,
            desc.reason,
        );
        None
    }
}

#[cfg(test)]
#[path = "enforce_tests.rs"]
mod tests;
