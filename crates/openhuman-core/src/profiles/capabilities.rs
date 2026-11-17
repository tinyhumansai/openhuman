//! Immutable operator-selected tool and memory adapters for hosted profiles.
//!
//! Registration is narrowed before the session receives its registry. Every
//! call additionally passes the profile lease fence and the host permission
//! callback. These adapters never install process-wide user capabilities.

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::{HostTools, HostTurnTools, TurnContext};
use crate::config::Config;
use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx};
use crate::tools::toolpacks::{GroupMode, ToolGroups};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tinyagents_hooks::tool::{ToolHook, ToolHookContext, ToolHookDecision};
use tinytools::PermissionLevel;

/// Capabilities fixed when the profile runtime boots.
pub struct ProfileCapabilities {
    host: Option<HostTools>,
    names: Option<HashSet<String>>,
    permission: Option<Arc<dyn ToolHook<OpenHumanRunContext>>>,
    channels: HashMap<String, PermissionLevel>,
    groups: Vec<super::tools::SaasToolGroup>,
    /// The shared engine, resolved only under the forced profile memory root.
    pub memory_engine: Option<Arc<dyn tinymemory_api::MemoryEngine>>,
    /// The operator's whole system prompt, when set.
    system_prompt: Option<String>,
}

impl ProfileCapabilities {
    /// Construct immutable adapters. `None` names select only supplied host tools.
    pub fn new(
        host: Option<HostTools>,
        names: Option<HashSet<String>>,
        permission: Option<Arc<dyn ToolHook<OpenHumanRunContext>>>,
        channels: HashMap<String, PermissionLevel>,
        groups: Vec<super::tools::SaasToolGroup>,
        memory_engine: Option<Arc<dyn tinymemory_api::MemoryEngine>>,
    ) -> Self {
        Self {
            host,
            names,
            permission,
            channels,
            groups,
            memory_engine,
            system_prompt: None,
        }
    }

    /// Replace every profile's system prompt with `prompt`, verbatim: no
    /// SOUL/IDENTITY files, routing text, tool guidance or style block.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: Option<String>) -> Self {
        self.system_prompt = prompt;
        self
    }

    /// Make `definition` render the operator's system prompt, if one is set.
    pub fn apply_system_prompt(
        &self,
        definition: &mut crate::agent_registry::host_definition::AgentDefinition,
    ) {
        if let Some(prompt) = &self.system_prompt {
            definition.system_prompt =
                tinyagents_definition::PromptSource::Verbatim(prompt.clone());
        }
    }

    /// Capabilities of the authenticated profile `ctx` runs for, never the
    /// operator: `None` for a call that serves no profile.
    pub fn in_ctx(ctx: &DispatchCtx) -> Option<Arc<Self>> {
        let core = ctx.core()?;
        core.profile()?;
        core.host_overrides()?.profile_capabilities()
    }

    /// Permission ceiling explicitly granted to this hosted channel.
    pub fn channel_permission(&self, channel: &str) -> PermissionLevel {
        self.channels.get(channel).copied().unwrap_or_default()
    }

    /// Overlay the operator's explicit ceilings; unknown channels stay read-only.
    pub fn configure(&self, config: &mut Config) {
        config.agent.channel_permissions.extend(
            self.channels
                .iter()
                .map(|(name, level)| (name.clone(), level.to_string().to_lowercase())),
        );
    }

    /// Whether the operator named `name` in its tool scope and it is one of
    /// [`OPERATOR_ADMITTABLE`](super::tools::OPERATOR_ADMITTABLE), the tools
    /// registered although their domain family is off for profiles.
    pub fn names_admittable(&self, name: &str) -> bool {
        super::tools::OPERATOR_ADMITTABLE.contains(&name)
            && self
                .names
                .as_ref()
                .is_some_and(|names| names.contains(name))
    }

    /// Whether a selected name may reach the host permission decision.
    pub fn admits(&self, ctx: &DispatchCtx, name: &str) -> bool {
        let binding = super::provider_settings::ProfileProviderSettings::in_ctx(ctx);
        let canonical = match &binding {
            Some(binding) => match binding.policy_name(name) {
                Some(canonical) => canonical,
                None => return false,
            },
            None => std::borrow::Cow::Borrowed(name),
        };
        self.admits_capability(&canonical)
            && self.names.as_ref().is_none_or(|names| names.contains(name))
    }

    fn admits_capability(&self, name: &str) -> bool {
        super::tools::admits_with(name, true, &self.groups)
            && !matches!(
                crate::tools::ops::tool_group(name),
                crate::core::all::DomainGroup::Skills
                    | crate::core::all::DomainGroup::Flows
                    | crate::core::all::DomainGroup::Mcp
            )
            && !matches!(name, "http_request" | "use_skill")
            && !super::provider_settings::is_profile_spawn_tool(name)
            && !name.starts_with("delegate_")
            && !name.starts_with("mcp_")
            && crate::tools::toolpacks::pack_for_tool(name)
                .is_none_or(|pack| pack.id != "workflows")
    }

    /// An explicitly named operation can pass the supervised gate only with a host callback.
    pub fn authorizes_approval(&self, ctx: &DispatchCtx, name: &str) -> bool {
        self.permission.is_some()
            && self
                .names
                .as_ref()
                .is_some_and(|names| names.contains(name))
            && self.admits(ctx, name)
    }

    /// Build the exact selected belt for this profile and session.
    pub async fn turn_tools(&self, config: &Config, turn: TurnContext<'_>) -> HostTurnTools {
        let mut supplied = match self.host.as_ref() {
            Some(host) => host(turn).await,
            None => HostTurnTools::default(),
        };
        let dispatch = turn.dispatch_ctx();
        supplied
            .tools
            .retain(|tool| self.admits(&dispatch, tool.name()));
        if let Some(names) = &self.names {
            let mut groups = ToolGroups::none();
            for name in names.iter().filter(|name| self.admits(&dispatch, name)) {
                let binding = super::provider_settings::ProfileProviderSettings::in_ctx(&dispatch);
                let canonical = binding
                    .as_ref()
                    .and_then(|binding| binding.policy_name(name));
                if let Some(pack) =
                    crate::tools::toolpacks::pack_for_tool(canonical.as_deref().unwrap_or(name))
                {
                    groups = groups.with(pack.id, GroupMode::Advertised);
                }
            }
            if let Some(ctx) = dispatch.core().cloned() {
                // Native construction stays inside the domains registered for this
                // profile. The named packs narrow registration within those domains.
                let assembly =
                    ctx.derive_with(ContextOverlay::new(config.clone(), ctx.domains(), groups));
                let security = Arc::new(crate::security::SecurityPolicy::from_config(
                    &config.autonomy,
                    &config.workspace_dir,
                    &config.action_dir,
                ));
                let native = CoreContext::scope(assembly, async {
                    crate::tools::ops::all_tools(
                        Arc::new(config.clone()),
                        &security,
                        crate::security::AuditSink::disabled(),
                        &config.browser,
                        &config.http_request,
                        &config.action_dir,
                        config,
                    )
                    .await
                });
                let native = native.await;
                let supplied_names: HashSet<_> = supplied
                    .tools
                    .iter()
                    .map(|t| t.name().to_string())
                    .collect();
                // Search tools are classified as ordinary platform tools, which
                // a profile's domain set withholds. The operator's direct search
                // settings are the profile-owned source for them, so add the
                // roles the binding advertises; `admits` still applies.
                let search = if config.profile_providers.is_some() {
                    crate::search::build_search_tools(config).await
                } else {
                    Vec::new()
                };
                supplied.tools.extend(
                    native
                        .into_iter()
                        .filter(|tool| tool.name() != "web_search_tool")
                        .chain(search)
                        .filter(|tool| {
                            self.admits(&dispatch, tool.name())
                                && !supplied_names.contains(tool.name())
                        }),
                );
            }
        }
        // Metadata from the host cannot widen registration or hide a registered
        // tool from the catalogue: the selected belt is advertised exactly.
        supplied.visible = supplied
            .tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();
        supplied.permanent.clear();
        supplied.withheld.clear();
        supplied
    }

    /// A factory retaining these immutable adapters and this forced config.
    pub fn factory(self: &Arc<Self>, config: &Config) -> HostTools {
        let capabilities = Arc::clone(self);
        let config = config.clone();
        Arc::new(move |turn| {
            let capabilities = Arc::clone(&capabilities);
            let config = config.clone();
            Box::pin(async move { capabilities.turn_tools(&config, turn).await })
        })
    }

    /// Bind the permission callback to the owning profile's lease.
    pub fn hook(
        self: &Arc<Self>,
        profile: &super::ProfileId,
        fence: Arc<crate::storage::fence::LeaseFence>,
    ) -> Arc<dyn ToolHook<OpenHumanRunContext>> {
        Arc::new(ProfilePermission {
            capabilities: Arc::clone(self),
            profile: profile.as_str().to_owned(),
            fence,
        })
    }
}

struct ProfilePermission {
    capabilities: Arc<ProfileCapabilities>,
    profile: String,
    fence: Arc<crate::storage::fence::LeaseFence>,
}

#[async_trait::async_trait]
impl ToolHook<OpenHumanRunContext> for ProfilePermission {
    fn name(&self) -> &str {
        "profile.can_use_tool"
    }
    async fn before_tool_decision(
        &self,
        host: &OpenHumanRunContext,
        context: &ToolHookContext,
    ) -> ToolHookDecision {
        let dispatch = host.dispatch_ctx();
        let tenant = dispatch.tenant().ok().and_then(|tenant| tenant.profile);
        if tenant.as_deref() != Some(self.profile.as_str())
            || self.fence.check_local(super::lease::now_ms()).is_err()
            || !self.capabilities.admits(&dispatch, &context.tool_name)
        {
            return ToolHookDecision::deny(
                "tool is outside the hosted profile capability boundary",
            );
        }
        match &self.capabilities.permission {
            Some(permission) => {
                let binding = super::provider_settings::ProfileProviderSettings::in_ctx(&dispatch);
                let canonical = binding
                    .as_ref()
                    .and_then(|binding| binding.canonical_role_name(&context.tool_name));
                let mut advertised = context.clone();
                let mut rewritten = false;
                if let Some(canonical) = canonical.filter(|name| *name != context.tool_name) {
                    if !self.capabilities.admits_capability(canonical) {
                        return ToolHookDecision::deny("canonical search capability is denied");
                    }
                    let mut original = context.clone();
                    original.tool_name = canonical.to_owned();
                    let decision = permission.before_tool_decision(host, &original).await;
                    if decision.is_deny() || decision.is_ask() {
                        return decision;
                    }
                    if let Some(arguments) = decision.updated_input {
                        advertised.arguments = arguments;
                        rewritten = true;
                    }
                }
                let mut decision = permission.before_tool_decision(host, &advertised).await;
                if rewritten && !decision.is_deny() && !decision.is_ask() {
                    decision.updated_input = Some(advertised.arguments);
                }
                decision
            }
            None => ToolHookDecision::deny("this profile has no host tool permission callback"),
        }
    }
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
