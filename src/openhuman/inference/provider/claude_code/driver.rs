//! Spawn the `claude` CLI for one chat turn, stream its stdout into the
//! event mapper, and return an aggregated `ChatResponse`.
//!
//! The driver does *not* own concurrency limits; the `ClaudeCodeProvider`
//! holds a `Semaphore` and acquires a permit before calling this.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Hard timeout per turn (PLAN §8). If the CLI hangs (network stall,
/// infinite loop, MCP deadlock) we kill the child and surface a timeout.
const TURN_TIMEOUT: Duration = Duration::from_secs(300);

use super::event_mapper::EventMapper;
use super::input_builder::build_stdin;
use super::session_store::{generate_uuid_v4, is_uuid_v4, SessionStore};
use super::stream_parser::StreamJsonParser;
use crate::openhuman::inference::provider::traits::{ChatMessage, ChatResponse, ProviderDelta};

/// Tools withheld in the DEFAULT (`acceptEdits`) posture: Claude Code can
/// read/edit files in the project, but not run shell, hit the network, or
/// fan out CC subagents. The user opts into the full toolset separately by
/// enabling full access (see [`claude_code_full_access`]), which switches to
/// `bypassPermissions` and drops this list.
const DISALLOWED_CC_BUILTINS: &[&str] = &[
    "Bash",
    "BashOutput",
    "KillShell",
    "WebFetch",
    "WebSearch",
    "Task",
];

/// Whether the user opted into FULL access for Claude Code (`bypassPermissions`
/// + full native toolset incl. Bash/network). Default is **off** → the safer
/// `acceptEdits` posture (file edits only). This is a deliberate user choice,
/// not the default — enabling Claude Code alone does not grant shell/network
/// power.
///
/// Resolution order:
/// 1. `OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE` env var, when set to a recognised
///    value, wins (debugging / power users). `bypass`/`bypassPermissions`/`full`
///    force ON; `acceptEdits`/`edits`/`default`/`off`/`false`/`0` force OFF.
/// 2. Otherwise the persisted UI toggle in
///    [`super::settings`] (the Claude Code modal "Full access" switch).
fn claude_code_full_access(workspace_dir: &std::path::Path) -> bool {
    if let Ok(raw) = std::env::var("OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE") {
        match raw.trim() {
            "bypass" | "bypassPermissions" | "full" => return true,
            "acceptEdits" | "edits" | "default" | "off" | "false" | "0" => return false,
            _ => {}
        }
    }
    super::settings::load(workspace_dir).full_access
}

/// Whether to wrap the `claude` spawn in the macOS Seatbelt jail. On by
/// default on macOS where `sandbox-exec` exists; opt out with
/// `OPENHUMAN_CLAUDE_CODE_SANDBOX=0`.
fn seatbelt_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        let opted_out = std::env::var("OPENHUMAN_CLAUDE_CODE_SANDBOX")
            .map(|v| v == "0")
            .unwrap_or(false);
        !opted_out && std::path::Path::new("/usr/bin/sandbox-exec").exists()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Render a Seatbelt profile that lets Claude Code do **everything** the user
/// can — read/write anywhere, run subprocesses, use the network — EXCEPT touch
/// OpenHuman's internal workspace (`~/.openhuman*`: memory DB, sessions, auth
/// tokens, config). That is the one hard wall: CC's raw tools must not be able
/// to corrupt OpenHuman's own state. This mirrors OpenHuman's existing
/// `is_workspace_internal_path` invariant (its native tools already can't write
/// there) and now enforces the same boundary for the CC subprocess at the OS
/// level. Everything else is the user's call.
///
/// Denies BOTH reads and writes of the OpenHuman workspace: CC's raw tools
/// can neither corrupt nor exfiltrate OpenHuman's internal state (memory DB,
/// sessions, auth tokens, config). CC still reaches OpenHuman memory — but only
/// through the MCP HTTP server, which runs in the unjailed core (not as CC's
/// child), so this full deny is safe.
#[cfg(target_os = "macos")]
fn seatbelt_profile(workspace_dir: &std::path::Path) -> String {
    let esc = |p: String| p.replace('\\', "\\\\").replace('"', "\\\"");
    // Deny the ENTIRE `~/.openhuman[-staging]` tree, not just the per-user
    // workspace subdir. `workspace_dir` is `…/.openhuman-staging/users/<id>/…`,
    // but sensitive files (core.token, credentials) also live at the root — so
    // denying only the subdir leaves them readable. Walk up to the `.openhuman*`
    // ancestor and deny that whole tree.
    let root = openhuman_internal_root(workspace_dir);
    let root = esc(std::fs::canonicalize(&root)
        .unwrap_or(root)
        .to_string_lossy()
        .to_string());
    format!(
        "(version 1)\n(allow default)\n\
         (deny file-write*\n  (subpath \"{root}\")\n)\n\
         (deny file-read*\n  (subpath \"{root}\")\n)\n"
    )
}

/// Resolve the OpenHuman internal root (`~/.openhuman` / `~/.openhuman-staging`)
/// from a path inside it by walking up to the first `.openhuman*` ancestor.
/// Falls back to the input path when no such ancestor exists.
#[cfg(target_os = "macos")]
fn openhuman_internal_root(workspace_dir: &std::path::Path) -> std::path::PathBuf {
    let mut cur = workspace_dir;
    loop {
        if cur
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with(".openhuman"))
            .unwrap_or(false)
        {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => return workspace_dir.to_path_buf(),
        }
    }
}

/// One CC chat turn.
pub struct TurnContext<'a> {
    pub bin_path: PathBuf,
    pub workspace_dir: PathBuf,
    /// The user's project root (`config.action_dir`). Claude Code runs here
    /// (cwd + `--add-dir`) so its file tools act on the user's code, not the
    /// internal OpenHuman workspace.
    pub project_dir: PathBuf,
    pub thread_id: String,
    pub model: String,
    pub append_system_prompt: Option<String>,
    pub messages: &'a [ChatMessage],
    pub session_store: Arc<SessionStore>,
    pub stream: Option<&'a mpsc::Sender<ProviderDelta>>,
    /// Optional explicit `ANTHROPIC_API_KEY` to set on the child. When
    /// `None`, the CLI falls back to its own `~/.claude/.credentials.json`.
    pub anthropic_api_key: Option<String>,
}

/// Write a CC `--mcp-config` JSON pointing at OpenHuman's in-process HTTP MCP
/// server (running in the unjailed core). CC connects over loopback, so the
/// MCP server is NOT a child of the sandboxed `claude` and keeps full access
/// to `~/.openhuman` for memory — while CC's own raw tools are denied that dir
/// by the jail. Returns the on-disk path; caller cleans up.
fn write_mcp_http_config(
    dir: &std::path::Path,
    addr: std::net::SocketAddr,
    token: &str,
) -> std::io::Result<PathBuf> {
    let path = dir.join("openhuman-mcp-config.json");
    // The loopback MCP server is authenticated — carry the per-process bearer
    // token so only this `claude` launch (not other local processes) can reach
    // OpenHuman's tools/memory.
    let cfg = json!({
        "mcpServers": {
            "openhuman": {
                "type": "http",
                "url": format!("http://{addr}/"),
                "headers": {
                    "Authorization": format!("Bearer {token}"),
                }
            }
        }
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&cfg).unwrap_or_default(),
    )?;
    Ok(path)
}

/// Run one turn against the `claude` CLI. Awaits process exit. Forwards
/// `ProviderDelta`s through `ctx.stream` as they arrive and returns the
/// aggregated `ChatResponse` when done.
pub async fn run_turn(ctx: TurnContext<'_>) -> anyhow::Result<ChatResponse> {
    let stored = ctx.session_store.get(&ctx.thread_id);
    let is_new = !stored.as_deref().map(is_uuid_v4).unwrap_or(false);
    let cc_session_id = if is_new {
        let id = generate_uuid_v4();
        if let Err(e) = ctx.session_store.set(&ctx.thread_id, &id) {
            log::warn!(
                "[claude-code][driver] failed to persist session uuid for thread {}: {}",
                ctx.thread_id,
                e
            );
        }
        id
    } else {
        stored.expect("checked Some above")
    };

    // Set up a per-turn scratch dir for --mcp-config and any other transient
    // state. Best-effort cleanup at end of turn.
    let scratch = tempfile::Builder::new()
        .prefix("openhuman-cc-")
        .tempdir()
        .map_err(|e| anyhow::anyhow!("create scratch dir: {e}"))?;
    // Point CC at OpenHuman's in-process HTTP MCP server (unjailed core), so
    // the memory bridge survives CC's `.openhuman` jail deny.
    let mut mcp_config_path: Option<PathBuf> = None;
    match crate::openhuman::mcp_server::ensure_local_http().await {
        Ok(endpoint) => match write_mcp_http_config(scratch.path(), endpoint.addr, &endpoint.token) {
            Ok(p) => {
                log::debug!(
                    "[claude-code][driver] wrote http mcp-config path={} url=http://{}/ (authenticated)",
                    p.display(),
                    endpoint.addr
                );
                mcp_config_path = Some(p);
            }
            Err(e) => log::warn!(
                "[claude-code][driver] failed to write mcp-config: {e}; CC will run without OpenHuman MCP tools"
            ),
        },
        Err(e) => log::warn!(
            "[claude-code][driver] in-process MCP HTTP server unavailable: {e}; CC running without OpenHuman MCP tools"
        ),
    }

    // The user explicitly opts into Claude Code, so we do NOT limit its toolset
    // on any platform — CC always gets its full tools + `bypassPermissions`.
    // The jail (macOS Seatbelt, below) is purely the `.openhuman` wall: it
    // doesn't restrict CC, it just protects OpenHuman's internal data where the
    // OS supports it. On Linux/Windows there's no OS wall yet, so CC runs
    // unconfined there (user's machine, user's call).
    let jailed = seatbelt_available();
    // `jailed` is only consumed by the macOS Seatbelt spawn-wrap below.
    #[cfg(not(target_os = "macos"))]
    let _ = jailed;

    // Permission posture is a USER choice. Default `acceptEdits` (file edits
    // only); the user opts into `bypassPermissions` (full toolset incl. bash)
    // explicitly. On macOS the Seatbelt jail walls off `~/.openhuman` in either
    // mode; on Linux/Windows full access is unconfined.
    let full_access = claude_code_full_access(&ctx.workspace_dir);
    let permission_mode = if full_access {
        "bypassPermissions"
    } else {
        "acceptEdits"
    };

    let mut args: Vec<String> = vec![
        "-p".into(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        // Grant file-tool access to the user's project root (cwd is set to the
        // same dir below). This is where the coding agent reads/edits code.
        "--add-dir".into(),
        ctx.project_dir.display().to_string(),
        // Default `acceptEdits` (auto-apply edits, gate the rest); the user can
        // opt into `bypassPermissions` for the full toolset (see above).
        "--permission-mode".into(),
        permission_mode.to_string(),
        if is_new {
            "--session-id".into()
        } else {
            "--resume".into()
        },
        cc_session_id.clone(),
        "--model".into(),
        ctx.model.clone(),
    ];
    if let Some(sp) = ctx
        .append_system_prompt
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        args.push("--append-system-prompt".into());
        args.push(sp.clone());
    }
    if let Some(p) = mcp_config_path.as_ref() {
        args.push("--mcp-config".into());
        args.push(p.display().to_string());
        args.push("--strict-mcp-config".into());
    }
    // Tool surface follows the permission posture: full access → no
    // `--disallowedTools` (CC keeps its entire toolset incl. Bash/network);
    // default `acceptEdits` → withhold the dangerous builtins (edits only).
    if !full_access {
        args.push("--disallowedTools".into());
        args.push(DISALLOWED_CC_BUILTINS.join(","));
    }

    // Validate input *before* spawning so we don't launch a process we
    // can't feed (CodeRabbit: validate before spawn).
    let stdin_bytes = build_stdin(ctx.messages, is_new);
    if stdin_bytes.is_empty() {
        anyhow::bail!("[claude-code][driver] no input messages to deliver");
    }

    log::debug!(
        "[claude-code][driver] spawn bin={} model={} is_new={} cc_session_id={}",
        ctx.bin_path.display(),
        ctx.model,
        is_new,
        cc_session_id
    );

    // Best-effort: ensure the project dir exists so spawn (cwd) doesn't fail.
    std::fs::create_dir_all(&ctx.project_dir).ok();

    // Wrap the spawn in the macOS Seatbelt jail when available so CC's file
    // writes are OS-confined: `sandbox-exec -p <profile> <claude> <args…>`.
    #[cfg(target_os = "macos")]
    let (program, final_args): (PathBuf, Vec<String>) = if jailed {
        let profile = seatbelt_profile(&ctx.workspace_dir);
        let mut wrapped = vec![
            "-p".to_string(),
            profile,
            ctx.bin_path.display().to_string(),
        ];
        wrapped.extend(args.iter().cloned());
        log::debug!(
            "[claude-code][driver] seatbelt jail active root={}",
            ctx.project_dir.display()
        );
        (PathBuf::from("/usr/bin/sandbox-exec"), wrapped)
    } else {
        (ctx.bin_path.clone(), args.clone())
    };
    #[cfg(not(target_os = "macos"))]
    let (program, final_args): (PathBuf, Vec<String>) = (ctx.bin_path.clone(), args.clone());

    let mut cmd = Command::new(&program);
    cmd.args(&final_args)
        .current_dir(&ctx.project_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(key) = &ctx.anthropic_api_key {
        cmd.env("ANTHROPIC_API_KEY", key);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn `claude`: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&stdin_bytes)
            .await
            .map_err(|e| anyhow::anyhow!("write stdin: {e}"))?;
        stdin
            .shutdown()
            .await
            .map_err(|e| anyhow::anyhow!("close stdin: {e}"))?;
    }

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("claude child stdout missing"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("claude child stderr missing"))?;

    let mut parser = StreamJsonParser::new();
    let mut mapper = EventMapper::new();
    let mut buf = [0u8; 8192];

    // Drain stderr in parallel into a buffer for diagnostics.
    let stderr_task = tokio::spawn(async move {
        let mut acc = String::new();
        let mut tmp = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut tmp).await {
            if n == 0 {
                break;
            }
            acc.push_str(&String::from_utf8_lossy(&tmp[..n]));
            if acc.len() > 16_384 {
                acc.truncate(16_384);
            }
        }
        acc
    });

    // Wrap the streaming + wait in a timeout so a stuck CLI doesn't
    // block this task forever (PLAN §8).
    let timed = tokio::time::timeout(TURN_TIMEOUT, async {
        loop {
            let n = stdout
                .read(&mut buf)
                .await
                .map_err(|e| anyhow::anyhow!("read stdout: {e}"))?;
            if n == 0 {
                break;
            }
            for ev in parser.feed_bytes(&buf[..n]) {
                for delta in mapper.handle(ev) {
                    if let Some(tx) = ctx.stream {
                        let _ = tx.send(delta).await;
                    }
                }
            }
        }
        for ev in parser.end() {
            for delta in mapper.handle(ev) {
                if let Some(tx) = ctx.stream {
                    let _ = tx.send(delta).await;
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| anyhow::anyhow!("wait child: {e}"))?;
        Ok::<_, anyhow::Error>(status)
    })
    .await;

    let status = match timed {
        Ok(inner) => inner?,
        Err(_elapsed) => {
            log::error!(
                "[claude-code][driver] turn timeout ({TURN_TIMEOUT:?}) exceeded; killing child"
            );
            // kill_on_drop handles cleanup, but explicit kill gives us
            // a chance to collect stderr.
            let _ = child.kill().await;
            anyhow::bail!(
                "[claude-code][driver] turn timed out after {:?}",
                TURN_TIMEOUT
            );
        }
    };

    let stderr_text = stderr_task.await.unwrap_or_default();

    if !status.success() {
        anyhow::bail!(
            "[claude-code][driver] exit {:?} stderr={}",
            status.code(),
            stderr_text.trim()
        );
    }
    if let Some(err) = mapper.error.clone() {
        anyhow::bail!("[claude-code][driver] {}", err);
    }

    Ok(mapper.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_mcp_http_config_emits_http_url_with_bearer_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let addr: std::net::SocketAddr = "127.0.0.1:54321".parse().unwrap();
        let path = write_mcp_http_config(dir.path(), addr, "tok-abc123").expect("write config");
        let raw = std::fs::read_to_string(&path).expect("read config");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        let server = &v["mcpServers"]["openhuman"];
        assert_eq!(
            server["type"], "http",
            "MCP transport must be http (out-of-jail)"
        );
        assert_eq!(server["url"], "http://127.0.0.1:54321/");
        // The loopback server is authenticated — the config must carry the bearer.
        assert_eq!(server["headers"]["Authorization"], "Bearer tok-abc123");
        // It must NOT spawn a stdio child (the old jailed path).
        assert!(server.get("command").is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_profile_denies_whole_openhuman_root_not_just_subdir() {
        // Driver passes the per-user subdir; the jail must deny the WHOLE
        // `.openhuman-staging` tree (so root-level core.token/credentials are
        // protected), not just the subdir.
        let ws = std::path::Path::new("/Users/test/.openhuman-staging/users/abc/workspace");
        let p = seatbelt_profile(ws);
        assert!(
            p.contains("(allow default)"),
            "CC does everything by default"
        );
        assert!(p.contains("(deny file-write*"), "must deny writes");
        assert!(
            p.contains("(deny file-read*"),
            "must deny reads (no token exfil)"
        );
        // Denied path is the ROOT, not the per-user subdir.
        assert!(
            p.contains("/Users/test/.openhuman-staging\""),
            "deny subpath must be the .openhuman root: {p}"
        );
        assert!(
            !p.contains("users/abc"),
            "deny must NOT be scoped to the narrow subdir: {p}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn openhuman_internal_root_walks_up_to_dotopenhuman() {
        let r = openhuman_internal_root(std::path::Path::new(
            "/Users/x/.openhuman/users/id/workspace/memory",
        ));
        assert_eq!(r, std::path::Path::new("/Users/x/.openhuman"));
        // Fallback: no `.openhuman*` ancestor → returns the input.
        let r2 = openhuman_internal_root(std::path::Path::new("/tmp/custom/ws"));
        assert_eq!(r2, std::path::Path::new("/tmp/custom/ws"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_available_honors_opt_out() {
        let _env = super::super::ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("OPENHUMAN_CLAUDE_CODE_SANDBOX").ok();
        std::env::set_var("OPENHUMAN_CLAUDE_CODE_SANDBOX", "0");
        assert!(
            !seatbelt_available(),
            "explicit opt-out must disable the jail"
        );
        match prev {
            Some(v) => std::env::set_var("OPENHUMAN_CLAUDE_CODE_SANDBOX", v),
            None => std::env::remove_var("OPENHUMAN_CLAUDE_CODE_SANDBOX"),
        }
    }

    #[test]
    fn full_access_defaults_off_and_opts_in_via_env() {
        let _env = super::super::ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Empty workspace (no persisted toggle) → file layer resolves to OFF.
        let ws = std::env::temp_dir().join("oh_cc_fullaccess_env_test");
        let _ = std::fs::remove_dir_all(&ws);
        let key = "OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE";
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        assert!(
            !claude_code_full_access(&ws),
            "default posture must be acceptEdits (full access OFF)"
        );
        std::env::set_var(key, "bypass");
        assert!(
            claude_code_full_access(&ws),
            "explicit opt-in (`bypass`) enables full access"
        );
        std::env::set_var(key, "acceptEdits");
        assert!(
            !claude_code_full_access(&ws),
            "acceptEdits env override keeps the default (limited) posture"
        );
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn full_access_reads_persisted_toggle_when_env_unset() {
        use super::super::settings::{self, ClaudeCodeSettings};
        let _env = super::super::ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let ws = std::env::temp_dir().join("oh_cc_fullaccess_file_test");
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        let key = "OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE";
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);

        settings::save(&ws, &ClaudeCodeSettings { full_access: true }).unwrap();
        assert!(
            claude_code_full_access(&ws),
            "persisted toggle ON must enable full access when env is unset"
        );

        // Env override beats the persisted toggle.
        std::env::set_var(key, "acceptEdits");
        assert!(
            !claude_code_full_access(&ws),
            "env override OFF must beat a persisted ON toggle"
        );

        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        let _ = std::fs::remove_dir_all(&ws);
    }
}
