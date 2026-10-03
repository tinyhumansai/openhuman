//! Permanent tool sources attached to an already configured agent.
use super::Agent;
use openhuman_core::agent::tool_policy::{ToolPolicy, ToolPolicyDecision, ToolPolicyRequest};
use openhuman_core::agent::{HostTools, TurnContext};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

/// An invalid or conflicting permanent tool registration.
#[derive(Debug, thiserror::Error)]
pub enum ToolAttachmentError {
    /// Source keys must be nonempty.
    #[error("tool source key is empty")]
    EmptyKey,
    /// Another factory already owns this source key.
    #[error("tool source {0:?} is already attached")]
    SourceConflict(String),
    /// Tool names must be unique across sources and the configured agent.
    #[error("tool name {0:?} is already registered")]
    NameCollision(String),
}
#[derive(Default)]
pub(crate) struct Attachments(Mutex<BTreeMap<String, Attachment>>);
struct Attachment {
    factory: HostTools,
    names: HashSet<String>,
}
impl Agent {
    /// Attach a permanent, directly advertised tool source to this agent.
    ///
    /// Clones share registrations. The same key and factory `Arc` is idempotent;
    /// a different factory under that key is rejected. Factories are sampled
    /// with no session at registration to validate names, then rebuilt per turn.
    /// Keep tool names stable. Their callbacks should authorize their operations.
    /// Only these names use the attachment policy; other tools keep their gate.
    pub fn attach_tools(
        &self,
        key: impl Into<String>,
        factory: HostTools,
    ) -> Result<(), ToolAttachmentError> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(ToolAttachmentError::EmptyKey);
        }
        let mut sources = self
            .inner
            .attachments
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = sources.get(&key) {
            return if Arc::ptr_eq(&existing.factory, &factory) {
                Ok(())
            } else {
                Err(ToolAttachmentError::SourceConflict(key))
            };
        }
        let context = TurnContext::new(self.id(), None);
        let mut occupied: HashSet<String> = sources
            .values()
            .flat_map(|source| source.names.iter().cloned())
            .collect();
        if let Some(host) = &self.inner.host_tools {
            occupied.extend(
                host(context)
                    .tools
                    .iter()
                    .map(|tool| tool.name().to_string()),
            );
        }
        occupied.extend(
            openhuman_core::tools::registry::ops::registry_entries_for_config(&self.inner.config)
                .into_iter()
                .map(|entry| entry.name),
        );
        let config = &self.inner.config;
        let security = Arc::new(openhuman_core::security::SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        ));
        occupied.extend(
            openhuman_core::tools::ops::all_tools(
                Arc::new(config.clone()),
                &security,
                openhuman_core::security::AuditLogger::disabled(),
                &config.browser,
                &config.http_request,
                &config.action_dir,
                &Default::default(),
                config,
            )
            .iter()
            .map(|tool| tool.name().to_string()),
        );
        let tools = factory(context);
        let mut names = HashSet::new();
        for tool in tools.tools {
            let name = tool.name().to_string();
            if occupied.contains(&name) || !names.insert(name.clone()) {
                return Err(ToolAttachmentError::NameCollision(name));
            }
        }
        sources.insert(key, Attachment { factory, names });
        Ok(())
    }

    /// Opaque process-runtime identity shared by every agent on this runtime.
    pub fn runtime_id(&self) -> &str {
        &self.inner.runtime_id
    }

    /// Whether both handles refer to the same instantiated agent.
    pub fn same_agent(&self, other: &Agent) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl super::AgentInner {
    pub(crate) fn composed_host_tools(self: &Arc<Self>) -> Option<HostTools> {
        let sources: Vec<HostTools> = self
            .attachments
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(|source| source.factory.clone())
            .collect();
        if sources.is_empty() {
            return self.host_tools.clone();
        }
        let host = self.host_tools.clone();
        Some(Arc::new(move |context| {
            let mut merged = host.as_ref().map(|host| host(context)).unwrap_or_default();
            let original = merged.policy.take();
            let mut policies = Vec::new();
            for source in &sources {
                let attached = source(context);
                let names: HashSet<String> = attached
                    .tools
                    .iter()
                    .map(|tool| tool.name().to_string())
                    .collect();
                merged.visible.extend(names.iter().cloned());
                merged.permanent.extend(names.iter().cloned());
                merged.tools.extend(attached.tools);
                policies.push((names, attached.policy));
            }
            merged.policy = Some(Arc::new(AttachmentPolicy { original, policies }));
            merged
        }))
    }
}
type SourcePolicy = (HashSet<String>, Option<Arc<dyn ToolPolicy>>);
struct AttachmentPolicy {
    original: Option<Arc<dyn ToolPolicy>>,
    policies: Vec<SourcePolicy>,
}
#[async_trait::async_trait]
impl ToolPolicy for AttachmentPolicy {
    fn name(&self) -> &str {
        "permanent_attachments"
    }
    async fn check(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
        for (names, policy) in &self.policies {
            if names.contains(&request.tool_name) {
                return match policy {
                    Some(policy) => policy.check(request).await,
                    None => ToolPolicyDecision::Allow,
                };
            }
        }
        match &self.original {
            Some(policy) => policy.check(request).await,
            None => ToolPolicyDecision::Allow,
        }
    }
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
