//! The one translator from product [`Config`] to the TinySecurity wire policy.
//!
//! Everything the module enforces for an agent is derived here and nowhere
//! else: the immutable filesystem scope, the command scope, the nonsensitive
//! effective settings, the exact-name tool rules and the verified subject.
//! The functions are pure. They read no environment, perform no I/O and log
//! only counts, so a secret in `Config` can never reach a log line or a
//! `PolicyInfo`; credentials are not part of any type built here.

use std::path::Path;

use tinysecurity_bus::{
    CommandAutonomy, CommandExecutionMode, CommandPolicy, CommandPolicyId, JudgeConfig, Origin,
    PathPolicy, PathPolicyId, PolicyConfig, PolicySettings, PrivacyMode, RegisteredPolicy, Secret,
    ShellDialect, SubjectScope, ToolRule, ToolRuleEffect,
};
use tinytools::rules::{DefaultEffect, RuleEffect, Surface, ToolRule as HostRule};

use crate::config::Config;
use crate::core::runtime::tenant::Tenant;
use crate::security::{AutonomyLevel, SecurityPolicy};

/// Verified identity of the policy's owner, derived from the authenticated
/// dispatch (never from the process environment).
pub(crate) fn subject_scope(
    tenant: &Tenant,
    workspace_dir: &Path,
    saas: bool,
) -> Result<SubjectScope, String> {
    let user = match (&tenant.profile, saas) {
        (Some(profile), _) if !profile.trim().is_empty() => profile.clone(),
        (_, true) => {
            return Err("TinySecurity SaaS policy requires an authenticated profile".into())
        }
        _ => "local".to_owned(),
    };
    Ok(SubjectScope {
        user,
        workspace: workspace_dir.to_string_lossy().into_owned(),
        agent: tenant
            .agent
            .clone()
            .filter(|agent| !agent.trim().is_empty())
            .unwrap_or_else(|| "orchestrator".to_owned()),
    })
}

/// Filesystem scope of `config`, including the `[privacy]` mode the old
/// `SecurityPolicy::from_config` used to drop.
pub(crate) fn path_policy(config: &Config) -> PathPolicy {
    SecurityPolicy::from_full_config(config).native_path_policy()
}

/// Command scope of `config`. The execution mode stays an explicit argument:
/// `HarnessGated` (class gate plus hidden-execution defense) and `Allowlisted`
/// (legacy curated allowlist) are different products of the same settings.
pub(crate) fn command_policy(
    config: &Config,
    mode: CommandExecutionMode,
    home_dir: Option<std::path::PathBuf>,
) -> CommandPolicy {
    let autonomy = &config.autonomy;
    CommandPolicy {
        enabled: autonomy.enabled,
        execution_mode: mode,
        shell_dialect: if cfg!(windows) {
            ShellDialect::Cmd
        } else {
            ShellDialect::Posix
        },
        autonomy: match autonomy.level {
            AutonomyLevel::ReadOnly => CommandAutonomy::ReadOnly,
            AutonomyLevel::Supervised => CommandAutonomy::Supervised,
            AutonomyLevel::Full => CommandAutonomy::Full,
        },
        allowed_commands: autonomy.allowed_commands.clone(),
        max_actions_per_hour: autonomy.max_actions_per_hour,
        require_approval_for_medium_risk: autonomy.require_approval_for_medium_risk,
        block_high_risk_commands: autonomy.block_high_risk_commands,
        action_dir: config.action_dir.clone(),
        home_dir,
    }
}

pub(crate) fn privacy_mode(mode: crate::config::PrivacyMode) -> PrivacyMode {
    match mode {
        crate::config::PrivacyMode::LocalOnly => PrivacyMode::LocalOnly,
        crate::config::PrivacyMode::Standard => PrivacyMode::Standard,
        crate::config::PrivacyMode::Sensitive => PrivacyMode::Sensitive,
    }
}

/// Origins that may be approved automatically. `auto_approve_all` is the only
/// blanket opt-in; a remote caller (`Saas`) never inherits it.
fn auto_origins(config: &Config) -> Vec<Origin> {
    if config.autonomy.auto_approve_all {
        vec![
            Origin::Operator,
            Origin::Copilot,
            Origin::SubAgent,
            Origin::Cron,
        ]
    } else {
        Vec::new()
    }
}

/// Origins opted in to the judge, or `None` when the judge is off. Unknown
/// origin names are dropped; an empty result also leaves the judge off, so a
/// typo never widens automatic approval.
fn judge_origins(config: &Config) -> Option<Vec<Origin>> {
    let judge = &config.security.auto_approve.judge;
    if !judge.enabled {
        return None;
    }
    let origins: Vec<Origin> = judge
        .origins
        .iter()
        .filter_map(|name| {
            serde_json::from_value::<Origin>(serde_json::Value::String(name.trim().to_lowercase()))
                .map_err(|_| log::warn!("[security][judge] ignoring unknown origin"))
                .ok()
        })
        .collect();
    (!origins.is_empty()).then_some(origins)
}

/// Init-only judge configuration for the module. The credential comes from
/// `resolve_backend_credential`; without one (or with the judge off) there is
/// no judge and every decision falls to a human.
pub(crate) fn judge_config(config: &Config) -> Option<JudgeConfig> {
    let origins = judge_origins(config)?;
    let credential =
        match crate::security::credentials::session_support::resolve_backend_credential(config) {
            Ok(credential) => credential,
            Err(_) => {
                log::warn!("[security][judge] enabled but no backend credential; judge disabled");
                return None;
            }
        };
    let judge = &config.security.auto_approve.judge;
    log::debug!(
        "[security][judge] configured origins={} threshold={} timeout_ms={}",
        origins.len(),
        judge.threshold_millis,
        judge.timeout_ms
    );
    Some(JudgeConfig {
        credential: Secret(credential.into_secret()),
        endpoint: judge.endpoint.clone(),
        provider: Default::default(),
        threshold_millis: judge.threshold_millis,
        exfiltrate_millis: judge.exfiltrate_millis,
        thresholds: Vec::new(),
        timeout_ms: judge.timeout_ms,
        origins,
        max_requests: judge.max_requests,
        budget_window_ms: judge.budget_window_ms,
        cache_entries: 0,
        cache_ttl_ms: 0,
    })
}

/// Result of translating the operator's `[tool_rules]` layer.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ToolRuleTranslation {
    /// Rules the module can enforce by exact tool name.
    pub rules: Vec<ToolRule>,
    /// Rules left to the host's tool-policy middleware (globs, tags, ...).
    pub host_only: usize,
}

fn exact_names(rule: &HostRule) -> Option<Vec<&str>> {
    let m = &rule.matcher;
    let only_name = m.family.is_none()
        && m.tags.is_none()
        && m.category.is_none()
        && m.exposure.is_none()
        && m.permission_at_least.is_none()
        && m.permission_at_most.is_none()
        && m.side_effects.is_none()
        && m.arg.is_none();
    let reaches_calls = rule.on.is_empty() || rule.on.contains(&Surface::Call);
    if !only_name || rule.except.is_some() || !rule.when.is_empty() || !reaches_calls {
        return None;
    }
    let patterns = &m.name.as_ref()?.0;
    if patterns.is_empty() || patterns.iter().any(|p| p.contains(['*', '?', '['])) {
        return None;
    }
    Some(patterns.iter().map(String::as_str).collect())
}

/// Translate the operator tool-rule layer. Only narrowing facts are sent: exact
/// `deny` names always, and exact `allow` names only when the layer's default
/// is `deny` and every allow rule is exact (a partial allowlist would deny the
/// tools a glob allow rule admits). Everything else stays host-enforced.
pub(crate) fn tool_rules(config: &Config) -> ToolRuleTranslation {
    let layer = &config.tool_rules;
    let mut out = ToolRuleTranslation::default();
    let allow_exact = layer.default == DefaultEffect::Deny
        && layer
            .rules
            .iter()
            .filter(|r| r.effect == RuleEffect::Allow)
            .all(|r| exact_names(r).is_some());
    for rule in &layer.rules {
        let effect = match rule.effect {
            RuleEffect::Deny => Some(ToolRuleEffect::Deny),
            RuleEffect::Allow if allow_exact => Some(ToolRuleEffect::Allow),
            _ => None,
        };
        match (effect, exact_names(rule)) {
            (Some(effect), Some(names)) => {
                out.rules.extend(names.into_iter().map(|tool| ToolRule {
                    tool: tool.to_owned(),
                    effect,
                }))
            }
            _ => out.host_only += 1,
        }
    }
    out
}

/// Nonsensitive effective settings of `config`.
pub(crate) fn settings(config: &Config) -> PolicySettings {
    PolicySettings {
        policy: PolicyConfig {
            enabled: config.autonomy.enabled,
            action_dir: config.action_dir.to_string_lossy().into_owned(),
            workspace_dir: config.workspace_dir.to_string_lossy().into_owned(),
            allowed_commands: config.autonomy.allowed_commands.clone(),
            // tinysecurity >= 0.5.0 (contract 1.13) folds `~`, relative
            // (against `action_dir`) and floor-covered entries, so the real
            // operator list is registered rather than an empty one.
            forbidden_paths: config.autonomy.forbidden_paths.clone(),
        },
        privacy: privacy_mode(config.privacy.mode),
        egress_hosts: Vec::new(),
        command_classes: Vec::new(),
        auto_origins: auto_origins(config),
        judge_enabled: judge_origins(config).is_some(),
    }
}

/// Complete registration for one verified subject, given the scope ids the
/// module returned for [`path_policy`] and [`command_policy`].
pub(crate) fn registered_policy(
    config: &Config,
    subject_scope: SubjectScope,
    path_policy_id: PathPolicyId,
    command_policy_id: CommandPolicyId,
) -> RegisteredPolicy {
    let translation = tool_rules(config);
    log::debug!(
        "[security][policy] translated enabled={} tool_rules={} host_only_rules={}",
        config.autonomy.enabled,
        translation.rules.len(),
        translation.host_only
    );
    RegisteredPolicy {
        path_policy_id,
        settings: settings(config),
        tool_rules: translation.rules,
        command_policy_id,
        subject_scope,
    }
}

#[cfg(test)]
#[path = "security_policy_tests.rs"]
mod tests;
