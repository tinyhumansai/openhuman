//! Shared core-level type definitions and response formats.
//!
//! This module contains structs and methods for handling RPC requests and
//! responses, as well as maintaining application state across subsystems.

use serde::Serialize;

/// Success payload from a core RPC handler before JSON-RPC wrapping.
///
/// This internal type allows handlers to return a generic JSON value along
/// with optional logs. It is transformed into a [`crate::core::RpcSuccess`] or a
/// combined object by [`invocation_to_rpc_json`].
#[derive(Debug, Clone)]
pub struct InvocationResult {
    /// The value returned by the RPC function call, serialized to JSON.
    pub value: serde_json::Value,
    /// A list of execution logs.
    pub logs: Vec<String>,
}

impl InvocationResult {
    /// Creates a success result from any serializable value with no logs.
    ///
    /// This is the most common way to return a value from a controller.
    pub fn ok<T: Serialize>(v: T) -> Result<Self, String> {
        Ok(Self {
            value: serde_json::to_value(v).map_err(|e| e.to_string())?,
            logs: vec![],
        })
    }
}

/// Formats an [`InvocationResult`] into its standard JSON-RPC format.
///
/// If there are no logs, returns the value directly. Otherwise, returns an
/// object containing both `result` and `logs` keys.
///
/// # Logic
///
/// - `logs.is_empty()` -> `inv.value`
/// - `!logs.is_empty()` -> `{ "result": inv.value, "logs": inv.logs }`
pub fn invocation_to_rpc_json(inv: InvocationResult) -> serde_json::Value {
    // Delegates rather than repeating the rule. This function and
    // `Outcome::into_cli_compatible_json` are the two ways a controller
    // result reaches a caller, and they carried independent copies of the same
    // six lines — so a fix to one would have silently left the other on the old
    // shape. See `crate::core::apply_log_envelope` (#6080).
    crate::core::apply_log_envelope(inv.value, inv.logs)
}

/// Global core-level application state.
///
/// Currently holds shared metadata like the core version.
#[derive(Clone)]
pub struct AppState {
    /// The current version of the OpenHuman core binary, usually from `CARGO_PKG_VERSION`.
    pub core_version: String,
}

/// Identifies the host process that started the core runtime. Threaded into
/// `bootstrap_core_runtime` so policy-sensitive boot decisions (currently the
/// approval-gate env-override evaluation, future host-specific defaults) can
/// be made deterministically rather than guessed from the absence of other
/// signals.
///
/// Mapping:
/// - [`HostKind::TauriShell`] — the desktop app shell embedded the core as an
///   in-process tokio task. Operator-supplied env-as-config is treated as
///   advisory only; the gate ALWAYS installs.
/// - [`HostKind::Cli`] — `openhuman-core` standalone binary spawned by an
///   operator on the command line. Env-as-config is the operator's chosen
///   override surface; the gate honours it.
/// - [`HostKind::Docker`] — containerised deployment. Same env honour-rule as
///   CLI; the host shell is not the user's desktop so there is no UI surface
///   to route an approval prompt to.
/// - [`HostKind::Library`] — an in-process embedding host that supplies its own
///   provider credentials and does not participate in OpenHuman app login.
///
/// [`HostKind::detect_standalone`] picks `Docker` vs `Cli` for standalone
/// invocations using the standard Docker signals (`/.dockerenv` or
/// `OPENHUMAN_DOCKER=1`). Tauri-shell callers MUST pass `TauriShell`
/// explicitly — there is no env detection because the embedding shell is the
/// only authority on this fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    TauriShell,
    Cli,
    Docker,
    Library,
    /// A many-user server behind a trusted gateway ([`Mode::Saas`]). Treated
    /// as the most restrictive host wherever behaviour differs by host.
    ///
    /// [`Mode::Saas`]: crate::core::runtime::Mode::Saas
    Saas,
}

impl HostKind {
    /// Choose between [`HostKind::Cli`] and [`HostKind::Docker`] for a
    /// standalone-core invocation. Tauri-shell callers must NOT use this —
    /// they pass [`HostKind::TauriShell`] directly because the shell is the
    /// only authority on whether the core is embedded.
    pub fn detect_standalone() -> Self {
        if std::env::var("OPENHUMAN_DOCKER")
            .map(|v| {
                let t = v.trim();
                t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
            })
            .unwrap_or(false)
            || std::path::Path::new("/.dockerenv").exists()
        {
            HostKind::Docker
        } else {
            HostKind::Cli
        }
    }

    /// True when the host is the desktop Tauri shell. Used by the approval
    /// gate to decide whether the `OPENHUMAN_APPROVAL_GATE=0` env override
    /// is honoured (CLI / Docker) or ignored with a UI-routable warning
    /// event (Tauri shell).
    pub fn is_desktop_shell(self) -> bool {
        matches!(self, HostKind::TauriShell)
    }

    /// Short tag used in structured logs / event payloads. Stable —
    /// downstream subscribers may key on the exact string.
    pub fn tag(self) -> &'static str {
        match self {
            HostKind::TauriShell => "tauri-shell",
            HostKind::Cli => "cli",
            HostKind::Docker => "docker",
            HostKind::Library => "library",
            HostKind::Saas => "saas",
        }
    }
}

/// Pure decision helper for the approval-gate host-aware bootstrap branch.
/// Takes the host kind and whether an `OPENHUMAN_APPROVAL_GATE=0` env
/// override was observed; returns:
/// - `install_gate`: true when the gate should be installed at boot
/// - `override_ignored`: true when an env override was seen but suppressed
///   (Tauri shell with override-requested)
/// - `gate_disabled_by_override`: true when an env override was honored and
///   the gate is intentionally not installed (CLI / Docker / library)
///
/// Extracted as a pure function so the host-aware policy can be exercised
/// in isolation without standing up the full `bootstrap_core_runtime` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalGateBootDecision {
    pub install_gate: bool,
    pub override_ignored: bool,
    pub gate_disabled_by_override: bool,
}

pub fn approval_gate_boot_decision(
    host: HostKind,
    env_override_requested: bool,
) -> ApprovalGateBootDecision {
    match host {
        // A SaaS core, like the desktop shell, never lets the environment
        // switch the gate off.
        HostKind::TauriShell | HostKind::Saas => ApprovalGateBootDecision {
            install_gate: true,
            override_ignored: env_override_requested,
            gate_disabled_by_override: false,
        },
        HostKind::Cli | HostKind::Docker | HostKind::Library => ApprovalGateBootDecision {
            install_gate: !env_override_requested,
            override_ignored: false,
            gate_disabled_by_override: env_override_requested,
        },
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
