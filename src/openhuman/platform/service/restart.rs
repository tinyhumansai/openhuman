//! Core self-restart orchestration for the service domain.
//!
//! This module intentionally splits restart into two phases:
//! 1. RPC/CLI acknowledges the request and publishes an event.
//! 2. A long-lived event-bus subscriber performs the actual respawn and exit.
//!
//! Keeping the side effect behind the event bus lets JSON-RPC, CLI, and any
//! future in-process trigger share one restart path with the same logging.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::rpc::RpcOutcome;

const RESTART_DELAY_ENV: &str = "OPENHUMAN_RESTART_DELAY_MS";
const DEFAULT_RESTART_DELAY_MS: u64 = 350;

static RESTART_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// JSON-serializable acknowledgement returned to CLI / JSON-RPC callers before
/// the current process exits.
#[derive(Debug, Clone, Serialize)]
pub struct RestartStatus {
    pub accepted: bool,
    pub source: String,
    pub reason: String,
}

/// Applies a short delay to a freshly respawned process.
///
/// The replacement child starts before the old process exits so the restart can
/// be initiated from inside the running server. A small delay reduces bind-race
/// failures on the HTTP port while the old process is still releasing sockets.
pub fn apply_startup_restart_delay_from_env() {
    let Some(raw) = std::env::var(RESTART_DELAY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };

    match raw.parse::<u64>() {
        Ok(delay_ms) => {
            eprintln!(
                "[service:restart] delaying restarted process startup by {}ms",
                delay_ms
            );
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
        Err(err) => {
            eprintln!(
                "[service:restart] ignoring invalid {}='{}': {}",
                RESTART_DELAY_ENV, raw, err
            );
        }
    }
}

/// Accepts a restart request and publishes it to the global event bus.
///
/// This function does not kill or respawn the process directly; that work is
/// performed by [`crate::openhuman::platform::service::bus::RestartSubscriber`] so every
/// in-process trigger uses the same restart execution path.
pub async fn service_restart(
    source: Option<String>,
    reason: Option<String>,
) -> Result<RpcOutcome<RestartStatus>, String> {
    let source = source
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "jsonrpc".to_string());
    let reason = reason
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "service.restart".to_string());

    let _ = crate::core::bus::init().await;
    log::info!(
        "[service:restart] accepted restart request source={} reason={}",
        source,
        reason
    );
    BUS.publish(DomainEvent::SystemRestartRequested {
        source: source.clone(),
        reason: reason.clone(),
    });

    Ok(RpcOutcome::single_log(
        RestartStatus {
            accepted: true,
            source,
            reason,
        },
        "service restart requested",
    ))
}

/// Starts the replacement process for the current core instance.
///
/// The caller is responsible for exiting the current process after this returns
/// successfully.
pub fn trigger_self_restart_now(source: &str, reason: &str) -> Result<u32, String> {
    if RESTART_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err("restart already in progress".to_string());
    }

    match spawn_restart_child() {
        Ok(child_pid) => {
            log::info!(
                "[service:restart] spawned replacement process pid={} source={} reason={}",
                child_pid,
                source,
                reason
            );
            Ok(child_pid)
        }
        Err(err) => {
            RESTART_IN_PROGRESS.store(false, Ordering::SeqCst);
            Err(err)
        }
    }
}

/// Respawns the current executable with the original argument list.
///
/// This preserves the launch mode the user already chose, for example
/// `openhuman run --jsonrpc-only` or another long-lived server mode.
fn spawn_restart_child() -> Result<u32, String> {
    let current_exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Err("cannot self-restart without original launch arguments".to_string());
    }

    log::debug!(
        "[service:restart] respawning exe={} args={:?}",
        current_exe.display(),
        args
    );

    let child = Command::new(&current_exe)
        .args(&args)
        .env(RESTART_DELAY_ENV, DEFAULT_RESTART_DELAY_MS.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to spawn replacement process: {e}"))?;

    Ok(child.id())
}

#[cfg(test)]
#[path = "restart_tests.rs"]
mod tests;
