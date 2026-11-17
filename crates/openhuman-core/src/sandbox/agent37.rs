//! The agent37 sandbox backend: the tenant's retained box on the hosted
//! agent37 service.
//!
//! With `[security.sandbox] backend = "agent37"`, a sandboxed shell command
//! runs in a box the TinyBox module keeps for the calling tenant on agent37's
//! hosted HTTP service, through the shared box registry in
//! [`boxes`](super::boxes) — the same per-tenant registry the OpenShell
//! backend uses. The agent37 module backend (`tinybox-agent37`) owns the HTTP
//! transport and its credential (`AGENT37_API_KEY`, with the hosts fixed and
//! overridable via `AGENT37_API_BASE` / `AGENT37_SANDBOX_BASE`); OpenHuman
//! maps policy onto the create request and runs the command.
//!
//! Each box is created with box contract 2.4 retention, keyed on an opaque
//! hash of the tenant ([`boxes::owner_key`](super::boxes::owner_key)): it
//! sleeps after `[runtime.agent37] idle_sleep_secs` without a command, wakes
//! on the next one, and is deleted only after `delete_after_secs` asleep. The
//! host never deletes it: going idle, a release and process shutdown only
//! forget it, so the tenant's files survive across commands and restarts.
//! Commands run in `/home/node`, wrapped in `timeout` so the box stops them
//! itself. Template, CPU and memory come from `[runtime.agent37]` (the
//! service snaps CPU and memory up to a fixed instance shape, so they are best
//! effort); network is always `Denied`.
//!
//! Under `[privacy] mode = "local_only"` nothing is sent: the command, its
//! environment and any file would leave the device.

use std::collections::HashMap;
use std::ffi::OsString;
use std::time::Duration;

use super::boxes::BoxBackend;
use super::types::SandboxExecResult;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;

/// Run `command` in the calling tenant's agent37 box and return its result.
///
/// # Errors
///
/// Returns an error under local-only privacy, for a call with no tenant, or
/// when the TinyBox module cannot create the box or run the command (for
/// example, no `AGENT37_API_KEY`), or when module support is compiled out.
pub(crate) async fn execute(
    ctx: &DispatchCtx,
    config: &Config,
    command: &str,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    super::boxes::execute(
        ctx,
        config,
        BoxBackend::Agent37,
        command,
        extra_env,
        timeout,
    )
    .await
}

#[cfg(test)]
#[path = "agent37_tests.rs"]
mod tests;
