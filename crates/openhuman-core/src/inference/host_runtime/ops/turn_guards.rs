//! Shared guards applied before a turn runs: prompt-injection enforcement,
//! model-override normalisation, working-directory resolution and grant, and
//! the origin label an agent chat runs under.

use crate::config::Config;
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};

pub(super) fn prompt_guard_user_message(action: PromptEnforcementAction) -> &'static str {
    match action {
        PromptEnforcementAction::Allow => "Message accepted.",
        PromptEnforcementAction::Blocked => {
            "Prompt blocked by security policy. Please rephrase without instruction overrides or exfiltration requests."
        }
        PromptEnforcementAction::ReviewBlocked => {
            "Prompt flagged for security review and was not processed. Please rephrase clearly."
        }
    }
}

/// Normalize a `model_override` string into the `Option<String>` form the
/// downstream config-resolution path expects.
///
/// `None` → `None`. `Some(non-empty-after-trim)` → `Some(trimmed)`. Anything
/// else (`Some("")`, `Some("   ")`, `Some("\t\n")`) collapses to `None` so
/// the existing default-model fallback applies instead of overwriting
/// `config.default_model` with a blank string that the OpenHuman backend
/// would reject with `400 model is required` (Sentry TAURI-RUST-RS).
///
/// Extracted to keep `agent_chat` and `agent_chat_simple` in lockstep —
/// future tweaks (additional log lines, tightening the trim rules) live in
/// exactly one place.
pub(super) fn normalize_model_override(opt: Option<String>) -> Option<String> {
    opt.and_then(|m| {
        let t = m.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

pub(super) fn enforce_user_prompt_or_reject(
    prompt: &str,
    source: &'static str,
) -> Result<(), String> {
    let decision = enforce_prompt_input(
        prompt,
        PromptEnforcementContext {
            source,
            request_id: None,
            user_id: None,
            session_id: Some("local_ai"),
        },
    );
    match decision.action {
        PromptEnforcementAction::Allow => Ok(()),
        PromptEnforcementAction::Blocked | PromptEnforcementAction::ReviewBlocked => {
            Err(prompt_guard_user_message(decision.action).to_string())
        }
    }
}

/// Resolve the per-turn `cwd` parameter into the directory the turn's tools
/// should be rooted at.
///
/// `None` / empty / whitespace-only collapses to `None`, which keeps the turn on
/// the configured `action_dir` exactly as before. A present value must name an
/// existing directory: rooting an agent at a path that does not exist would give
/// it a cwd every shell and file call fails against, so this rejects loudly
/// instead. The path is canonicalized so symlinked and `..`-containing inputs
/// compare equal to the paths the security policy derives from it.
pub(super) fn resolve_turn_cwd(cwd: Option<String>) -> Result<Option<std::path::PathBuf>, String> {
    let Some(raw) = cwd else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = std::path::Path::new(trimmed);
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| format!("cwd '{trimmed}' is not accessible: {e}"))?;
    if !canonical.is_dir() {
        return Err(format!("cwd '{trimmed}' is not a directory"));
    }
    Ok(Some(canonical))
}

/// Grant `root` as a `ReadWrite` trusted root on a per-turn config clone.
///
/// Setting `action_dir` alone only moves where a *relative* path resolves; the
/// allow/deny decision reads `workspace_dir` + `trusted_roots`, so without this
/// grant an absolute path in `root` is refused by `workspace_only` and a
/// relative one is refused as "escapes workspace". Idempotent: an entry the
/// user already configured for the same path is left untouched so a `Read`-only
/// grant is not silently widened by the presence of a `cwd`.
///
/// Only ever called on a *clone* of the config, on the `cwd`-present branch —
/// the no-`cwd` path is untouched, and nothing process-global is mutated.
pub(super) fn grant_turn_cwd(config: &mut Config, root: &std::path::Path) {
    let path = root.to_string_lossy().to_string();
    if config.autonomy.trusted_roots.iter().any(|r| r.path == path) {
        return;
    }
    config
        .autonomy
        .trusted_roots
        .push(crate::security::TrustedRoot {
            path,
            access: crate::security::TrustedAccess::ReadWrite,
        });
}

/// The origin label [`agent_chat`] scopes around its turn.
///
/// An ambient origin — scoped by an in-process embedder around its
/// `invoke("openhuman.inference_agent_chat", …)` — is the caller's own,
/// deliberate trust statement about the turn and is kept. Absent one,
/// [`AgentTurnOrigin::DirectChat`] applies: this RPC is reached by trusted
/// clients (the desktop Settings agent-chat panel, an operator running the RPC
/// by hand), and leaving it unlabelled would fail the approval gate closed on
/// every external-effect tool.
///
/// `DirectChat` rather than the historical `Cli`, and the difference is not
/// cosmetic. The approval gate treats the two identically, so trust is
/// unchanged — but `message` here is something a *person* typed, and `Cli`
/// answers "who wrote this" with a trust answer. Its own documentation covers
/// sub-agent and internal invocations, so
/// [`is_user_authored`](crate::agent::turn_origin::AgentTurnOrigin::is_user_authored)
/// reads `false` for it and the conversation autosave would silently drop a
/// real user message.
pub(super) fn effective_agent_chat_origin(
    ctx: &crate::core::runtime::DispatchCtx,
) -> crate::agent::turn_origin::AgentTurnOrigin {
    ctx.turn_origin_or_scoped()
        .unwrap_or(crate::agent::turn_origin::AgentTurnOrigin::DirectChat)
}
