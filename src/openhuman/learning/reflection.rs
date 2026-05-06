//! Post-turn reflection engine.
//!
//! After each qualifying turn, builds a reflection prompt, sends it to the
//! configured LLM (local Ollama or cloud reasoning model), parses structured
//! JSON output, and stores observations in memory.

use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::config::{Config, LearningConfig, ReflectionSource};
use crate::openhuman::memory::{Memory, MemoryCategory};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Memory namespace + custom-category tag for explicit user reflections.
///
/// Distinct from `learning_observations` (agent-extracted) and
/// `user_profile` (preference facts) — these are sentences the user
/// authored about themselves that should steer future agent behaviour.
pub const REFLECTIONS_NAMESPACE: &str = "learning_reflections";

/// Structured output expected from the reflection LLM call.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReflectionOutput {
    #[serde(default)]
    pub observations: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub user_preferences: Vec<String>,
    /// Explicit user reflections lifted out of the conversation — the
    /// user's own intentional self-statements ("I realized…", "going
    /// forward…", "remember that I…"). Stored as a distinct memory
    /// class and rendered in the prompt above generic tree summaries.
    #[serde(default)]
    pub user_reflections: Vec<String>,
}

/// Post-turn hook that reflects on completed turns and stores observations.
pub struct ReflectionHook {
    config: LearningConfig,
    full_config: Arc<Config>,
    memory: Arc<dyn Memory>,
    provider: Option<Arc<dyn crate::openhuman::providers::Provider>>,
    /// Per-session reflection counts for throttling. Key is session_id (or "__global__").
    session_counts: Mutex<HashMap<String, usize>>,
}

impl ReflectionHook {
    pub fn new(
        config: LearningConfig,
        full_config: Arc<Config>,
        memory: Arc<dyn Memory>,
        provider: Option<Arc<dyn crate::openhuman::providers::Provider>>,
    ) -> Self {
        Self {
            config,
            full_config,
            memory,
            provider,
            session_counts: Mutex::new(HashMap::new()),
        }
    }

    fn session_key(ctx: &TurnContext) -> String {
        ctx.session_id
            .clone()
            .unwrap_or_else(|| "__global__".to_string())
    }

    /// Attempt to increment the session counter. Returns true if under the limit.
    fn try_increment(&self, session_key: &str) -> bool {
        let mut counts = self.session_counts.lock();
        let count = counts.entry(session_key.to_string()).or_insert(0);
        if *count >= self.config.max_reflections_per_session {
            log::debug!(
                "[learning] reflection throttled for session {session_key}: {count} >= {}",
                self.config.max_reflections_per_session
            );
            return false;
        }
        *count += 1;
        true
    }

    /// Rollback the session counter (e.g. on reflection failure).
    fn rollback_increment(&self, session_key: &str) {
        let mut counts = self.session_counts.lock();
        if let Some(count) = counts.get_mut(session_key) {
            *count = count.saturating_sub(1);
        }
    }

    /// Check if this turn warrants reflection (complexity check only).
    fn should_reflect(&self, ctx: &TurnContext) -> bool {
        if !self.config.enabled || !self.config.reflection_enabled {
            return false;
        }

        // Check minimum complexity
        let tool_count = ctx.tool_calls.len();
        let response_long = ctx.assistant_response.chars().count() > 500;
        tool_count >= self.config.min_turn_complexity || response_long
    }

    /// Build the reflection prompt from turn context.
    fn build_reflection_prompt(&self, ctx: &TurnContext) -> String {
        let mut prompt = String::from(
            "Analyze this completed agent turn and extract learnings.\n\
             Return a JSON object with these fields:\n\
             - \"observations\": array of strings — what worked, what failed, notable patterns\n\
             - \"patterns\": array of strings — recurring patterns worth remembering\n\
             - \"user_preferences\": array of strings — any user preferences detected\n\
             - \"user_reflections\": array of strings — explicit reflections the user \
             made about themselves, their goals, what they want to do differently, \
             or what they want you to remember going forward. Only include statements \
             the user clearly authored as a reflection (\"I realized…\", \"remember that I…\", \
             \"going forward I want…\"). Leave empty if none.\n\n\
             Keep each entry concise (one sentence). Return ONLY valid JSON, no markdown.\n\n",
        );

        prompt.push_str(&format!(
            "## User Message\n{}\n\n",
            truncate(&ctx.user_message, 500)
        ));
        prompt.push_str(&format!(
            "## Assistant Response\n{}\n\n",
            truncate(&ctx.assistant_response, 500)
        ));

        if !ctx.tool_calls.is_empty() {
            prompt.push_str("## Tool Calls\n");
            for tc in &ctx.tool_calls {
                prompt.push_str(&format!(
                    "- {} (success={}, duration={}ms): {}\n",
                    tc.name,
                    tc.success,
                    tc.duration_ms,
                    truncate(&tc.output_summary, 100)
                ));
            }
            prompt.push('\n');
        }

        prompt.push_str(&format!(
            "Turn took {}ms across {} iteration(s).\n",
            ctx.turn_duration_ms, ctx.iteration_count
        ));

        prompt
    }

    /// Call the configured LLM for reflection.
    async fn run_reflection(&self, prompt: &str) -> anyhow::Result<String> {
        match self.config.reflection_source {
            ReflectionSource::Local => {
                // Gate: local reflection requires the per-feature flag.
                // When off, no-op silently rather than erroring the turn.
                // TODO: wire a cloud fallback here when use_local_for_learning is false.
                if !self.full_config.local_ai.use_local_for_learning() {
                    tracing::info!(
                        "[learning::reflection] local_ai.usage.learning_reflection not enabled — \
                         skipping local reflection (no cloud fallback configured for this subsystem)"
                    );
                    return Ok(String::new());
                }
                // Local reflection acquires the scheduler_gate LLM
                // permit transitively through `service.prompt` →
                // `inference_with_temperature_internal`. Cloud
                // reflection skips the gate (#1073 intentionally
                // gates only local routes; cloud rate limiting is
                // tracked separately).
                log::debug!(
                    "[learning::reflection] local route — gate permit acquired via LocalAiService"
                );
                let service = crate::openhuman::local_ai::global(&self.full_config);
                service
                    .prompt(&self.full_config, prompt, Some(512), true)
                    .await
                    .map_err(|e| anyhow::anyhow!("local reflection failed: {e}"))
            }
            ReflectionSource::Cloud => {
                let provider = self.provider.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("no cloud provider configured for reflection")
                })?;
                provider.simple_chat(prompt, "hint:reasoning", 0.3).await
            }
        }
    }

    /// Parse the LLM response into structured reflection output.
    fn parse_reflection(raw: &str) -> ReflectionOutput {
        // Try to extract JSON from the response (may have surrounding text)
        let trimmed = raw.trim();
        let json_str = if let Some(start) = trimmed.find('{') {
            if let Some(end) = trimmed.rfind('}') {
                &trimmed[start..=end]
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        serde_json::from_str(json_str).unwrap_or_else(|_| {
            log::debug!(
                "[learning] could not parse reflection JSON, using raw text as observation"
            );
            ReflectionOutput {
                observations: vec![trimmed.to_string()],
                patterns: Vec::new(),
                user_preferences: Vec::new(),
                user_reflections: Vec::new(),
            }
        })
    }

    /// Store reflection output in memory.
    async fn store_reflection(&self, output: &ReflectionOutput) -> anyhow::Result<()> {
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let hash = &uuid::Uuid::new_v4().to_string()[..8];

        if !output.observations.is_empty() {
            let content = output.observations.join("\n");
            let key = format!("obs/{date}/{hash}");
            self.memory
                .store(
                    "learning_observations",
                    &key,
                    &content,
                    MemoryCategory::Custom("learning_observations".into()),
                    None,
                )
                .await?;
            log::debug!(
                "[learning] stored {} observation(s) at {key}",
                output.observations.len()
            );
        }

        for pattern in &output.patterns {
            let slug = slugify(pattern);
            let key = format!("pat/{slug}");
            self.memory
                .store(
                    "learning_patterns",
                    &key,
                    pattern,
                    MemoryCategory::Custom("learning_patterns".into()),
                    None,
                )
                .await?;
        }

        // User preferences are handled by UserProfileHook, but store raw if present
        for pref in &output.user_preferences {
            let slug = slugify(pref);
            let key = format!("pref/{slug}");
            self.memory
                .store(
                    "user_profile",
                    &key,
                    pref,
                    MemoryCategory::Custom("user_profile".into()),
                    None,
                )
                .await?;
        }

        // Reflection persistence is handled by the caller
        // (`on_turn_complete`) so the heuristic fast-path and the LLM
        // path share a single per-turn dedupe set and never write the
        // same sentence twice.
        Ok(())
    }

    /// Persist a single reflection sentence into the dedicated namespace.
    /// Public to the crate so the heuristic fast-path can reuse the same
    /// storage shape without going through the LLM round-trip.
    pub(crate) async fn persist_reflection(&self, reflection: &str) -> anyhow::Result<()> {
        let trimmed = reflection.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let hash = &uuid::Uuid::new_v4().to_string()[..8];
        let key = format!("ref/{date}/{hash}");
        self.memory
            .store(
                REFLECTIONS_NAMESPACE,
                &key,
                trimmed,
                MemoryCategory::Custom(REFLECTIONS_NAMESPACE.into()),
                None,
            )
            .await?;
        log::debug!(
            "[learning] stored user reflection at {key} ({} chars)",
            trimmed.chars().count()
        );
        Ok(())
    }

    /// Persist a reflection sentence iff its normalised form has not
    /// already been seen in the current turn. `seen` is the per-turn
    /// dedupe set shared between the heuristic fast-path and the LLM
    /// `user_reflections` path, so a sentence captured by both routes
    /// only lands in memory once.
    async fn persist_reflection_deduped(
        &self,
        reflection: &str,
        seen: &mut std::collections::HashSet<String>,
    ) -> anyhow::Result<()> {
        let normalised = normalise_reflection(reflection);
        if normalised.is_empty() {
            return Ok(());
        }
        if !seen.insert(normalised) {
            log::debug!(
                "[learning] reflection already captured this turn — skipping duplicate write"
            );
            return Ok(());
        }
        self.persist_reflection(reflection).await
    }
}

/// Normalise a reflection sentence for per-turn dedupe comparisons:
/// trim outer whitespace and lower-case so casing or trailing
/// punctuation differences do not bypass the duplicate check.
fn normalise_reflection(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// Heuristic detector for explicit reflection cues in a user message.
///
/// Returns the trimmed sentences from `user_message` that match a known
/// reflection cue ("I realized", "going forward", "remember that I",
/// "I learned", "I want to", "I've decided"). Used as a fast-path so
/// reflections get captured even when the post-turn LLM reflection is
/// throttled, disabled, or routed to a slow cloud model.
///
/// The detector is intentionally conservative — false positives would
/// flood the privileged reflection namespace and dilute its signal.
pub fn extract_reflection_cues(user_message: &str) -> Vec<String> {
    const CUES: &[&str] = &[
        "i realized",
        "i realised",
        "i learned",
        "i learnt",
        "i've decided",
        "i have decided",
        "going forward",
        "from now on",
        "remember that i",
        "remember i",
        "please remember",
        "i want you to remember",
    ];

    let mut hits: Vec<String> = Vec::new();
    for sentence in split_sentences(user_message) {
        let lower = sentence.to_ascii_lowercase();
        if CUES.iter().any(|cue| lower.contains(cue)) {
            let trimmed = sentence.trim();
            if !trimmed.is_empty() && !hits.iter().any(|h| h == trimmed) {
                hits.push(trimmed.to_string());
            }
        }
    }
    hits
}

/// Split free text into sentence-shaped chunks on `.`, `!`, `?`, and
/// newlines. Cheap and good enough for cue detection — full NLP is
/// overkill for matching a known short cue list.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if matches!(ch, '.' | '!' | '?' | '\n') {
            if !buf.trim().is_empty() {
                out.push(buf.trim().to_string());
            }
            buf.clear();
        } else {
            buf.push(ch);
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

#[async_trait]
impl PostTurnHook for ReflectionHook {
    fn name(&self) -> &str {
        "reflection"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        // Per-turn dedupe set: shared between the heuristic fast-path
        // below and the LLM `user_reflections` persistence below, so
        // the same sentence captured by both routes only lands in
        // memory once and cannot crowd out unique reflections in the
        // bounded top-N retrieval window.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Fast-path heuristic capture — runs whenever the learning
        // subsystem is on, regardless of turn complexity, so single-turn
        // reflections like "remember that I prefer terse answers" are
        // promoted to the privileged reflection namespace without paying
        // for a reflection-LLM round-trip.
        if self.config.enabled {
            for cue in extract_reflection_cues(&ctx.user_message) {
                if let Err(e) = self.persist_reflection_deduped(&cue, &mut seen).await {
                    log::warn!("[learning] failed to persist heuristic reflection: {e}");
                }
            }
        }

        if !self.should_reflect(ctx) {
            return Ok(());
        }

        let session_key = Self::session_key(ctx);
        if !self.try_increment(&session_key) {
            return Ok(());
        }

        log::debug!("[learning] starting reflection for session={session_key}",);

        let prompt = self.build_reflection_prompt(ctx);
        let result = self.run_reflection(&prompt).await;

        let raw = match result {
            Ok(raw) => raw,
            Err(e) => {
                // Rollback the counter so failures don't consume quota
                self.rollback_increment(&session_key);
                return Err(e);
            }
        };

        // Empty response is the sentinel `run_reflection` uses when the
        // local-only `use_local_for_learning` gate is off. Don't burn quota
        // on an empty parse — clean-skip without storing a blank record.
        if raw.trim().is_empty() {
            self.rollback_increment(&session_key);
            log::debug!(
                "[learning] reflection skipped (empty response — gate off or local AI unavailable)"
            );
            return Ok(());
        }

        let output = Self::parse_reflection(&raw);

        log::info!(
            "[learning] reflection complete: observations={} patterns={} prefs={} user_reflections={}",
            output.observations.len(),
            output.patterns.len(),
            output.user_preferences.len(),
            output.user_reflections.len(),
        );

        if let Err(e) = self.store_reflection(&output).await {
            self.rollback_increment(&session_key);
            return Err(e);
        }

        // Persist LLM-extracted reflections through the shared dedupe
        // set so any sentence the heuristic already captured above is
        // not written twice. Failures here are logged but never roll
        // back the session counter — observations / patterns /
        // preferences from the same turn have already been committed
        // and the throttle quota is correctly accounted for.
        for reflection in &output.user_reflections {
            if let Err(e) = self.persist_reflection_deduped(reflection, &mut seen).await {
                log::warn!("[learning] failed to persist LLM-extracted reflection: {e}");
            }
        }

        Ok(())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

fn slugify(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' || c == '_' {
                Some('_')
            } else {
                None
            }
        })
        .take(40)
        .collect()
}

#[cfg(test)]
#[path = "reflection_tests.rs"]
mod tests;
