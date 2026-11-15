//! Listener port selection for the core's HTTP server.
//!
//! [`serve`](super::serve) binds through [`pick_listen_port_for_host_with`]:
//! try the preferred port, retry transient `AddrInUse` races, ask for the
//! stale-listener takeover (#1130) when another OpenHuman core holds it, and
//! otherwise move to a fallback port. The desktop shell matches
//! [`PickListenPortError::WouldTakeOver`] on the server task's error to run
//! that takeover.

use std::fmt;
use std::io::ErrorKind;
use tokio::net::TcpListener;
use tokio::time::{sleep, Duration};
use tracing::{debug, warn};

const DEFAULT_CORE_PORT: u16 = 7788;
const DEFAULT_FALLBACK_START: u16 = 7789;
const DEFAULT_FALLBACK_END: u16 = 7798;

/// Successful bind selection for the embedded core HTTP listener.
#[derive(Debug)]
pub struct PickListenPortResult {
    pub listener: TcpListener,
    pub port: u16,
    /// Present when the preferred port was occupied and we moved to another
    /// port in the fallback pool.
    pub fallback_from: Option<u16>,
}

#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    attempts: usize,
    backoff: Duration,
}

impl RetryPolicy {
    const DEFAULT: Self = Self {
        attempts: 3,
        backoff: Duration::from_millis(500),
    };
}

#[derive(Debug, Clone)]
enum ListenerFingerprint {
    OpenHumanCore,
    Other(String),
}

impl ListenerFingerprint {
    fn as_human_readable(&self) -> String {
        match self {
            Self::OpenHumanCore => "openhuman-core".to_string(),
            Self::Other(reason) => reason.clone(),
        }
    }
}

/// Failure modes for preferred-port selection.
#[derive(Debug, Clone)]
pub enum PickListenPortError {
    /// Port is occupied by another OpenHuman core; caller should run the stale
    /// listener takeover flow (#1130) before retrying startup.
    WouldTakeOver { preferred: u16, fingerprint: String },
    /// No candidate port was available after trying the fallback pool.
    NoAvailablePort {
        preferred: u16,
        fingerprint: String,
        attempted: Vec<u16>,
    },
    /// Bind failed with a non-AddrInUse error.
    BindFailed { port: u16, reason: String },
}

impl fmt::Display for PickListenPortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldTakeOver {
                preferred,
                fingerprint,
            } => write!(
                f,
                "preferred core port {preferred} is occupied by {fingerprint}; stale-listener takeover required"
            ),
            Self::NoAvailablePort {
                preferred,
                fingerprint,
                attempted,
            } => write!(
                f,
                "preferred core port {preferred} is occupied by {fingerprint}; no fallback ports available in {:?}",
                attempted
            ),
            Self::BindFailed { port, reason } => {
                write!(f, "failed to bind core listener on port {port}: {reason}")
            }
        }
    }
}

impl std::error::Error for PickListenPortError {}

/// Pick a listen port for the embedded core listener on loopback.
///
/// Behavior:
/// - first tries `preferred`
/// - retries transient `AddrInUse` races a few times
/// - if still occupied by another OpenHuman core, asks caller to takeover
/// - otherwise falls back to ports 7789..=7798
pub async fn pick_listen_port(preferred: u16) -> Result<PickListenPortResult, PickListenPortError> {
    pick_listen_port_for_host("127.0.0.1", preferred).await
}

/// What to do when the preferred port is held by another live OpenHuman core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupiedByCore {
    /// Return [`PickListenPortError::WouldTakeOver`] so the caller can run the
    /// stale-listener takeover (#1130). The desktop shell's embedded core: a
    /// second copy of the app's own core on its port is a leftover to replace.
    Takeover,
    /// Treat it like any other occupant and move to a fallback port. Headless
    /// `openhuman-core serve`: another checkout's or the desktop app's core is
    /// a neighbour to coexist with, not something to replace.
    Fallback,
}

/// Same as [`pick_listen_port`] but allows an explicit host (used by the core
/// server bootstrap so CLI host overrides remain respected).
pub async fn pick_listen_port_for_host(
    host: &str,
    preferred: u16,
) -> Result<PickListenPortResult, PickListenPortError> {
    pick_listen_port_for_host_with(host, preferred, OccupiedByCore::Takeover).await
}

/// [`pick_listen_port_for_host`] with an explicit policy for a port that
/// another OpenHuman core already holds.
pub async fn pick_listen_port_for_host_with(
    host: &str,
    preferred: u16,
    occupied_by_core: OccupiedByCore,
) -> Result<PickListenPortResult, PickListenPortError> {
    let fallbacks: Vec<u16> = if preferred == DEFAULT_CORE_PORT {
        (DEFAULT_FALLBACK_START..=DEFAULT_FALLBACK_END).collect()
    } else {
        (1..=10)
            .filter_map(|delta| preferred.checked_add(delta))
            .collect()
    };
    pick_listen_port_with_policy(
        host,
        preferred,
        &fallbacks,
        RetryPolicy::DEFAULT,
        occupied_by_core,
    )
    .await
}

async fn pick_listen_port_with_policy(
    host: &str,
    preferred: u16,
    fallback_ports: &[u16],
    retry_policy: RetryPolicy,
    occupied_by_core: OccupiedByCore,
) -> Result<PickListenPortResult, PickListenPortError> {
    // `None`  → preferred port is occupied (AddrInUse): probe for a stale
    //           OpenHuman listener to take over before falling back.
    // `Some`  → preferred port is OS-excluded (Windows WSAEACCES / os error
    //           10013): nothing is listening, so skip the takeover probe and
    //           go straight to the fallback ports. The string is the bind
    //           error rendered for the warn / NoAvailablePort surfaces.
    let excluded_reason: Option<String> = match TcpListener::bind((host, preferred)).await {
        Ok(listener) => {
            return Ok(PickListenPortResult {
                listener,
                port: preferred,
                fallback_from: None,
            });
        }
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            // Retry transient bind races before we decide whether this needs
            // stale-listener takeover (#1130) or a fallback port.
            for _ in 0..retry_policy.attempts {
                sleep(retry_policy.backoff).await;
                match TcpListener::bind((host, preferred)).await {
                    Ok(listener) => {
                        return Ok(PickListenPortResult {
                            listener,
                            port: preferred,
                            fallback_from: None,
                        });
                    }
                    Err(retry_err) if retry_err.kind() == ErrorKind::AddrInUse => {}
                    Err(retry_err) if is_port_excluded_bind_error(&retry_err) => {
                        // Raced from in-use into an OS exclusion — treat as
                        // excluded and skip straight to fallbacks.
                        return pick_fallback_port(
                            host,
                            preferred,
                            fallback_ports,
                            retry_policy,
                            format!("port excluded by OS ({retry_err})"),
                        )
                        .await;
                    }
                    Err(retry_err) => {
                        return Err(PickListenPortError::BindFailed {
                            port: preferred,
                            reason: retry_err.to_string(),
                        });
                    }
                }
            }
            None
        }
        // Sentry OPENHUMAN-TAURI-500 (Windows): WSAEACCES / os error 10013 —
        // the preferred port sits inside a system-reserved/excluded range
        // (Hyper-V / WinNAT / WSL2 / Docker). Nothing is listening, so there
        // is no takeover to do, but a neighbour port outside the reserved
        // block typically binds. Previously this fell into the catch-all arm
        // below and gave up immediately with `BindFailed`, leaving the core
        // unable to start. Route it to the fallback ports instead.
        Err(err) if is_port_excluded_bind_error(&err) => {
            Some(format!("port excluded by OS ({err})"))
        }
        Err(err) => {
            return Err(PickListenPortError::BindFailed {
                port: preferred,
                reason: err.to_string(),
            });
        }
    };

    // Stale-listener takeover only applies when something is actually
    // listening (AddrInUse). An OS-excluded port has no listener to identify,
    // so skip the probe and synthesize a human-readable reason instead.
    let fingerprint_label = match excluded_reason {
        None => {
            let fingerprint = identify_listener(host, preferred).await;
            if matches!(fingerprint, ListenerFingerprint::OpenHumanCore)
                && occupied_by_core == OccupiedByCore::Takeover
            {
                return Err(PickListenPortError::WouldTakeOver {
                    preferred,
                    fingerprint: fingerprint.as_human_readable(),
                });
            }
            fingerprint.as_human_readable()
        }
        Some(reason) => reason,
    };

    pick_fallback_port(
        host,
        preferred,
        fallback_ports,
        retry_policy,
        fingerprint_label,
    )
    .await
}

/// Try each fallback port in turn, retrying transient `AddrInUse` races on
/// each candidate. `unusable_label` describes why `preferred` was rejected
/// (stale-listener fingerprint, or an OS port-exclusion reason) and is used
/// only for the warn / `NoAvailablePort` diagnostic surfaces.
async fn pick_fallback_port(
    host: &str,
    preferred: u16,
    fallback_ports: &[u16],
    retry_policy: RetryPolicy,
    unusable_label: String,
) -> Result<PickListenPortResult, PickListenPortError> {
    for fallback in fallback_ports {
        // Retry each fallback candidate on transient AddrInUse so a brief
        // race on 7789–7798 (AV scanner / prior-instance teardown) doesn't
        // surface as NoAvailablePort. Mirrors the preferred-port retry above.
        let mut bound: Option<TcpListener> = None;
        for attempt in 0..=retry_policy.attempts {
            match TcpListener::bind((host, *fallback)).await {
                Ok(listener) => {
                    bound = Some(listener);
                    break;
                }
                Err(err) if err.kind() == ErrorKind::AddrInUse => {
                    if attempt < retry_policy.attempts {
                        sleep(retry_policy.backoff).await;
                        continue;
                    }
                }
                Err(err) => {
                    debug!(
                        "[connectivity][rpc] fallback bind failed on {}:{}: {}",
                        host, fallback, err
                    );
                    break;
                }
            }
        }
        if let Some(listener) = bound {
            warn!(
                "[CORE] preferred port {} unusable ({}); bound to {}",
                preferred, unusable_label, fallback
            );
            return Ok(PickListenPortResult {
                listener,
                port: *fallback,
                fallback_from: Some(preferred),
            });
        }
    }

    // When an OS-exclusion blocked the preferred port *and* every fallback is
    // also unavailable, surface the Windows diagnostic command so users can
    // identify the reserved range without waiting for a support escalation.
    if unusable_label.contains("excluded by OS") {
        warn!(
            "[CORE] preferred port {} and all fallbacks {:?} are unavailable. \
             On Windows, run `netsh interface ipv4 show excludedportrange protocol=tcp` \
             to inspect system-reserved port ranges (Hyper-V / WinNAT / WSL2 / Docker).",
            preferred, fallback_ports
        );
    }

    Err(PickListenPortError::NoAvailablePort {
        preferred,
        fingerprint: unusable_label,
        attempted: fallback_ports.to_vec(),
    })
}

/// Returns `true` when a preferred-port bind failure means *that specific
/// port* is unusable but a different port likely works — so the caller should
/// try the fallback ports rather than give up.
///
/// Targets Windows `WSAEACCES` (os error 10013): the port sits inside a
/// system-reserved/excluded range (Hyper-V / WinNAT / WSL2 / Docker — visible
/// via `netsh interface ipv4 show excludedportrange protocol=tcp`). Nothing is
/// listening on it, so there is no takeover to perform, but a neighbour port
/// outside the reserved block binds fine.
///
/// We match on `raw_os_error()` directly because Rust's `ErrorKind` mapping
/// for `10013` is not stable across releases (mirrors the raw-code approach in
/// `openhuman_core::util::is_transient_fs_error`); the `PermissionDenied`
/// kind is accepted too in case a future Rust maps it.
fn is_port_excluded_bind_error(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(10013) || err.kind() == ErrorKind::PermissionDenied
}

async fn identify_listener(host: &str, port: u16) -> ListenerFingerprint {
    let probe_host = if host == "0.0.0.0" || host == "::" {
        "127.0.0.1"
    } else {
        host
    };
    // IPv6 literals must be bracketed in the URL authority per RFC 3986; an
    // un-bracketed `http://::1:7788/` parses the colons as host:port and
    // mis-classifies live OpenHuman cores on IPv6 hosts as `Other`.
    let authority = if probe_host.contains(':') && !probe_host.starts_with('[') {
        format!("[{probe_host}]")
    } else {
        probe_host.to_string()
    };
    let url = format!("http://{authority}:{port}/");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(750))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            return ListenerFingerprint::Other(format!("probe client build failed: {err}"));
        }
    };

    let response = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return ListenerFingerprint::Other(format!("probe GET / failed: {err}"));
        }
    };

    if !response.status().is_success() {
        return ListenerFingerprint::Other(format!(
            "probe GET / returned status {}",
            response.status()
        ));
    }

    let body = match response.text().await {
        Ok(text) => text,
        Err(err) => {
            return ListenerFingerprint::Other(format!("probe body read failed: {err}"));
        }
    };

    fingerprint_root_body(&body)
}

fn fingerprint_root_body(body: &str) -> ListenerFingerprint {
    if is_openhuman_root_body(body) {
        ListenerFingerprint::OpenHumanCore
    } else {
        ListenerFingerprint::Other("probe body did not identify as openhuman".to_string())
    }
}

fn is_openhuman_root_body(body: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return false,
    };
    value
        .get("name")
        .and_then(|v| v.as_str())
        .map(|name| name == "openhuman")
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "listen_port_tests.rs"]
mod tests;
