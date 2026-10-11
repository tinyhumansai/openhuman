//! Operator adapters installed once, before the SaaS profile host boots.

use super::{ProfileError, ProfileRuntimeBuilder, SaasConfig};
use crate::seams::{ToolHook, ToolHookContext};
use crate::{
    HostTools, HostTurnTools, PermissionFuture, PermissionLevel, ToolScopeSpec, TurnContext,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct ProfileHostOptions {
    tools: Option<HostTools>,
    scope: Option<ToolScopeSpec>,
    permission: Option<Arc<dyn ToolHook>>,
    channels: HashMap<String, PermissionLevel>,
    memory: Option<Arc<dyn tinymemory_api::MemoryEngine>>,
    system_prompt: Option<String>,
}

impl ProfileHostOptions {
    pub(super) fn finish(
        self,
        config: &SaasConfig,
    ) -> Result<Arc<openhuman_core::profiles::capabilities::ProfileCapabilities>, ProfileError>
    {
        let names = match self.scope {
            Some(ToolScopeSpec::Wildcard) => {
                return Err(ProfileError::Boot(
                    "profile tools require a named or host-only scope".into(),
                ))
            }
            Some(ToolScopeSpec::Named(names)) => Some(names.into_iter().collect()),
            _ => None,
        };
        if self
            .channels
            .values()
            .any(|level| *level == PermissionLevel::Dangerous)
        {
            return Err(ProfileError::Boot(
                "profile channel permissions cannot be dangerous".into(),
            ));
        }
        if self
            .system_prompt
            .as_deref()
            .is_some_and(|p| p.trim().is_empty())
        {
            return Err(ProfileError::Boot(
                "the profile system prompt must not be empty".into(),
            ));
        }
        Ok(Arc::new(
            openhuman_core::profiles::capabilities::ProfileCapabilities::new(
                self.tools,
                names,
                self.permission,
                self.channels,
                openhuman_core::profiles::tools::parse_allowlist(&config.tool_allowlist),
                self.memory,
            )
            .with_system_prompt(self.system_prompt),
        ))
    }
}

impl ProfileRuntimeBuilder {
    /// Supply a per-turn host tool factory. Hard SaaS denies remain stronger.
    /// The factory receives the exact chat or relay thread in `session_id`.
    #[must_use]
    pub fn tools(
        mut self,
        factory: impl for<'a> Fn(TurnContext<'a>) -> HostTurnTools + Send + Sync + 'static,
    ) -> Self {
        self.host_options.tools = Some(Arc::new(factory));
        self
    }

    /// Select exactly these native and host names, or only the host belt.
    /// Wildcard scopes are refused at build time.
    #[must_use]
    pub fn tool_scope(mut self, scope: ToolScopeSpec) -> Self {
        self.host_options.scope = Some(scope);
        self
    }

    /// Decide each admitted call under its owning, lease-fenced profile.
    /// Without this callback all tool executions are denied.
    #[must_use]
    pub fn can_use_tool<F>(mut self, callback: F) -> Self
    where
        F: for<'a> Fn(&'a ToolHookContext) -> PermissionFuture<'a> + Send + Sync + 'static,
    {
        self.host_options.permission =
            Some(Arc::new(crate::permission::PermissionHook::new(callback)));
        self
    }

    /// Grant explicit ceilings to hosted profile channels. Unknown channels
    /// remain read-only. Write or Execute is needed for native memory and cron
    /// mutation; Dangerous is refused. This never widens ordinary channel turns.
    #[must_use]
    pub fn channel_permissions(
        mut self,
        channels: impl IntoIterator<Item = (impl Into<String>, PermissionLevel)>,
    ) -> Self {
        self.host_options.channels = channels
            .into_iter()
            .map(|(name, level)| (name.into(), level))
            .collect();
        self
    }

    /// Own every profile's system prompt. `prompt` is sent verbatim, as the
    /// whole system prompt of every chat, relay and cron turn: it replaces
    /// each profile's SOUL.md and IDENTITY.md and OpenHuman's own routing,
    /// tool-guidance and style sections, so nothing in it can mention a tool
    /// the host does not offer. Users cannot override it. An empty prompt is
    /// refused at build time. Without this the built-in prompt is used.
    #[must_use]
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.host_options.system_prompt = Some(prompt.into());
        self
    }

    /// Bind an existing engine through each profile's forced memory namespace.
    /// The binding is profile-local; the global SaaS engine refusal stays intact.
    #[must_use]
    pub fn memory_engine(mut self, engine: Arc<dyn tinymemory_api::MemoryEngine>) -> Self {
        self.host_options.memory = Some(engine);
        self
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
