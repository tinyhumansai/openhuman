//! Hook-driven context assembly for the multi-agent harness.
//!
//! Before entering the orchestrator loop, this module assembles the bootstrap
//! context: identity files, workspace state, and relevant memory.

use crate::openhuman::config::Config;
use crate::openhuman::memory::store::profile;
use crate::openhuman::memory::Memory;
use crate::openhuman::memory::UnifiedMemory;
use std::path::Path;
use std::sync::Arc;

/// Assembled context for the orchestrator's system prompt.
#[derive(Debug, Clone, Default)]
pub struct BootstrapContext {
    /// Contents of the archetype-specific system prompt file.
    pub archetype_prompt: String,
    /// Core identity (from IDENTITY.md / SOUL.md).
    pub identity_context: String,
    /// Workspace state summary (git status, file tree).
    pub workspace_summary: String,
    /// Relevant memory context.
    pub memory_context: String,
    /// User profile context (accumulated preferences, facts, skills).
    pub user_profile_context: String,
}

impl BootstrapContext {
    /// Render the full system prompt by combining all context sections.
    pub fn render(&self) -> String {
        let mut parts = Vec::new();

        if !self.identity_context.is_empty() {
            parts.push(format!("## Identity\n{}", self.identity_context));
        }
        if !self.archetype_prompt.is_empty() {
            parts.push(self.archetype_prompt.clone());
        }
        if !self.workspace_summary.is_empty() {
            parts.push(format!("## Workspace\n{}", self.workspace_summary));
        }
        if !self.user_profile_context.is_empty() {
            parts.push(format!("## User Profile\n{}", self.user_profile_context));
        }
        if !self.memory_context.is_empty() {
            parts.push(format!("## Relevant Memory\n{}", self.memory_context));
        }

        parts.join("\n\n---\n\n")
    }
}

/// Load an archetype prompt file from the prompts directory.
pub async fn load_archetype_prompt(prompts_dir: &Path, relative_path: &str) -> String {
    let path = prompts_dir.join(relative_path);
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            tracing::debug!(
                "[context-assembly] loaded archetype prompt: {}",
                path.display()
            );
            content
        }
        Err(e) => {
            tracing::warn!(
                "[context-assembly] failed to load prompt {}: {e}",
                path.display()
            );
            String::new()
        }
    }
}

/// Load identity context from workspace IDENTITY.md and SOUL.md.
pub async fn load_identity_context(workspace_dir: &Path) -> String {
    let mut parts = Vec::new();

    for filename in &["IDENTITY.md", "SOUL.md"] {
        let path = workspace_dir.join(filename);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            parts.push(content);
            tracing::debug!(
                "[context-assembly] loaded identity file: {}",
                path.display()
            );
        }
    }

    parts.join("\n\n")
}

/// Build memory context by recalling relevant entries.
pub async fn build_memory_context(memory: &dyn Memory, query: &str, max_chars: usize) -> String {
    match memory.recall(query, 5, None).await {
        Ok(entries) => {
            let mut context = String::new();
            for entry in entries {
                let addition = format!("- {}: {}\n", entry.key, entry.content);
                if context.len() + addition.len() > max_chars {
                    break;
                }
                context.push_str(&addition);
            }
            context
        }
        Err(e) => {
            tracing::debug!("[context-assembly] memory recall failed: {e}");
            String::new()
        }
    }
}

/// Load user profile context from the profile table.
pub fn load_user_profile_context(_memory: &dyn Memory) -> String {
    // Try to access the UnifiedMemory connection for profile loading.
    // The Memory trait doesn't expose this, so we use a separate function
    // that accepts UnifiedMemory directly.
    // This is a best-effort operation — returns empty if profile is unavailable.
    String::new()
}

/// Load user profile from a UnifiedMemory instance.
pub fn load_user_profile_from_unified(unified: &UnifiedMemory) -> String {
    match profile::profile_load_all(&unified.conn) {
        Ok(facets) if !facets.is_empty() => {
            tracing::debug!("[context-assembly] loaded {} profile facets", facets.len());
            profile::render_profile_context(&facets)
        }
        Ok(_) => String::new(),
        Err(e) => {
            tracing::debug!("[context-assembly] profile load failed: {e}");
            String::new()
        }
    }
}

/// Assemble the full bootstrap context for an orchestrator turn.
pub async fn assemble_orchestrator_context(
    config: &Config,
    memory: Arc<dyn Memory>,
    user_message: &str,
) -> BootstrapContext {
    let prompts_dir = config.workspace_dir.join("agent").join("prompts");

    let archetype_prompt = load_archetype_prompt(&prompts_dir, "ORCHESTRATOR.md").await;
    let identity_context = load_identity_context(&config.workspace_dir).await;

    let memory_context = build_memory_context(
        memory.as_ref(),
        user_message,
        config.agent.max_memory_context_chars,
    )
    .await;

    BootstrapContext {
        archetype_prompt,
        identity_context,
        workspace_summary: String::new(), // populated by workspace_state tool on demand
        memory_context,
        user_profile_context: load_user_profile_context(memory.as_ref()),
    }
}

/// Assemble context with direct UnifiedMemory access (includes profile).
pub async fn assemble_orchestrator_context_with_unified(
    config: &Config,
    memory: Arc<dyn Memory>,
    unified: &UnifiedMemory,
    user_message: &str,
) -> BootstrapContext {
    let mut ctx = assemble_orchestrator_context(config, memory, user_message).await;
    ctx.user_profile_context = load_user_profile_from_unified(unified);
    ctx
}
