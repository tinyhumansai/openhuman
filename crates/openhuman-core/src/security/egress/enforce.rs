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
//! [`current_privacy_mode`](crate::security::live_policy::current_privacy_mode)
//! defaults to [`PrivacyMode::Standard`] (= no restriction) when no session
//! policy is installed (CLI / cron / background), so enforcement is a correct
//! no-op in those unmanaged contexts — exactly like the inference block.
//!
//! ## Control-plane exemption (do not brick sign-in)
//!
//! `LocalOnly` blocks the user's *data* from leaving the device — it must not
//! block the backend **control-plane** the app needs to stay usable (session /
//! auth / team / billing round-trips, plus the integration connection-management
//! and catalog reads that carry no user content). The descriptor's builder
//! classifies its own routes; see [`is_control_plane`].

use super::types::{EgressDescriptor, EgressReason};
use crate::config::PrivacyMode;
use crate::security::live_policy::current_privacy_mode;
use tinytools::status::POLICY_BLOCKED_MARKER;

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
/// tool calls, cloud embeddings, and agent network fetches all ship the user's
/// data and are never control-plane.
///
/// The core does not know any host's routes. The party that builds the
/// descriptor classifies its own path and sets
/// [`EgressDescriptor::control_plane`]; this predicate honours that flag and is
/// otherwise **fail-closed**: an integration descriptor nobody classified is
/// treated as shipping user data and stays blocked.
pub(crate) fn is_control_plane(desc: &EgressDescriptor) -> bool {
    desc.reason == EgressReason::Integration && desc.control_plane
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
