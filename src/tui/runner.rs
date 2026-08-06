//! CLI entry point for the tabbed terminal UI (`openhuman` / `tui` / `chat`).
//!
//! Parses flags, initializes **file-only** logging (the TUI owns the terminal —
//! see `logging::init_for_tui`), boots the core in-process with no transport and
//! no background services, resolves the target thread, and hands off to the
//! event loop in [`super::app`].

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::core::runtime::{
    CoreBuilder, CoreRuntime, DomainSet, ServiceSet, AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS,
};
use crate::core::types::HostKind;

/// Entry point dispatched from the `"tui" | "chat"` arm in `src/core/cli.rs`.
///
/// Flags:
///   * `--thread <id>` — attach to an existing thread.
///   * `--new` — force a brand-new thread (default when `--thread` is absent).
///   * `-v` / `--verbose` — debug-level file logging.
pub fn run_from_cli(args: &[String]) -> anyhow::Result<()> {
    let mut thread_id: Option<String> = None;
    let mut force_new = false;
    let mut verbose = false;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--thread" => {
                thread_id = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --thread"))?
                        .clone(),
                );
                i += 2;
            }
            "--new" => {
                force_new = true;
                i += 1;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown tui arg: {other}")),
        }
    }

    // File-only logging — never stderr while the TUI owns the terminal.
    let data_dir = resolve_data_dir();
    let log_dir = crate::core::logging::init_for_tui(&data_dir, verbose);
    log::info!(
        "[tui] starting tabbed terminal UI (thread={:?} new={} logs={:?})",
        thread_id,
        force_new,
        log_dir
    );

    // A chat turn is a large async state machine that can delegate to
    // sub-agents; give the tokio workers the same roomy stack the server uses
    // so a nested turn cannot overflow the default 2 MiB stack.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .build()?;
    rt.block_on(async_main(thread_id, force_new))
}

async fn async_main(thread_flag: Option<String>, force_new: bool) -> anyhow::Result<()> {
    // In-process core: full domains (channel.web_chat needs DomainGroup::Channels,
    // so harness() is not enough), no transport, no background services.
    let runtime = Arc::new(
        CoreBuilder::new(HostKind::detect_standalone())
            .domains(DomainSet::full())
            .services(ServiceSet::none())
            .build()
            .await?,
    );
    log::info!("[tui] core built (DomainSet::full, ServiceSet::none)");

    let client_id = format!("tui-{}", short_hex());
    let thread_id = resolve_thread(&runtime, thread_flag, force_new).await?;
    log::info!("[tui] resolved thread={thread_id} client_id={client_id}");

    // Subscribe BEFORE the first turn so no streamed event is missed.
    let web_rx = crate::openhuman::web_chat::subscribe_web_channel_events();

    super::app::run(runtime, client_id, thread_id, web_rx).await
}

/// Resolve the thread to open: the `--thread` id (unless `--new`), otherwise a
/// freshly created thread.
async fn resolve_thread(
    runtime: &CoreRuntime,
    thread_flag: Option<String>,
    force_new: bool,
) -> anyhow::Result<String> {
    if let (Some(id), false) = (thread_flag.as_ref(), force_new) {
        log::debug!("[tui] attaching to existing thread {id}");
        return Ok(id.clone());
    }

    let created = runtime
        .invoke("openhuman.threads_create_new", json!({}))
        .await
        .map_err(|e| anyhow::anyhow!("openhuman.threads_create_new failed: {e}"))?;
    extract_thread_id(&created).ok_or_else(|| {
        anyhow::anyhow!("openhuman.threads_create_new returned no thread id: {created}")
    })
}

/// Pull a thread id out of a `threads.create_new` / `threads.list` response,
/// tolerant of the `RpcOutcome` log-envelope wrapping (`{result, logs}`) and the
/// `ApiEnvelope` data wrapping (`{data, meta}`).
pub(super) fn extract_thread_id(value: &Value) -> Option<String> {
    // Unwrap the optional `{result, logs}` log envelope first.
    let inner = value.get("result").unwrap_or(value);
    // Then the optional `{data, meta}` ApiEnvelope.
    let payload = inner.get("data").unwrap_or(inner);
    payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Resolve the OpenHuman data dir (host of `logs/`), mirroring the shell's
/// resolution: `OPENHUMAN_WORKSPACE` override, else `~/.openhuman`, else a temp
/// fallback. No `eprintln!` — the TUI is about to take the terminal.
fn resolve_data_dir() -> PathBuf {
    if let Ok(workspace) = std::env::var("OPENHUMAN_WORKSPACE") {
        if !workspace.is_empty() {
            return PathBuf::from(workspace);
        }
    }
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("openhuman"))
}

/// 12 hex chars of randomness for the client stream id.
fn short_hex() -> String {
    let u = uuid::Uuid::new_v4();
    u.simple().to_string()[..12].to_string()
}

fn print_help() {
    println!("Usage: openhuman tui [--thread <id>] [--new] [-v|--verbose]");
    println!("       openhuman chat [--thread <id>] [--new] [-v|--verbose]");
    println!();
    println!("Open the tabbed terminal UI for core logs, orchestrator chat, configuration,");
    println!("and account settings. Runs the core in-process — no server, no ports.");
    println!();
    println!("  --thread <id>   Attach to an existing conversation thread.");
    println!("  --new           Force a new thread (default when --thread is omitted).");
    println!("  -v, --verbose   Debug-level logging (written to the log file, never the UI).");
    println!();
    println!("Keys: Tab/Shift+Tab or Alt+1-4 switch tabs · arrows navigate · Enter select ·");
    println!("      Ctrl+C / Ctrl+D quit.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_thread_id_handles_bare_summary() {
        let v = json!({ "id": "thread-1", "title": "x" });
        assert_eq!(extract_thread_id(&v).as_deref(), Some("thread-1"));
    }

    #[test]
    fn extract_thread_id_handles_api_envelope() {
        let v = json!({ "data": { "id": "thread-2" }, "meta": {} });
        assert_eq!(extract_thread_id(&v).as_deref(), Some("thread-2"));
    }

    #[test]
    fn extract_thread_id_handles_log_envelope_around_api_envelope() {
        let v = json!({
            "result": { "data": { "id": "thread-3" }, "meta": {} },
            "logs": ["created"]
        });
        assert_eq!(extract_thread_id(&v).as_deref(), Some("thread-3"));
    }

    #[test]
    fn extract_thread_id_missing_returns_none() {
        let v = json!({ "meta": {} });
        assert_eq!(extract_thread_id(&v), None);
    }

    #[test]
    fn short_hex_is_twelve_chars() {
        assert_eq!(short_hex().len(), 12);
    }

    /// Regression guard: the RPC method names the TUI invokes must be the
    /// canonical `openhuman.<namespace>_<function>` form that the registry
    /// resolves — NOT the dotted `namespace.function` short form, which
    /// `schema_for_rpc_method` does not recognise and which would make every
    /// turn (and the launch-time thread creation) fail with "unknown method".
    /// The dispatcher only rewrites a fixed set of legacy aliases; none of
    /// these three are in that table, so the short form never resolves.
    #[test]
    fn tui_invokes_use_canonical_registered_rpc_method_names() {
        for method in [
            "openhuman.channel_web_chat",
            "openhuman.channel_web_cancel",
            "openhuman.threads_create_new",
            "openhuman.config_get_client_config",
            "openhuman.config_update_model_settings",
            "openhuman.config_get_autonomy_settings",
            "openhuman.config_update_autonomy_settings",
            "openhuman.config_get_privacy_mode",
            "openhuman.config_set_privacy_mode",
            "openhuman.auth_get_state",
            "openhuman.auth_get_me",
            "openhuman.auth_consume_login_token",
            "openhuman.auth_store_session",
            "openhuman.auth_clear_session",
        ] {
            assert!(
                crate::core::all::schema_for_rpc_method(method).is_some(),
                "TUI invokes `{method}`, but it is not a registered RPC method — \
                 the tabbed terminal UI would fail with `unknown method: {method}`"
            );
        }
    }
}
