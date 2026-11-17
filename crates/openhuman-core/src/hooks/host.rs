//! The process-global hook engine and the OpenHuman seams it is built with.
//!
//! The engine, `hooks.json` contract and process runner live in
//! `tinyagents_hooks::command`. This module supplies what is specific
//! to this host: the product name that decides discovery paths and the
//! `OPENHUMAN_*` hook environment, the user's home directory, the platform
//! shell, and the model behind `prompt` hooks.

use crate::core::runtime::DispatchCtx;
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use tinyagents_hooks::command::config::{HookKind, HookLayer};
use tinyagents_hooks::command::{config, HookConfig, HookEngine, HookEnvironment, PromptEvaluator};

/// Product name handed to the engine: `ProgramData\OpenHuman`,
/// `/Library/Application Support/OpenHuman`, `/etc/openhuman`, `.openhuman`,
/// and the `OPENHUMAN_*` hook variables all derive from it.
pub const PRODUCT_NAME: &str = "OpenHuman";

struct HostPromptEvaluator;

#[async_trait]
impl PromptEvaluator for HostPromptEvaluator {
    async fn evaluate(&self, instruction: &str, model: Option<&str>) -> Result<String, String> {
        super::prompt_eval::evaluate(&DispatchCtx::ambient(), instruction, model).await
    }
}

pub(super) fn environment() -> HookEnvironment {
    HookEnvironment::new(PRODUCT_NAME, dirs::home_dir())
        .with_shell(super::shell::command)
        .with_prompt_evaluator(Arc::new(HostPromptEvaluator))
}

static ENGINE: LazyLock<Arc<HookEngine>> =
    LazyLock::new(|| Arc::new(HookEngine::new(environment())));

/// The process-global engine.
pub fn engine() -> &'static Arc<HookEngine> {
    #[cfg(test)]
    if let Some(engine) = super::fixture::pinned() {
        return engine;
    }
    &ENGINE
}

/// Drop the command hooks this host will not run, with a warning naming each:
///
/// - Every project-layer command hook. `<action_dir>/.openhuman/hooks.json`
///   is the least trusted layer (a cloned repository controls it), and a hook
///   process cannot be isolated yet: the engine spawns it synchronously through
///   [`super::shell::command`], and TinyBox's native jail does not run on Linux
///   (TS-E2E-020). Until both change, such a hook does not run at all.
/// - Any other command hook TinySecurity's command check refuses, the same
///   check a scheduled shell job passes before it runs.
///
/// Prompt hooks spawn no process and are kept. A config that cannot be read
/// drops every command hook rather than running one unchecked.
async fn admit_command_hooks(ctx: &DispatchCtx, loaded: &mut HookConfig) {
    let checker = ctx
        .load_config()
        .await
        .map(|config| {
            // The configured privacy mode, not only the live one: hooks load
            // before the live policy is installed, and with security off the
            // local-only refusal of network commands reads the policy's mode.
            crate::security::SecurityPolicy::from_config(
                &config.autonomy,
                &config.workspace_dir,
                &config.action_dir,
            )
            .with_privacy_mode(config.privacy.mode)
        })
        .map_err(|error| error.to_string());
    let mut refused = Vec::new();
    for definitions in loaded.by_event.values_mut() {
        let mut admitted = Vec::with_capacity(definitions.len());
        for definition in std::mem::take(definitions) {
            let refusal = if definition.kind != HookKind::Command {
                None
            } else if definition.layer == Some(HookLayer::Project) {
                Some("project hooks need isolation, which is not available yet".to_owned())
            } else {
                match &checker {
                    Ok(security) => check_hook_command(security, &definition.command).await,
                    Err(error) => Some(format!("cannot read config to check it: {error}")),
                }
            };
            match refusal {
                None => admitted.push(definition),
                Some(reason) => refused.push(format!(
                    "command hook {} disabled: {reason}",
                    definition.label()
                )),
            }
        }
        *definitions = admitted;
    }
    loaded.warnings.extend(refused);
}

/// Why TinySecurity refuses `command` as a scheduled shell command, if it does.
///
/// The same question cron asks of a shell job, at the `Authorize` stage: it
/// filters what may load and spends no action budget, so a reload is free.
/// Only an `Allow` admits the hook: a `Prompt` cannot be answered by a hook.
/// Each later run is not checked again: the engine spawns hook processes
/// synchronously, which leaves no async seam for a per-run check.
async fn check_hook_command(
    security: &crate::security::SecurityPolicy,
    command: &str,
) -> Option<String> {
    use crate::security::policy::{CommandFlow, CommandQuery, CommandStage, GateDecision};
    match security
        .check_command(CommandQuery {
            command,
            declared: None,
            flow: CommandFlow::Scheduled,
            stage: CommandStage::Authorize,
        })
        .await
    {
        Ok(verdict) => match verdict.gate {
            GateDecision::Allow => None,
            // A hook runs unattended, so nobody can grant the approval a
            // `Prompt` asks for; admitting it would run it without one.
            GateDecision::Prompt => Some("needs approval, which a hook cannot ask for".to_owned()),
            GateDecision::Block => Some("blocked by security policy: command rejected".to_owned()),
        },
        Err(reason) => Some(reason.to_string()),
    }
}

/// Re-read every `hooks.json` layer and install the result.
///
/// The engine's own [`HookEngine::reload`] reuses the environment it was built
/// with, which froze the home directory at first use. Discovery resolves the
/// user layer against the home directory **now**, as it did before the engine
/// moved upstream, so a reload after `HOME` changes reads the right file.
pub async fn reload(
    ctx: &DispatchCtx,
    project_dir: Option<std::path::PathBuf>,
    workspace_dir: Option<std::path::PathBuf>,
) -> Arc<HookConfig> {
    // Hook commands run under the shell TinyBox selects; ask it here, where the
    // caller is async, so the engine's synchronous shell factory can use it.
    super::shell::resolve(ctx).await;
    let current = environment();
    let mut loaded = tokio::task::spawn_blocking(move || {
        config::load(&current, project_dir.as_deref(), workspace_dir.as_deref())
    })
    .await
    .unwrap_or_default();
    // Under the reload's own context: TinySecurity's command check reloads config
    // through the ambient context, which SaaS startup does not otherwise bind.
    ctx.enter(admit_command_hooks(ctx, &mut loaded)).await;
    for warning in &loaded.warnings {
        log::warn!("[hooks] {warning}");
    }
    log::info!(
        "[hooks] active configuration: {} hook(s) across {} file(s)",
        loaded.len(),
        loaded.sources.len()
    );
    ENGINE.install(loaded).await;
    ENGINE.snapshot().await
}
