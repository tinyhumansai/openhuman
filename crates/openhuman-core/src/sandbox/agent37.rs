//! The agent37 sandbox backend: a box per command on the hosted agent37 service.
//!
//! With `[security.sandbox] backend = "agent37"`, a sandboxed shell command
//! runs in a box the TinyBox module creates on agent37's hosted HTTP service,
//! through the module's *generic* resource lifecycle (`Reserve` → `Create` with
//! backend `"agent37"` → `Exec` → `Close`) — the same contract the Docker and
//! OpenShell backends use. The agent37 module backend (`tinybox-agent37`) owns
//! the HTTP transport and its credential (`AGENT37_API_KEY`, with the hosts
//! fixed and overridable via `AGENT37_API_BASE` / `AGENT37_SANDBOX_BASE`);
//! OpenHuman only maps policy onto the create request and runs the command.
//!
//! Unlike OpenShell, which keeps one long-lived box per tenant, this backend
//! currently creates a box per command and closes it afterwards. The box
//! template, CPU and memory come from `[runtime.agent37]` (the service snaps
//! CPU and memory up to a fixed instance shape, so they are best effort);
//! network is always `Denied`, since widening egress is the service's
//! decision, not this host's. The command is wrapped in `timeout` so the box
//! stops it itself, exactly as the OpenShell backend does.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::time::Duration;

use super::types::SandboxExecResult;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;

/// The TinyBox backend name for agent37 boxes.
pub(crate) const BACKEND: &str = "agent37";

/// `timeout` reports this exit code when it stops a command (coreutils).
const TIMED_OUT_EXIT: i32 = 124;

/// Grace added to the command deadline for the bus round-trip, so the box's own
/// `timeout` fires before the bus call gives up.
const BUS_REPLY_GRACE: Duration = Duration::from_secs(10);

/// Run `command` in a fresh agent37 box and return its result.
///
/// # Errors
///
/// Returns an error when the TinyBox module cannot create the box or run the
/// command (for example, no `AGENT37_API_KEY`), or when module support is
/// compiled out.
pub(crate) async fn execute(
    _ctx: &DispatchCtx,
    config: &Config,
    command: &str,
    extra_env: &std::collections::HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    // agent37 runs the command — and its environment — on a hosted service off
    // this device. Under local-only privacy that egress is not permitted, so
    // refuse before anything leaves the host. This holds independently of the
    // security module, so it is enforced even when security is disabled.
    if local_only_forbids_agent37(config) {
        anyhow::bail!(
            "agent37 sandbox execution is unavailable under [privacy] mode = \"local_only\": \
             it would send the command and its environment to a hosted service"
        );
    }
    #[cfg(feature = "modules")]
    {
        tracing::debug!(
            timeout_secs = timeout.as_secs(),
            "[sandbox:agent37] running a command"
        );
        run_command(config, command, extra_env, timeout).await
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (config, command, extra_env, timeout);
        anyhow::bail!(
            "TinyBox module support is disabled; agent37 sandbox execution is unavailable"
        )
    }
}

/// Whether `[privacy] mode = "local_only"` forbids agent37 execution. agent37
/// sends the command and its environment to a hosted service, which local-only
/// privacy does not permit; every other mode allows it.
fn local_only_forbids_agent37(config: &Config) -> bool {
    config.privacy.mode == crate::config::PrivacyMode::LocalOnly
}

/// Owns an agent37 reservation between create and close so a cancelled turn
/// still tears the hosted box down. While armed (`resource` is `Some`), dropping
/// the guard — which is what a cancelled future does to its locals — closes the
/// box on a detached task, since `Drop` cannot await. The happy path disarms it
/// with [`BoxGuard::disarm`] and closes the box itself.
#[cfg(feature = "modules")]
struct BoxGuard {
    config: Config,
    resource: Option<crate::modules::tinybox::contract::ResourceId>,
}

#[cfg(feature = "modules")]
impl BoxGuard {
    fn disarm(&mut self) -> Option<crate::modules::tinybox::contract::ResourceId> {
        self.resource.take()
    }
}

#[cfg(feature = "modules")]
impl Drop for BoxGuard {
    fn drop(&mut self) {
        let Some(resource) = self.resource.take() else {
            return;
        };
        let config = self.config.clone();
        // Cancelled between create and close. Drop cannot await, so close the box
        // on a detached task rather than leak it until the service's idle reaper
        // takes it.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = crate::modules::tinybox::close(&config, resource).await {
                    tracing::warn!(%error, "[sandbox:agent37] cancelled-box cleanup failed");
                }
            });
        }
    }
}

/// The limits a box is created with, from `[runtime.agent37]` rather than
/// `[runtime.docker]`. The service snaps them up to a fixed instance shape, so
/// they are a request, not a ceiling. CPU above 4 vCPU is capped before a shape
/// is chosen, and memory above 32 GiB gets the largest shape (logged, since the
/// box then gets less than was configured). A memory size too large to express
/// in bytes asks for the service's maximum.
#[cfg(feature = "modules")]
fn box_resources(config: &Config) -> crate::modules::tinybox::contract::ResourceLimits {
    let agent37 = &config.runtime.agent37;
    if agent37.exceeds_largest_shape() {
        tracing::warn!(
            cpu_millis = agent37.cpu_millis,
            memory_mb = agent37.memory_mb,
            "[sandbox:agent37] request exceeds the largest instance shape and will be capped"
        );
    }
    crate::modules::tinybox::contract::ResourceLimits {
        cpu_millis: agent37.cpu_millis,
        memory_bytes: agent37.memory_bytes().unwrap_or(u64::MAX),
        ..Default::default()
    }
}

#[cfg(feature = "modules")]
async fn run_command(
    config: &Config,
    command: &str,
    extra_env: &std::collections::HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    use crate::modules::tinybox::{self, contract};

    let resource = tinybox::reserve(config, contract::ReserveRequest::Resource)
        .await
        .map_err(|error| anyhow::anyhow!("agent37 reserve failed: {error}"))?;

    // From here the box must be closed on every path, cancellation included.
    let mut guard = BoxGuard {
        config: config.clone(),
        resource: Some(resource.clone()),
    };

    if let Err(error) = tinybox::create(
        config,
        contract::CreateRequest {
            resource: resource.clone(),
            backend: BACKEND.to_owned(),
            host: contract::HostConfig::Local,
            // agent37 starts a box from a registered template name, not an OCI
            // image; forwarding the Docker image slot is rejected with HTTP 400.
            workspace: contract::Workspace::Image(config.runtime.agent37.template.clone()),
            network: contract::NetworkPolicy::Denied,
            resources: box_resources(config),
            ports: Vec::new(),
            env: BTreeMap::new(),
        },
    )
    .await
    {
        // No box to close: the backend tears a half-created instance down
        // itself, so disarm rather than close an id the service never kept.
        guard.disarm();
        return Err(anyhow::anyhow!("agent37 create failed: {error}"));
    }

    let secs = whole_seconds(timeout);
    let outcome = tinybox::exec_within(
        config,
        contract::ExecRequest {
            resource: resource.clone(),
            argv: timeout_argv(secs, command),
            cwd: None,
            env: utf8_env(extra_env),
            stdin: None,
        },
        timeout.saturating_add(BUS_REPLY_GRACE),
    )
    .await;

    // Close the box ourselves, but keep the guard armed across the await: a turn
    // cancelled during this close drops the future, and the still-armed guard
    // then closes the box on its detached task. Disarm only once close has
    // returned, so the reservation is never closed twice.
    if let Some(resource) = guard.resource.clone() {
        let _ = tinybox::close(config, resource).await;
        guard.disarm();
    }

    match outcome {
        Ok(output) => Ok(SandboxExecResult {
            exit_code: output.exit_code,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            timed_out: output.exit_code == TIMED_OUT_EXIT,
        }),
        Err(error) => Err(anyhow::anyhow!("agent37 execution failed: {error}")),
    }
}

/// `timeout -k 5 <secs> sh -c <command>`: the box stops the command itself.
#[cfg(feature = "modules")]
fn timeout_argv(secs: u64, command: &str) -> Vec<String> {
    ["timeout", "-k", "5", &secs.to_string(), "sh", "-c", command]
        .map(str::to_owned)
        .to_vec()
}

/// `timeout` counts whole seconds: round up, and never ask for none.
#[cfg(feature = "modules")]
fn whole_seconds(timeout: Duration) -> u64 {
    (timeout.as_secs() + u64::from(timeout.subsec_nanos() > 0)).max(1)
}

/// The per-command environment as UTF-8 string pairs, dropping any entry whose
/// name or value is not UTF-8 (the module contract carries strings).
#[cfg(feature = "modules")]
fn utf8_env(extra_env: &std::collections::HashMap<OsString, OsString>) -> BTreeMap<String, String> {
    extra_env
        .iter()
        .filter_map(|(name, value)| Some((name.to_str()?.to_owned(), value.to_str()?.to_owned())))
        .collect()
}

#[cfg(test)]
#[path = "agent37_tests.rs"]
mod tests;
