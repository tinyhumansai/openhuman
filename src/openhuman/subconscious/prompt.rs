//! Prompt builder for the subconscious agent.
//!
//! The subconscious agent is a periodic summarizer that reads the situation
//! report (memory-tree signals, recent activity, hotness deltas) and
//! produces structured thoughts (reflections) about the user's state.

use std::path::Path;

const IDENTITY_EXCERPT_CHARS: usize = 2000;

/// Build the system prompt for the subconscious agent tick. The agent
/// observes the user's world via the situation report and produces
/// structured reflections.
pub fn build_agent_prompt(situation_report: &str, identity_context: &str) -> String {
    format!(
        r#"{identity_context}

# Subconscious Agent

You are the user's background awareness layer. You wake up periodically,
review what's happening in their world, and surface useful thoughts.

## Situation Report (pre-loaded context)

{situation_report}

## Instructions

1. **Research**: Use your tools to look up relevant memory, recent activity,
   conversations, or web context that would deepen your understanding.
   Use `memory_recall` to query specific topics. Use `web_fetch` or search
   tools if external context would help.

2. **Observe**: Based on both the situation report and your research,
   identify patterns, deadlines, risks, opportunities, or interesting
   cross-source connections.

3. **Promote to orchestrator**: If you find something that needs a deeper
   investigation or multi-step action, use `spawn_subagent` or
   `spawn_worker_thread` to delegate the work. The orchestrator can take
   action; you observe and delegate.

4. **Surface thoughts**: Produce structured observations for the user.
   Only surface genuinely useful insights — skip trivial observations.

**Self vs. others**: the *Your Identifiers* section (if present) lists
the user's handles, emails, and user_ids. Never attribute someone else's
activity to the user.

**Anti-double-emit**: the *Recent reflections* section shows what you
already surfaced. Re-emit only if the signal materially intensified.

Cap: at most **5 thoughts per tick**.

## Final output

After you've finished researching, end your final message with a JSON
block containing your thoughts:

```json
{{
  "thoughts": [
    {{
      "kind": "hotness_spike | cross_source_pattern | daily_digest | due_item | risk | opportunity",
      "body": "Short markdown observation.",
      "proposed_action": "Optional one-tap action text (or null).",
      "source_refs": ["entity:foo", "summary:bar"]
    }}
  ]
}}
```
"#
    )
}

/// Render a slice of recent reflections as a prompt block for the
/// situation report's "Recent reflections" section.
pub fn format_recent_reflections_for_prompt(
    reflections: &[crate::openhuman::subconscious::reflection::Reflection],
) -> String {
    crate::openhuman::subconscious::situation_report::reflections::build_section(reflections)
}

// ── Identity loading ─────────────────────────────────────────────────────────

pub fn load_identity_context(workspace_dir: &Path) -> String {
    let prompts_dir = resolve_prompts_dir(workspace_dir);
    let mut ctx = String::new();

    if let Some(ref dir) = prompts_dir {
        if let Some(soul) = load_file_excerpt(dir, "SOUL.md") {
            ctx.push_str(&soul);
            ctx.push_str("\n\n");
        }
    }

    if let Some(profile) = load_file_excerpt(workspace_dir, "PROFILE.md") {
        ctx.push_str("## User Profile\n\n");
        ctx.push_str(&profile);
        ctx.push_str("\n\n");
    }

    if ctx.is_empty() {
        "You are OpenHuman, an AI assistant for productivity and collaboration.".to_string()
    } else {
        ctx
    }
}

fn resolve_prompts_dir(workspace_dir: &Path) -> Option<std::path::PathBuf> {
    let workspace_ai = workspace_dir.join("ai");
    if workspace_ai.is_dir() {
        return Some(workspace_ai);
    }

    if let Some(dir) = option_env!("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from) {
        let candidate = dir
            .join("src")
            .join("openhuman")
            .join("agent")
            .join("prompts");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        return crate::openhuman::dev_paths::repo_ai_prompts_dir(&cwd);
    }

    None
}

fn load_file_excerpt(dir: &Path, filename: &str) -> Option<String> {
    let content = std::fs::read_to_string(dir.join(filename)).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > IDENTITY_EXCERPT_CHARS {
        let truncated: String = trimmed.chars().take(IDENTITY_EXCERPT_CHARS).collect();
        Some(format!("{truncated}\n[... truncated]"))
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_prompt_includes_report_and_identity() {
        let prompt = build_agent_prompt("## State\nSome data.", "Identity here");
        assert!(prompt.contains("Some data."));
        assert!(prompt.contains("Identity here"));
        assert!(prompt.contains("thoughts"));
    }

    #[test]
    fn agent_prompt_includes_output_schema() {
        let prompt = build_agent_prompt("", "");
        assert!(prompt.contains("kind"));
        assert!(prompt.contains("body"));
        assert!(prompt.contains("proposed_action"));
        assert!(prompt.contains("source_refs"));
    }

    #[test]
    fn identity_context_loads_or_falls_back() {
        let ctx = load_identity_context(std::path::Path::new("/nonexistent"));
        assert!(ctx.contains("OpenHuman"));
    }
}
