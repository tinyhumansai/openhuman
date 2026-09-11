//! Running one hook.
//!
//! ## The contract
//!
//! A command hook receives the event JSON on stdin, and answers on stdout. Its
//! exit code decides how the answer is read:
//!
//! | Exit | Meaning |
//! | ---- | ------- |
//! | `0`  | Success. stdout is parsed as a [`HookOutput`]; empty stdout is a no-op. |
//! | `2`  | Deny, regardless of stdout. stderr becomes the reason the agent sees. |
//! | else | Failure. Fails open — unless the definition sets `fail_closed`. |
//!
//! A timeout, a missing interpreter, and unparseable stdout are all *failures*
//! and take the same path as a non-zero exit: open by default, closed on
//! request. That symmetry matters — a hook that denies only when it manages to
//! run is not a security control, so `fail_closed` has to cover every way the
//! script can fail to answer, not just the tidy one.
//!
//! ## Why stdout is parsed leniently
//!
//! Scripts print. A hook that echoes a progress line and *then* prints its JSON
//! is the common case, not an error, so the parser takes the last complete JSON
//! object on stdout rather than demanding the whole stream be JSON.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;

use super::config::{HookDefinition, HookKind};
use super::types::{HookInput, HookOutput, HookPermission};

/// Exit code a hook uses to refuse an action.
pub const EXIT_DENY: i32 = 2;

/// Timeout applied when neither the definition nor the engine names one.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// What running one hook produced, with enough detail for diagnostics.
#[derive(Debug, Clone)]
pub struct HookRun {
    /// The hook's own label, for logs.
    pub label: String,
    /// The decision, after exit-code and fail-closed handling.
    pub output: HookOutput,
    /// Wall-clock runtime.
    pub duration: Duration,
    /// Set when the hook did not answer cleanly. Present even when the engine
    /// failed open, so "nothing happened" and "it broke and we continued" stay
    /// distinguishable in the logs and in the RPC test endpoint.
    pub error: Option<String>,
}

impl HookRun {
    fn noop(label: String, duration: Duration, error: Option<String>) -> Self {
        Self {
            label,
            output: HookOutput::default(),
            duration,
            error,
        }
    }
}

/// Run one hook against one event.
///
/// `env` carries session-scoped variables contributed by an earlier
/// `sessionStart` hook, plus the ambient `OPENHUMAN_*` variables the engine
/// adds. Never returns `Err`: a hook that cannot run is a [`HookRun`] with an
/// `error` and either an empty output (fail-open) or a denial (fail-closed).
pub async fn run(
    definition: &HookDefinition,
    input: &HookInput,
    env: &BTreeMap<String, String>,
    default_timeout: Duration,
) -> HookRun {
    let label = definition.label();
    let started = Instant::now();
    match definition.kind {
        HookKind::Command => {
            let timeout = definition
                .timeout
                .map(Duration::from_secs)
                .unwrap_or(default_timeout);
            let result = run_command(definition, input, env, timeout).await;
            finish(definition, label, started.elapsed(), result)
        }
        HookKind::Prompt => {
            let result = run_prompt(definition, input).await;
            finish(definition, label, started.elapsed(), result)
        }
    }
}

/// Apply the fail-open / fail-closed policy to a raw outcome.
fn finish(
    definition: &HookDefinition,
    label: String,
    duration: Duration,
    result: Result<HookOutput, String>,
) -> HookRun {
    match result {
        Ok(output) => HookRun {
            label,
            output,
            duration,
            error: None,
        },
        Err(error) if definition.fail_closed => {
            log::warn!("[hooks] {label} failed and is fail-closed; denying: {error}");
            HookRun {
                output: HookOutput::deny(format!("hook '{label}' could not run: {error}")),
                label,
                duration,
                error: Some(error),
            }
        }
        Err(error) => {
            log::warn!("[hooks] {label} failed; continuing (fail-open): {error}");
            HookRun::noop(label, duration, Some(error))
        }
    }
}

async fn run_command(
    definition: &HookDefinition,
    input: &HookInput,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<HookOutput, String> {
    let payload =
        serde_json::to_vec(input).map_err(|error| format!("serializing input: {error}"))?;

    let mut command =
        crate::openhuman::agent::platform_shell::build_tokio_command(&definition.command);
    if let Some(dir) = definition.source_dir.as_deref().filter(|dir| dir.is_dir()) {
        command.current_dir(dir);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|error| format!("spawning {:?}: {error}", definition.command))?;

    if let Some(mut stdin) = child.stdin.take() {
        // A hook that ignores stdin closes the pipe early; that is a broken pipe
        // on our side, not a hook failure, so the write error is only logged.
        if let Err(error) = stdin.write_all(&payload).await {
            log::debug!("[hooks] {} closed stdin early: {error}", definition.label());
        }
        drop(stdin);
    }

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("waiting on hook: {error}")),
        Err(_) => return Err(format!("timed out after {}s", timeout.as_secs())),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code();

    if code == Some(EXIT_DENY) {
        let reason = first_non_empty(&[&stderr, &stdout])
            .unwrap_or_else(|| format!("hook {} denied the action", definition.label()));
        return Ok(HookOutput::deny(reason));
    }
    if !output.status.success() {
        let detail = first_non_empty(&[&stderr, &stdout]).unwrap_or_default();
        return Err(match code {
            Some(code) => format!("exited with status {code}: {detail}"),
            None => format!("terminated by signal: {detail}"),
        });
    }
    parse_stdout(&stdout)
}

/// Extract the hook's decision from stdout, ignoring anything printed around it.
fn parse_stdout(stdout: &str) -> Result<HookOutput, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(HookOutput::default());
    }
    if let Ok(output) = serde_json::from_str::<HookOutput>(trimmed) {
        return Ok(output);
    }
    // Fall back to the last line that parses on its own — the "script logged
    // first, then answered" case.
    let recovered = trimmed
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
        .find_map(|line| serde_json::from_str::<HookOutput>(line).ok());
    recovered.ok_or_else(|| {
        format!(
            "stdout was not a hook decision object: {}",
            truncate(trimmed, 200)
        )
    })
}

/// Answer shape for a `prompt` hook, mirroring Cursor's `{ ok, reason }`.
#[derive(serde::Deserialize)]
struct PromptVerdict {
    ok: bool,
    #[serde(default)]
    reason: Option<String>,
}

/// Evaluate a natural-language condition with a model.
///
/// The prompt text may contain `$ARGUMENTS`, replaced by the event JSON — the
/// same placeholder Cursor uses, so a prompt hook ports across unchanged.
async fn run_prompt(definition: &HookDefinition, input: &HookInput) -> Result<HookOutput, String> {
    let arguments =
        serde_json::to_string(input).map_err(|error| format!("serializing input: {error}"))?;
    let prompt = if definition.command.contains("$ARGUMENTS") {
        definition.command.replace("$ARGUMENTS", &arguments)
    } else {
        format!("{}\n\nEvent:\n{arguments}", definition.command)
    };
    let instruction = format!(
        "{prompt}\n\nAnswer with JSON only: {{\"ok\": true}} to allow, or \
         {{\"ok\": false, \"reason\": \"...\"}} to deny."
    );

    let answer = super::prompt_eval::evaluate(&instruction, definition.model.as_deref()).await?;
    let verdict: PromptVerdict = serde_json::from_str(answer.trim())
        .or_else(|_| {
            answer
                .lines()
                .rev()
                .map(str::trim)
                .filter(|line| line.starts_with('{'))
                .find_map(|line| serde_json::from_str::<PromptVerdict>(line).ok())
                .ok_or_else(|| {
                    format!("model answer was not a verdict: {}", truncate(&answer, 200))
                })
        })
        .map_err(|error| error.to_string())?;

    if verdict.ok {
        return Ok(HookOutput {
            permission: Some(HookPermission::Allow),
            ..HookOutput::default()
        });
    }
    Ok(HookOutput::deny(verdict.reason.unwrap_or_else(|| {
        format!("hook '{}' denied the action", definition.label())
    })))
}

fn first_non_empty(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .map(|text| text.trim())
        .find(|text| !text.is_empty())
        .map(|text| truncate(text, 2000))
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let head: String = text.chars().take(max_chars).collect();
    format!("{head}…")
}

/// Ambient environment every hook receives, on top of the process environment.
///
/// `CLAUDE_PROJECT_DIR` and `CURSOR_PROJECT_DIR` are set alongside the
/// OpenHuman names so a script written for either host finds its project root
/// without an OpenHuman-specific branch.
pub fn ambient_env(input: &HookInput) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if let Some(root) = input.workspace_roots.first() {
        env.insert("OPENHUMAN_PROJECT_DIR".into(), root.clone());
        env.insert("CLAUDE_PROJECT_DIR".into(), root.clone());
        env.insert("CURSOR_PROJECT_DIR".into(), root.clone());
    }
    env.insert("OPENHUMAN_VERSION".into(), input.openhuman_version.clone());
    env.insert("OPENHUMAN_HOOK_EVENT".into(), input.hook_event_name.clone());
    if let Some(session) = &input.session_id {
        env.insert("OPENHUMAN_SESSION_ID".into(), session.clone());
    }
    if let Some(agent) = &input.agent_id {
        env.insert("OPENHUMAN_AGENT_ID".into(), agent.clone());
    }
    env
}

/// True when a path exists and is executable by this process.
///
/// Read at **load** time so a hook whose script is missing or was committed
/// without the executable bit is reported when the file is read, not when the
/// moment it was meant to guard finally arrives.
pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
#[path = "exec_tests.rs"]
mod tests;
