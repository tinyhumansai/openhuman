//! The CLI's memory-capability gate — `docs/specs/kernel.md` §3.3's one named
//! exception to "degradation is absence".
//!
//! §3.3 says an unadvertised family is UNREGISTERED over `/rpc` and ABSENT from
//! the agent tool list, because a registered-but-failing method teaches a model
//! that the capability exists and makes it retry. It then names one exception:
//!
//! > The one exception is the **CLI**, which keeps its subcommand arm and
//! > reports a *build/config fact* ("memory driver `supermemory` does not
//! > support tree summarisation") — same reasoning as the retained `mcp` and
//! > `tui` CLI arms.
//!
//! A human reads silence as a typo and goes off debugging their own command
//! line. So the CLI — and ONLY the CLI — converts that absence into a sentence
//! naming the bound driver and the family it does not advertise. Nothing here
//! is reachable from `/rpc` or from an agent tool; `core::dispatch` is
//! deliberately untouched, because it is the shared HTTP path where §3.3's
//! absence rule still holds.
//!
//! ## Why this resolves the binding itself instead of asking the ambient context
//!
//! [`crate::core::all::capability_allowed`] resolves through
//! `CoreContext::current()`, and no CLI subcommand except `run`/`serve` (and the
//! TUI) ever builds a `CoreContext` — `DEFAULT_CONTEXT` is set only in
//! `CoreContext::init`. On a plain CLI invocation `current()` is `None`, the
//! gate defaults OPEN, and nothing is filtered. Asking the ambient context here
//! would therefore always answer "everything is allowed". This module resolves
//! the binding the same way `memory::ops::provider::status_from_binding` does.
//!
//! ## Redaction
//!
//! The message carries exactly two values: the driver id from
//! `[subsystems.memory] driver` and a `Capability::as_str()` constant. Neither
//! is a credential nor user content. `MemoryDriverConfig`'s `endpoint` and
//! `credential_ref` are NEVER interpolated — the same rule
//! `binding::FallbackReason` documents. The invocation string is built from
//! static namespace/function names, never from user-supplied argument values,
//! and no memory content can reach this path at all.

use anyhow::Result;
use tinymemory_api::capabilities::{Capabilities, Capability};

use crate::core::subsystem::DriverClass;

/// Stable, grep-friendly opening of the config-fact diagnostic. Shared between
/// the emit site and the tests so the two cannot drift.
pub const CAPABILITY_UNAVAILABLE_PREFIX: &str = "memory driver ";

/// Stable, grep-friendly opening of the legacy-client diagnostic. Shared the
/// same way as [`CAPABILITY_UNAVAILABLE_PREFIX`].
pub const LEGACY_CLIENT_UNAVAILABLE_PREFIX: &str = "memory driver ";

/// The operator-facing sentence.
///
/// `invocation` is a CLI form built from static strings only (e.g.
/// `"openhuman memory_tree list_chunks"`), never from user argument values.
pub fn capability_unavailable_message(
    driver_id: &str,
    capability: Capability,
    invocation: &str,
) -> String {
    format!(
        "{CAPABILITY_UNAVAILABLE_PREFIX}`{driver_id}` does not advertise the `{cap}` capability, \
         so `{invocation}` is unavailable in this configuration. Run `openhuman subsystems` to \
         see the bound driver and the families it advertises, or change \
         `[subsystems.memory] driver` in your config.",
        cap = capability.as_str()
    )
}

/// The operator-facing sentence for a legacy CLI command that cannot run under
/// the bound driver.
///
/// `invocation` follows the same static-strings-only rule as
/// [`capability_unavailable_message`]. This is a distinct diagnostic from the
/// capability one: a null or external binding does not necessarily lack the
/// family — the legacy command is unavailable because it operates on the
/// embedded engine directly, and the bound driver is not that engine.
///
/// The `class` parameter tailors the explanation: a `Null` driver is
/// unconfigured (no external source), while an `External` one answers from
/// a remote store — both refuse legacy subcommands for the same structural
/// reason but deserve different descriptions.
pub fn legacy_client_unavailable_message(
    driver_id: &str,
    class: DriverClass,
    invocation: &str,
) -> String {
    let detail = if class == DriverClass::Null {
        "no memory driver is configured".to_string()
    } else {
        format!(
            "this configuration bound a driver (`{driver_id}`) that answers from \
             a remote store, not this machine's embedded engine"
        )
    };
    format!(
        "{LEGACY_CLIENT_UNAVAILABLE_PREFIX}`{driver_id}` does not keep memory in the \
         local store, so `{invocation}` is unavailable: it reads that store \
         directly, and {detail}. Run `openhuman subsystems` to see the bound driver, \
         or change `[subsystems.memory] driver` in your config.",
    )
}

/// The pure verdict: given what a driver advertises, is `required` available?
///
/// `required == None` (ungated surface) is always `Ok`. Mirrors
/// `core::all::capability_allowed_in` exactly, so the CLI can never disagree
/// with the RPC registry about what is gated.
pub fn capability_verdict(
    driver_id: &str,
    advertised: Capabilities,
    required: Option<Capability>,
    invocation: &str,
) -> Result<()> {
    let Some(capability) = required else {
        return Ok(());
    };
    if advertised.contains(capability) {
        return Ok(());
    }
    log::warn!(
        "[cli][capability-gate] rejected invocation='{invocation}' driver='{driver_id}' \
         capability={} — not advertised by the bound driver",
        capability.as_str()
    );
    anyhow::bail!(capability_unavailable_message(
        driver_id, capability, invocation
    ))
}

/// The driver bound for this machine's configured workspace:
/// `(id, class, advertised)`.
///
/// `None` means "could not resolve" — a missing or unreadable config, or a
/// workspace that will not bind. **The caller then skips the gate entirely**,
/// matching [`crate::core::all::capability_allowed`]'s default-OPEN posture:
/// denying is only ever correct after a driver has actually answered
/// `capabilities()`. A CLI that refused commands because it could not read
/// config would be strictly worse than one that lets the command run and fail
/// on its own terms.
pub async fn bound_memory_driver() -> Option<(String, DriverClass, Capabilities)> {
    let config = match crate::openhuman::config::Config::load_or_init().await {
        Ok(config) => config,
        Err(err) => {
            log::debug!("[cli][capability-gate] config unresolved ({err}); gate defaults OPEN");
            return None;
        }
    };
    bound_memory_driver_for(&config.workspace_dir, &config.subsystems.memory)
}

/// [`bound_memory_driver`] against an already-loaded config.
///
/// The **single** place in the CLI layer that resolves a `MemoryBinding`, so the
/// memory-guard bypass ratchet
/// (`memory::bypass_allowlist_tests`) carries one allowlisted line rather than
/// one per CLI entry point. Callers that already hold a `Config` — the
/// `openhuman memory` adapter does — use this instead of loading it twice.
///
/// Nothing here touches memory *data*: only the driver id, its class, and the
/// advertised capability set — exactly what `memory.provider_status` already
/// reports over RPC.
pub fn bound_memory_driver_for(
    workspace_dir: &std::path::Path,
    cfg: &crate::openhuman::config::schema::MemorySubsystemConfig,
) -> Option<(String, DriverClass, Capabilities)> {
    match crate::openhuman::memory::binding::for_workspace(workspace_dir, cfg) {
        Ok(binding) => {
            log::debug!(
                "[cli][capability-gate] bound driver='{}' class={} capabilities=[{}]",
                binding.driver_id(),
                binding.class(),
                binding
                    .capabilities()
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            Some((
                binding.driver_id().to_string(),
                binding.class(),
                binding.capabilities(),
            ))
        }
        Err(err) => {
            log::debug!("[cli][capability-gate] bind unresolved ({err}); gate defaults OPEN");
            None
        }
    }
}

/// Blocking gate for the synchronous CLI call sites.
///
/// `required == None` short-circuits before any runtime is built or any config
/// is read, so a working command pays nothing: every call site reaches this
/// only on the failure path, and an ungated surface never gets that far.
///
/// A current-thread runtime is enough — resolving a binding touches config and
/// the driver's constructor, never an orchestrator turn.
pub fn ensure_capability_blocking(required: Option<Capability>, invocation: &str) -> Result<()> {
    let Some(required) = required else {
        return Ok(());
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let Some((driver_id, _class, advertised)) = rt.block_on(bound_memory_driver()) else {
        return Ok(());
    };
    capability_verdict(&driver_id, advertised, Some(required), invocation)
}

/// The pure verdict for the legacy-client gate: does the bound driver class
/// permit commands that operate on the embedded store directly?
///
/// Mirrors [`capability_verdict`]'s default-OPEN posture — `None` from the
/// caller means "no legacy gate applies", and an unresolvable binding has
/// already been defaulted-OPEN upstream by the caller skipping this entirely.
pub fn legacy_client_verdict(driver_id: &str, class: DriverClass, invocation: &str) -> Result<()> {
    // `Module` passes for the same reason `Embedded` does, and omitting it was
    // refusing every `openhuman memory` subcommand in the field: `binding::admit`
    // stopped admitting `Embedded` at all (the built-in driver binds as
    // `Module`), so a gate that only accepted `Embedded` accepted nothing.
    //
    // The gate's premise is about WHERE the memory lives, not how the driver is
    // linked. A module is a `cdylib` over the in-process bus with no egress and
    // no process boundary — the local store is still the store these
    // subcommands operate on. `External` and `Null` stay refused, and for the
    // reason the message gives: the local store is not the source of truth
    // there, so reading it directly would answer from the wrong place.
    if matches!(class, DriverClass::Embedded | DriverClass::Module) {
        return Ok(());
    }
    log::warn!(
        "[cli][legacy-client-gate] rejected invocation='{invocation}' driver='{driver_id}' \
         class={} — not the embedded engine",
        class.as_str()
    );
    anyhow::bail!(legacy_client_unavailable_message(
        driver_id, class, invocation
    ))
}

#[cfg(test)]
#[path = "cli_capability_tests.rs"]
mod tests;
