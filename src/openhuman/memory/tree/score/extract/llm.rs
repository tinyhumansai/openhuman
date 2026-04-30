//! LLM-based entity + importance extractor.
//!
//! Talks to an Ollama-compatible chat-completions HTTP endpoint, asks for
//! NER + an importance rating in one structured-JSON response, and parses
//! the result into [`ExtractedEntities`].
//!
//! ## Why this lives here
//!
//! Phase 2 ships a regex extractor only. Semantic NER (Person/Org/Loc/…)
//! requires a model. We use a small local LLM (Ollama default:
//! `qwen2.5:0.5b`) for two reasons:
//!
//! 1. **Reuse** — openhuman already runs Ollama for embeddings; no new
//!    deps, no native ONNX runtime to ship.
//! 2. **Free importance signal** — extending the NER prompt with one extra
//!    JSON field (`importance`) gives us an LLM-rated quality score per
//!    chunk for the cost of one prompt instead of two LLM calls.
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
//! If the HTTP call fails (Ollama not running, model not pulled, timeout),
//! we log a warn and return [`ExtractedEntities::default()`]. The
//! [`super::CompositeExtractor`] already tolerates errors from individual
//! extractors; ingestion never blocks on LLM availability.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::types::{EntityKind, ExtractedEntities, ExtractedEntity, ExtractedTopic};
use super::EntityExtractor;

// ── Configuration ────────────────────────────────────────────────────────

/// Configuration for [`LlmEntityExtractor`].
#[derive(Clone, Debug)]
pub struct LlmExtractorConfig {
    /// Base URL of the Ollama-compatible endpoint (e.g. `http://localhost:11434`).
    /// Do NOT include `/api/chat` — the extractor appends it.
    pub endpoint: String,
    /// Model identifier as known to the endpoint (e.g. `qwen2.5:0.5b`).
    pub model: String,
    /// Per-request timeout. The default is generous because the first
    /// request after a model swap may need to load weights.
    pub timeout: Duration,
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
}

impl Default for LlmExtractorConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".to_string(),
            model: "qwen2.5:0.5b".to_string(),
            timeout: Duration::from_secs(15),
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
        }
    }
}

// ── Extractor ────────────────────────────────────────────────────────────

/// LLM-backed entity + importance extractor.
pub struct LlmEntityExtractor {
    cfg: LlmExtractorConfig,
    http: Client,
}

impl LlmEntityExtractor {
    pub fn new(cfg: LlmExtractorConfig) -> anyhow::Result<Self> {
        let http = Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(anyhow::Error::from)?;
        Ok(Self { cfg, http })
    }

    /// Build the Ollama `/api/chat` request body.
    fn build_request(&self, text: &str) -> OllamaChatRequest {
        OllamaChatRequest {
            model: self.cfg.model.clone(),
            messages: vec![
                OllamaMessage {
                    role: "system".to_string(),
                    content: build_system_prompt(self.cfg.emit_topics),
                },
                OllamaMessage {
                    role: "user".to_string(),
                    content: format!("Text:\n{text}\n\nReturn JSON only."),
                },
            ],
            format: "json".to_string(),
            stream: false,
            options: OllamaOptions { temperature: 0.0 },
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
                Some(extracted) => return Ok(extracted),
                None => {
                    // Transport failure. Retry with exponential backoff
                    // unless we've exhausted attempts.
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

        log::warn!(
            "[memory_tree::extract::llm] transport failed after {} attempts — \
             returning empty extraction",
            MAX_ATTEMPTS
        );
        Ok(ExtractedEntities::default())
    }
}

impl LlmEntityExtractor {
    /// Internal: one attempt at calling Ollama.
    ///
    /// Returns:
    /// - `Some(extracted)` — call completed (HTTP returned). Includes the
    ///   "HTTP non-success" and "malformed JSON" cases, which return
    ///   `Some(empty)` because retrying the same input won't help.
    /// - `None` — transport-level failure (DNS, connect refused, timeout
    ///   before any HTTP response). Caller may retry.
    async fn try_extract(&self, text: &str) -> Option<ExtractedEntities> {
        let url = format!("{}/api/chat", self.cfg.endpoint.trim_end_matches('/'));
        let body = self.build_request(text);
        log::debug!(
            "[memory_tree::extract::llm] POST {url} model={} text_chars={}",
            self.cfg.model,
            text.chars().count()
        );

        let resp = match self.http.post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[memory_tree::extract::llm] transport failure to {url}: {e}");
                return None;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            log::warn!(
                "[memory_tree::extract::llm] ollama non-success status {status}: {} — \
                 returning empty extraction",
                truncate_for_log(&body, 200)
            );
            return Some(ExtractedEntities::default());
        }

        let envelope: OllamaChatResponse = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "[memory_tree::extract::llm] response body not Ollama-shaped JSON: {e} — \
                     returning empty extraction"
                );
                return Some(ExtractedEntities::default());
            }
        };
        log::debug!(
            "[memory_tree::extract::llm] response chars={}",
            envelope.message.content.len()
        );

        let parsed: LlmExtractionOutput = match serde_json::from_str(&envelope.message.content) {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "[memory_tree::extract::llm] LLM returned non-JSON or wrong-shape \
                     response: {e}; content was: {} — returning empty extraction",
                    truncate_for_log(&envelope.message.content, 400)
                );
                return Some(ExtractedEntities::default());
            }
        };

        Some(parsed.into_extracted_entities(text, &self.cfg))
    }
}

// ── Prompt ───────────────────────────────────────────────────────────────

/// Build the system prompt for the extractor. When `emit_topics` is true
/// the schema, required-fields list, and example outputs include a
/// `topics` array (free-form theme labels). When false the prompt
/// matches the pre-flag behaviour exactly — no mention of topics
/// anywhere — so the small model isn't asked to produce a field the
/// caller doesn't want.
fn build_system_prompt(emit_topics: bool) -> String {
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

    format!(
        "You are a named-entity extractor and importance rater. Return JSON only — \
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

// ── Wire types (Ollama API) ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    format: String,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    content: String,
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
