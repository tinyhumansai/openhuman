//! LLM-based entity + importance extractor.
//!
//! Builds a (system, user) prompt asking for NER + an importance rating
//! in one structured-JSON response, hands the prompt to a
//! [`ChatProvider`], and parses the result into [`ExtractedEntities`].
//!
//! ## Why this lives here
//!
//! Phase 2 ships a regex extractor only. Semantic NER (Person/Org/Loc/…)
//! requires a model. Originally we used a small local LLM (Ollama default:
//! `qwen2.5:0.5b`) because openhuman already ran Ollama for embeddings.
//! After the cloud-default refactor, the same prompt now routes through
//! whichever backend the workspace selected — typically the OpenHuman
//! backend's `summarization-v1`. The extractor itself is unchanged below the
//! HTTP layer; only the transport moved.
//!
//! ## Span recovery
//!
//! LLMs are unreliable about character offsets. We re-find each returned
//! entity surface in the source text via `text.find(...)` to recover spans.
//! Entities whose surface form can't be located in the source text are
//! dropped with a warn log (this catches model hallucinations).
//!
//! ## Soft fallback
//!
//! If the chat call fails (provider unavailable, malformed JSON, …), we
//! log a warn and return [`ExtractedEntities::default()`]. The
//! [`super::CompositeExtractor`] already tolerates errors from individual
//! extractors; ingestion never blocks on LLM availability.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use super::types::{EntityKind, ExtractedEntities, ExtractedEntity, ExtractedTopic};
use super::EntityExtractor;
use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};

/// Output-token cap requested for an extraction call (`max_tokens` on the
/// wire). An extraction response is one small structured-JSON object
/// (an entities array + an importance float + a one-sentence reason), so a
/// few thousand tokens is generous. The cap exists less to bound generation
/// than to keep credit-metered providers (OpenRouter) from reserving the
/// model's *entire* output window during their balance pre-flight: with no
/// `max_tokens`, OpenRouter priced extraction against the full 64k+ window
/// and returned `402 Payment Required` to any BYO user whose balance couldn't
/// cover it, on every call (TAURI-RUST-C62). 8192 is well under any plausible
/// extraction output yet small enough to clear the pre-flight for a
/// low-balance account.
const EXTRACTION_MAX_OUTPUT_TOKENS: u32 = 8192;

// ── Configuration ────────────────────────────────────────────────────────

/// Configuration for [`LlmEntityExtractor`].
#[derive(Clone, Debug)]
pub struct LlmExtractorConfig {
    /// Model identifier the chat provider should target. For cloud this
    /// is e.g. `summarization-v1`; for local Ollama it's the Ollama tag
    /// (`qwen2.5:0.5b`). Threaded through to [`ChatPrompt`] so the
    /// provider can route to the right model.
    ///
    /// Stored on the extractor for diagnostic logging only — the actual
    /// model selection happens inside the [`ChatProvider`].
    pub model: String,
    /// Which entity kinds the LLM is allowed to emit. Anything outside this
    /// set is mapped to [`EntityKind::Misc`] or dropped depending on
    /// `strict_kinds`.
    pub allowed_kinds: Vec<EntityKind>,
    /// If true, drop entities whose declared kind isn't in `allowed_kinds`
    /// instead of falling back to [`EntityKind::Misc`].
    pub strict_kinds: bool,
    /// If true, the system prompt asks the model to also emit a
    /// `topics` array (free-form theme labels), and the response parser
    /// populates [`ExtractedEntities::topics`]. Default `false` — the
    /// extractor's primary job is named-entity extraction; topics are
    /// an opt-in side-channel for callers that need a thematic
    /// summary in the same call (e.g. running over a sealed summary's
    /// content). Adds prompt tokens and gives the model one more
    /// schema field to keep track of, so leave off unless needed.
    pub emit_topics: bool,
    /// Optional configured output language for natural-language values such
    /// as `importance_reason` and topic labels. JSON field names and enum
    /// values remain stable.
    pub output_language: Option<String>,
}

impl Default for LlmExtractorConfig {
    fn default() -> Self {
        Self {
            model: "qwen2.5:0.5b".to_string(),
            allowed_kinds: vec![
                EntityKind::Person,
                EntityKind::Organization,
                EntityKind::Location,
                EntityKind::Event,
                EntityKind::Product,
                EntityKind::Datetime,
                EntityKind::Technology,
                EntityKind::Artifact,
                EntityKind::Quantity,
            ],
            strict_kinds: false,
            emit_topics: false,
            output_language: None,
        }
    }
}

// ── Extractor ────────────────────────────────────────────────────────────

/// LLM-backed entity + importance extractor.
///
/// Holds an `Arc<dyn ChatProvider>` rather than a per-instance HTTP
/// client. The provider abstraction lets a single workspace choose
/// cloud vs local at runtime (see
/// [`crate::openhuman::memory::chat::build_chat_provider`]). Tests
/// can mock the provider to assert the prompt / parse behaviour without
/// a real Ollama or backend.
pub struct LlmEntityExtractor {
    cfg: LlmExtractorConfig,
    provider: Arc<dyn ChatProvider>,
}

impl LlmEntityExtractor {
    /// Build the extractor with the supplied chat provider. Infallible —
    /// the caller is responsible for provider construction.
    pub fn new(cfg: LlmExtractorConfig, provider: Arc<dyn ChatProvider>) -> Self {
        Self { cfg, provider }
    }

    /// Build the chat prompt sent to the provider for `text`.
    fn build_prompt(&self, text: &str) -> ChatPrompt {
        ChatPrompt {
            system: build_system_prompt(self.cfg.emit_topics, self.cfg.output_language.as_deref()),
            user: format!("Text:\n{text}\n\nReturn JSON only."),
            temperature: 0.0,
            kind: "memory_tree::extract",
            max_tokens: Some(EXTRACTION_MAX_OUTPUT_TOKENS),
        }
    }
}

#[async_trait]
impl EntityExtractor for LlmEntityExtractor {
    fn name(&self) -> &'static str {
        "llm-ollama"
    }

    async fn extract(&self, text: &str) -> anyhow::Result<ExtractedEntities> {
        // Soft-fallback contract: every failure path (transport, HTTP status,
        // JSON parse) is logged as a warn and returns an empty
        // `ExtractedEntities` rather than `Err`. This makes the extractor
        // safe to call from any context, not just `score_chunk` (which
        // separately catches errors from its own extractor chain).
        //
        // Transport failures get bounded retry-with-backoff before falling
        // back to empty — see [`Self::try_extract`]. Non-transport failures
        // (HTTP non-success, malformed JSON) fall back immediately because
        // retrying the same input would yield the same bad response.
        const MAX_ATTEMPTS: u32 = 3;
        const BASE_BACKOFF_MS: u64 = 250;

        for attempt in 0..MAX_ATTEMPTS {
            match self.try_extract(text).await {
                AttemptOutcome::Done(extracted) => {
                    // #3365 + #002 (T013): the structure-degraded latch means
                    // "the extraction model is timing out / unreachable" — set
                    // ONLY after MAX_ATTEMPTS transport failures (below). A
                    // *completed* call disproves that: `try_extract` returns
                    // `Done` whenever the provider responded, including the
                    // valid-but-empty and malformed-JSON-as-empty cases. So a
                    // completed extraction clears the latch regardless of whether
                    // this particular text yielded entities — liveness, not
                    // content, is what it tracks.
                    //
                    // (Clearing only on a non-empty result left the latch stuck
                    // after the model recovered whenever later texts were
                    // entity-light, so the surface kept warning "extraction model
                    // is timing out" long after it had stopped — #3365.)
                    crate::openhuman::memory_tree::health::clear_structure_degraded();
                    return Ok(extracted);
                }
                AttemptOutcome::Permanent(code) => {
                    // Non-retryable client error — e.g. a 402 (the BYO
                    // provider account is out of credits), a rejected API
                    // key, or a model that no longer exists. Retrying
                    // reproduces the identical error and, on a credit-metered
                    // provider, fires one more Sentry event per attempt
                    // (TAURI-RUST-C62: 12k events from a single user). Skip
                    // the backoff loop entirely and fall back to the same
                    // degraded-but-empty result the exhausted-retry path
                    // returns — but tag the cause precisely so doctor/status
                    // can steer the user ("top up credits" / "fix the key")
                    // instead of "extraction is timing out".
                    log::warn!(
                        "[memory_tree::extract::llm] non-retryable provider failure ({}) — \
                         returning empty extraction without retry (structure degraded)",
                        code.as_str()
                    );
                    crate::openhuman::memory_tree::health::mark_structure_degraded(code);
                    return Ok(ExtractedEntities::default());
                }
                AttemptOutcome::Retryable => {
                    // Transport / transient failure. Retry with exponential
                    // backoff unless we've exhausted attempts.
                    if attempt + 1 < MAX_ATTEMPTS {
                        let delay_ms = BASE_BACKOFF_MS * 2u64.pow(attempt);
                        log::warn!(
                            "[memory_tree::extract::llm] transport failure, retrying in \
                             {delay_ms}ms (attempt {}/{})",
                            attempt + 2,
                            MAX_ATTEMPTS
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }

        // #002 (T013): every attempt hit a transport failure (the model
        // timed out / was unreachable). The soft-fallback contract still
        // returns empty (ingest never blocks on a slow model), but we now
        // record a structure-degraded signal with a typed cause so the
        // status/doctor surface can say "extraction is timing out — switch
        // the Memory extraction model" instead of presenting an empty wiki
        // as success.
        log::warn!(
            "[memory_tree::extract::llm] transport failed after {} attempts — \
             returning empty extraction (structure degraded)",
            MAX_ATTEMPTS
        );
        crate::openhuman::memory_tree::health::mark_structure_degraded(
            crate::openhuman::memory_tree::health::FailureCode::ExtractionTimeout,
        );
        Ok(ExtractedEntities::default())
    }
}

/// Outcome of a single [`LlmEntityExtractor::try_extract`] attempt.
///
/// Splits the old `Option` ("`None` = retry") into three cases so the retry
/// loop can tell a transient blip apart from a permanent client error and
/// stop hammering the latter (TAURI-RUST-C62).
enum AttemptOutcome {
    /// The call completed (provider returned content). Includes the
    /// "malformed wrong-shape JSON → empty" case, because re-sending the
    /// same input won't change the response.
    Done(ExtractedEntities),
    /// Transport / transient failure (unreachable backend, 5xx, truncated
    /// mid-JSON). Retrying may help.
    Retryable,
    /// Permanent, non-retryable provider failure (4xx client error: 402 out
    /// of credits, rejected key, model gone). Retrying reproduces the same
    /// error. Carries the typed [`FailureCode`] to record on the degraded
    /// signal so doctor/status can steer the user.
    Permanent(crate::openhuman::memory_tree::health::FailureCode),
}

impl LlmEntityExtractor {
    /// Internal: one attempt at calling the chat provider.
    async fn try_extract(&self, text: &str) -> AttemptOutcome {
        let prompt = self.build_prompt(text);
        log::debug!(
            "[memory_tree::extract::llm] chat provider={} model={} text_chars={}",
            self.provider.name(),
            self.cfg.model,
            text.chars().count()
        );

        let raw = match self.provider.chat_for_json(&prompt).await {
            Ok(v) => v,
            Err(e) => {
                // Classify against the provider stack's single source of
                // truth. A non-retryable client error (402/401/403/404/…)
                // must not be retried — that is the 12k-event amplifier in
                // TAURI-RUST-C62. Only genuine transport/transient failures
                // earn the backoff loop.
                if crate::openhuman::inference::provider::reliable::is_non_retryable(&e) {
                    log::warn!(
                        "[memory_tree::extract::llm] chat provider={} permanently failed: {e:#}",
                        self.provider.name()
                    );
                    return AttemptOutcome::Permanent(permanent_failure_code(&e));
                }
                log::warn!(
                    "[memory_tree::extract::llm] chat provider={} failed: {e:#}",
                    self.provider.name()
                );
                return AttemptOutcome::Retryable;
            }
        };
        log::debug!(
            "[memory_tree::extract::llm] response chars={} provider={}",
            raw.len(),
            self.provider.name()
        );

        let parsed: LlmExtractionOutput = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) if e.is_eof() => {
                // Truncated mid-JSON: the response stream closed before the
                // closing brace (token budget or a server-side timeout cut
                // it short). Unlike a wrong-shape body, this is transient —
                // signal a retryable failure so `extract`'s backoff loop
                // tries again rather than silently dropping the whole chunk
                // (bug-report-2026-05-26 I1).
                log::warn!(
                    "[memory_tree::extract::llm] LLM response truncated mid-JSON ({e}); \
                     response_bytes={} — retrying",
                    raw.len()
                );
                return AttemptOutcome::Retryable;
            }
            Err(e) => {
                log::warn!(
                    "[memory_tree::extract::llm] LLM returned non-JSON or wrong-shape \
                     response: {e}; content was: {} — returning empty extraction",
                    truncate_for_log(&raw, 400)
                );
                return AttemptOutcome::Done(ExtractedEntities::default());
            }
        };

        AttemptOutcome::Done(parsed.into_extracted_entities(text, &self.cfg))
    }
}

/// Map a permanent (non-retryable) extraction provider error to the most
/// honest *existing* [`FailureCode`], so the degraded-structure signal steers
/// the user without minting a new variant (and its 14-locale remediation key).
///
/// - 402 / "payment required" / "requires more credits" / "insufficient" →
///   [`FailureCode::BudgetExhausted`] (its remediation already reads "out of
///   budget; bring your own key or top up — retrying won't help"), the
///   dominant TAURI-RUST-C62 case.
/// - 401 / 403 / auth-key rejection → [`FailureCode::AuthInvalid`].
/// - any other permanent failure → [`FailureCode::ExtractionTimeout`], the
///   existing "extraction model isn't producing" bucket.
fn permanent_failure_code(
    err: &anyhow::Error,
) -> crate::openhuman::memory_tree::health::FailureCode {
    use crate::openhuman::memory_tree::health::FailureCode;
    let lower = format!("{err:#}").to_lowercase();
    if lower.contains("402")
        || lower.contains("payment required")
        || lower.contains("requires more credits")
        || lower.contains("insufficient")
        // Monthly-quota exhaustion (e.g. Kiro `MONTHLY_REQUEST_COUNT`) — the
        // budget-exhausted remediation ("out of budget; top up — retrying won't
        // help") is the honest signal even when the proxy omits a 402 status
        // (TAURI-RUST-C9A).
        || lower.contains("monthly_request_count")
        || lower.contains("monthly request")
        || lower.contains("quota")
    {
        FailureCode::BudgetExhausted
    } else if lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("invalid api key")
        || lower.contains("incorrect api key")
    {
        FailureCode::AuthInvalid
    } else {
        FailureCode::ExtractionTimeout
    }
}

// ── Prompt ───────────────────────────────────────────────────────────────

/// Build the system prompt for the extractor. When `emit_topics` is true
/// the schema, required-fields list, and example outputs include a
/// `topics` array (free-form theme labels). When false the prompt
/// matches the pre-flag behaviour exactly — no mention of topics
/// anywhere — so the small model isn't asked to produce a field the
/// caller doesn't want.
fn build_system_prompt(emit_topics: bool, output_language: Option<&str>) -> String {
    let topics_schema_line = if emit_topics {
        "  \"topics\": [\"<short theme label>\"],\n"
    } else {
        ""
    };
    let topics_required = if emit_topics { "topics, " } else { "" };
    let fields_count = if emit_topics { "four" } else { "three" };
    let topics_guide = if emit_topics {
        "Topics are short free-form theme labels for what the text is ABOUT \
         (e.g. \"rate limiting\", \"memory tree\", \"auth flow\"). They are \
         distinct from entities — entities are specific named things mentioned \
         in the text; topics are the abstract themes those things relate to.\n"
    } else {
        ""
    };
    let example1_topics = if emit_topics {
        ",\"topics\":[\"shipping\",\"auth\"]"
    } else {
        ""
    };
    let example2_topics = if emit_topics {
        ",\"topics\":[\"product launch\",\"revenue\"]"
    } else {
        ""
    };
    let language_directive = crate::openhuman::config::output_language_directive(output_language)
        .map(|directive| format!("{directive}\n\n"))
        .unwrap_or_default();

    format!(
        "{language_directive}You are a named-entity extractor and importance rater. Return JSON only — \
no prose, no markdown, no commentary. Do not summarize. Extract every named \
entity mention you find, including duplicates, and rate the chunk's overall \
importance as a float in [0.0, 1.0].

Schema:
{{
  \"entities\": [
    {{ \"kind\": \"person|organization|location|event|product|datetime|technology|artifact|quantity\",
      \"text\": \"<exact surface form as it appears in the text>\" }}
  ],
{topics_schema_line}  \"importance\": 0.0,
  \"importance_reason\": \"<one short sentence explaining the rating>\"
}}

Kinds guide:
  person       named human                            (\"Alice\", \"Steven Enamakel\")
  organization company / team / project               (\"Anthropic\", \"TinyHumans\")
  location     place                                  (\"SF office\", \"London\")
  event        scheduled occurrence                   (\"Q2 launch\", \"design review\")
  product      commercial offering                    (\"Claude Code\", \"OpenHuman\")
  datetime     temporal expression                    (\"Friday\", \"Q2 2026\", \"EOD tomorrow\")
  technology   tool / framework / language / service  (\"Rust\", \"OAuth\", \"Slack API\")
  artifact     code / ticket / doc reference          (\"PR #934\", \"src/foo.rs\", \"OH-42\")
  quantity     amount / metric / money                (\"$5K\", \"20/min\", \"10k tokens\")

{topics_guide} 
If a mention doesn't clearly fit a kind above, omit it rather than guessing.
Always emit ALL {fields_count} top-level fields (entities, {topics_required}importance, importance_reason),
even when entities is empty.

Examples:

Input: alice and bob shipped the auth migration friday. PR #42 ships OAuth refactor in src/auth/.
Output: {{\"entities\":[{{\"kind\":\"person\",\"text\":\"alice\"}},{{\"kind\":\"person\",\"text\":\"bob\"}},{{\"kind\":\"event\",\"text\":\"auth migration\"}},{{\"kind\":\"datetime\",\"text\":\"friday\"}},{{\"kind\":\"artifact\",\"text\":\"PR #42\"}},{{\"kind\":\"technology\",\"text\":\"OAuth\"}},{{\"kind\":\"artifact\",\"text\":\"src/auth/\"}}]{example1_topics},\"importance\":0.9,\"importance_reason\":\"explicit shipping commitment\"}}

Input: Anthropic shipped Claude Code in SF — $20M ARR target by Q2.
Output: {{\"entities\":[{{\"kind\":\"organization\",\"text\":\"Anthropic\"}},{{\"kind\":\"product\",\"text\":\"Claude Code\"}},{{\"kind\":\"location\",\"text\":\"SF\"}},{{\"kind\":\"quantity\",\"text\":\"$20M ARR\"}},{{\"kind\":\"datetime\",\"text\":\"Q2\"}}]{example2_topics},\"importance\":0.85,\"importance_reason\":\"factual content with key business metric\"}}

Importance guide:
  0.9+  actionable decisions, key information, explicit commitments
  0.6+  substantive discussion, factual content, named entities
  0.3+  ambient context, low-density prose
  <0.3  reactions, acknowledgments, bots, trivial exchanges
"
    )
}

// ── LLM JSON output ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LlmExtractionOutput {
    #[serde(default)]
    entities: Vec<LlmEntity>,
    /// Free-form theme labels — populated only when the extractor is
    /// configured with `emit_topics = true`. Always tolerant of absence
    /// so models that ignore the field don't fail parsing.
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    importance: Option<f32>,
    #[serde(default)]
    importance_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LlmEntity {
    kind: String,
    text: String,
}

impl LlmExtractionOutput {
    fn into_extracted_entities(
        self,
        source_text: &str,
        cfg: &LlmExtractorConfig,
    ) -> ExtractedEntities {
        let mut entities = Vec::with_capacity(self.entities.len());

        // Per-surface search cursor (char offset). When the LLM returns the
        // same surface text twice (deliberately — the prompt asks for
        // duplicates), we resume searching AFTER the previous occurrence so
        // each emitted entity points at a distinct span. Byte indices are
        // tracked separately from char indices because `str::find` returns
        // byte offsets while the rest of the pipeline uses char spans.
        use std::collections::HashMap;
        let mut cursors: HashMap<String, (usize /*byte*/, u32 /*char*/)> = HashMap::new();

        for raw in self.entities {
            let surface = raw.text.trim();
            if surface.is_empty() {
                continue;
            }

            let kind = match parse_kind(&raw.kind) {
                Some(k) => {
                    if cfg.allowed_kinds.contains(&k) {
                        k
                    } else if cfg.strict_kinds {
                        log::debug!(
                            "[memory_tree::extract::llm] dropping entity with disallowed kind: {}",
                            raw.kind
                        );
                        continue;
                    } else {
                        EntityKind::Misc
                    }
                }
                None => {
                    if cfg.strict_kinds {
                        log::debug!(
                            "[memory_tree::extract::llm] dropping entity with unknown kind: {}",
                            raw.kind
                        );
                        continue;
                    }
                    EntityKind::Misc
                }
            };

            // Recover spans by string search, advancing the cursor for this
            // surface so repeated mentions get distinct spans. If the model
            // hallucinated a surface (or we've exhausted all of its
            // occurrences), drop the entity.
            let (byte_from, char_from) = cursors.get(surface).copied().unwrap_or((0, 0));
            let (span_start, span_end, byte_after) =
                match find_char_span_from(source_text, surface, byte_from, char_from) {
                    Some(s) => s,
                    None => {
                        log::debug!(
                            "[memory_tree::extract::llm] dropping hallucinated or exhausted \
                             entity (not found beyond cursor): {surface:?}"
                        );
                        continue;
                    }
                };
            cursors.insert(surface.to_string(), (byte_after, span_end));

            entities.push(ExtractedEntity {
                kind,
                text: surface.to_string(),
                span_start,
                span_end,
                score: 0.85, // LLM-derived; lower confidence than regex
            });
        }

        let llm_importance = self.importance.map(|v| v.clamp(0.0, 1.0));

        // Topics: only populated when the caller enabled `emit_topics`
        // (the prompt asked for them). Otherwise this is empty by
        // default — the model didn't know to emit topics, so any value
        // here would be hallucination.
        let topics = self
            .topics
            .into_iter()
            .filter_map(|raw| {
                let label = raw.trim().to_string();
                if label.is_empty() {
                    None
                } else {
                    Some(ExtractedTopic { label, score: 0.85 })
                }
            })
            .collect();

        ExtractedEntities {
            entities,
            topics,
            llm_importance,
            llm_importance_reason: self.importance_reason,
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn parse_kind(s: &str) -> Option<EntityKind> {
    match s.trim().to_lowercase().as_str() {
        "person" | "people" => Some(EntityKind::Person),
        "organization" | "organisation" | "org" => Some(EntityKind::Organization),
        "location" | "place" | "loc" => Some(EntityKind::Location),
        "event" => Some(EntityKind::Event),
        "product" => Some(EntityKind::Product),
        "datetime" | "date" | "time" | "timestamp" => Some(EntityKind::Datetime),
        "technology" | "tech" | "tool" | "framework" | "library" | "language" | "service" => {
            Some(EntityKind::Technology)
        }
        "artifact" | "reference" | "ref" | "pr" | "ticket" | "file" | "commit" => {
            Some(EntityKind::Artifact)
        }
        "quantity" | "amount" | "metric" | "number" | "money" => Some(EntityKind::Quantity),
        "misc" | "miscellaneous" | "other" => Some(EntityKind::Misc),
        _ => None,
    }
}

/// Find `needle` in `haystack` and return its `(char_start, char_end)`.
///
/// Uses byte-level `find` then translates to char offsets so spans align
/// with the rest of the extractor pipeline (which is char-based).
fn find_char_span(haystack: &str, needle: &str) -> Option<(u32, u32)> {
    find_char_span_from(haystack, needle, 0, 0).map(|(s, e, _)| (s, e))
}

/// Find `needle` in `haystack` starting from `byte_from` and return
/// `(char_start, char_end, byte_after_needle)`.
///
/// The byte-offset return is so the caller can chain successive searches
/// without re-walking the prefix every time: pass the returned
/// `byte_after_needle` as the next call's `byte_from`.
///
/// `char_from` must correspond to `byte_from` in the same `haystack` —
/// i.e. `haystack[..byte_from].chars().count() == char_from as usize`.
/// The caller maintains this invariant (cheap: it's the return from the
/// previous call).
fn find_char_span_from(
    haystack: &str,
    needle: &str,
    byte_from: usize,
    char_from: u32,
) -> Option<(u32, u32, usize)> {
    if needle.is_empty() || byte_from > haystack.len() {
        return None;
    }
    // Guard against `byte_from` landing inside a multi-byte UTF-8 sequence.
    if !haystack.is_char_boundary(byte_from) {
        return None;
    }
    let rel = haystack[byte_from..].find(needle)?;
    let byte_start = byte_from + rel;
    let byte_end = byte_start + needle.len();
    // Walk forward from the previous char position to build the new char
    // offset — avoids re-walking the full prefix.
    let char_start = char_from + haystack[byte_from..byte_start].chars().count() as u32;
    let char_end = char_start + needle.chars().count() as u32;
    Some((char_start, char_end, byte_end))
}

fn truncate_for_log(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
