//! `context.md`: the compiled memory brief, and its injection.
//!
//! [`refresh`] compiles a brief from the bound engine with
//! `tinymemory-context` (four default briefs plus recent learnings, trimmed to
//! `[memory.context] budget_tokens`) and writes it to
//! `<workspace>/memory/context.md`, with its timestamp and token count in
//! `context_state.json` beside it. The `memory_context_refresh` cron job calls
//! it every `interval_mins`; `memory_context_refresh` calls it on demand.
//!
//! [`injection_block`] is what the session host prepends to the first user
//! message of a **new** session, wrapped in `<memory-context>…</memory-context>`.
//! A resumed session never gets it again: its transcript is frozen.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tinymemory::context::{Brief, ContextCompiler, ContextSpec, DEFAULT_LEARNINGS_LIMIT};

use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};
use super::types::{ContextSetParams, ContextView};

/// Fewest minutes between scheduled recompiles.
pub const MIN_INTERVAL_MINS: u32 = 5;

/// Smallest token budget.
pub const MIN_BUDGET_TOKENS: u32 = 100;

/// Largest token budget.
pub const MAX_BUDGET_TOKENS: u32 = 32_000;

/// Opening tag of the injected block.
pub const OPEN_TAG: &str = "<memory-context>";

/// Closing tag of the injected block.
pub const CLOSE_TAG: &str = "</memory-context>";

#[derive(Debug, Default, Serialize, Deserialize)]
struct ContextState {
    generated_at: Option<DateTime<Utc>>,
    tokens: usize,
}

fn memory_dir(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("memory")
}

/// Where `context.md` lives.
#[must_use]
pub fn context_path(workspace_dir: &Path) -> PathBuf {
    memory_dir(workspace_dir).join("context.md")
}

fn state_path(workspace_dir: &Path) -> PathBuf {
    memory_dir(workspace_dir).join("context_state.json")
}

fn read_state(workspace_dir: &Path) -> ContextState {
    std::fs::read_to_string(state_path(workspace_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn read_markdown(workspace_dir: &Path) -> String {
    std::fs::read_to_string(context_path(workspace_dir)).unwrap_or_default()
}

/// `memory_context_get`.
#[must_use]
pub fn view(config: &Config) -> ContextView {
    let state = read_state(&config.workspace_dir);
    let settings = &config.memory.context;
    ContextView {
        markdown: read_markdown(&config.workspace_dir),
        tokens: state.tokens,
        generated_at: state.generated_at,
        interval_mins: settings.interval_mins,
        budget_tokens: settings.budget_tokens,
        enabled: settings.enabled,
    }
}

/// The spec compiled for `config`.
#[must_use]
pub fn spec_for(config: &Config) -> ContextSpec {
    ContextSpec {
        budget_tokens: config.memory.context.budget_tokens.max(MIN_BUDGET_TOKENS) as usize,
        briefs: Brief::defaults(),
        learnings_limit: DEFAULT_LEARNINGS_LIMIT,
    }
}

/// Compiles `context.md` from the bound engine and writes it.
pub async fn refresh(config: &Config) -> MemoryResult<ContextView> {
    let bound = engine::resolve(config).engine()?;
    refresh_with(config, &*bound.engine, ContextCompiler::new()).await
}

/// [`refresh`] against an explicit engine and compiler.
pub async fn refresh_with(
    config: &Config,
    engine: &dyn tinymemory::MemoryEngine,
    compiler: ContextCompiler,
) -> MemoryResult<ContextView> {
    let doc = compiler
        .compile(engine, &spec_for(config))
        .await
        .map_err(|error| MemoryError::invalid(error.to_string()))?;
    let dir = memory_dir(&config.workspace_dir);
    let state = ContextState {
        generated_at: Some(doc.generated_at),
        tokens: doc.tokens,
    };
    let write = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(context_path(&config.workspace_dir), &doc.markdown))
        .and_then(|()| {
            let json = serde_json::to_vec_pretty(&state).map_err(std::io::Error::other)?;
            std::fs::write(state_path(&config.workspace_dir), json)
        });
    write.map_err(|error| MemoryError::Engine(format!("writing context.md failed: {error}")))?;
    tracing::info!(
        engine = %doc.engine,
        tokens = doc.tokens,
        refs = doc.refs.len(),
        "[memory:context] context.md compiled"
    );
    Ok(view(config))
}

/// Applies `memory_context_set` to `config`; the caller persists it.
pub fn apply_set(config: &mut Config, params: &ContextSetParams) -> MemoryResult<()> {
    let settings = &mut config.memory.context;
    if let Some(interval) = params.interval_mins {
        if interval < MIN_INTERVAL_MINS {
            return Err(MemoryError::invalid(format!(
                "interval_mins must be at least {MIN_INTERVAL_MINS}"
            )));
        }
        settings.interval_mins = interval;
    }
    if let Some(budget) = params.budget_tokens {
        if !(MIN_BUDGET_TOKENS..=MAX_BUDGET_TOKENS).contains(&budget) {
            return Err(MemoryError::invalid(format!(
                "budget_tokens must be between {MIN_BUDGET_TOKENS} and {MAX_BUDGET_TOKENS}"
            )));
        }
        settings.budget_tokens = budget;
    }
    if let Some(enabled) = params.enabled {
        settings.enabled = enabled;
    }
    Ok(())
}

/// Strips a leading `---` frontmatter block.
fn strip_frontmatter(markdown: &str) -> &str {
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return markdown;
    };
    match rest.find("\n---\n") {
        Some(end) => &rest[end + "\n---\n".len()..],
        None => markdown,
    }
}

/// The block a new session's first user message is prefixed with, or `None`
/// when context is disabled, memory is off, or there is nothing compiled.
#[must_use]
pub fn injection_block(config: &Config) -> Option<String> {
    if !config.memory.context.enabled {
        return None;
    }
    if !engine::is_on(config) {
        tracing::debug!("[memory:context] memory off; no context injected");
        return None;
    }
    let markdown = read_markdown(&config.workspace_dir);
    let body = strip_frontmatter(&markdown).trim();
    if body.is_empty() {
        return None;
    }
    tracing::debug!(chars = body.len(), "[memory:context] context.md injected");
    Some(format!("{OPEN_TAG}\n{body}\n{CLOSE_TAG}"))
}

/// Prepends [`injection_block`] (when there is one) to a new session's first
/// user message.
#[must_use]
pub fn prepend_to_first_message(config: &Config, message: &str) -> String {
    match injection_block(config) {
        Some(block) => format!("{block}\n\n{message}"),
        None => message.to_string(),
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
