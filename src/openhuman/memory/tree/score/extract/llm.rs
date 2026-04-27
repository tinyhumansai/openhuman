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

use super::types::{EntityKind, ExtractedEntities, ExtractedEntity};
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
            ],
            strict_kinds: false,
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
                    content: SYSTEM_PROMPT.to_string(),
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
        // separately catches errors from its own extractor chain). A caller
        // distinguishes "LLM had nothing to say" from "LLM ran and returned
        // zero entities" by inspecting `llm_importance` — `None` means the
        // call didn't complete successfully.
        Ok(self.extract_or_empty(text).await)
    }
}

impl LlmEntityExtractor {
    /// Internal: wraps the actual HTTP call and returns `ExtractedEntities`
    /// for every failure mode via soft-fallback. Split out of `extract` so
    /// the error branches can share logging without `?`-propagation.
    async fn extract_or_empty(&self, text: &str) -> ExtractedEntities {
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
                log::warn!(
                    "[memory_tree::extract::llm] transport failure to {url}: {e} — \
                     returning empty extraction"
                );
                return ExtractedEntities::default();
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
            return ExtractedEntities::default();
        }

        let envelope: OllamaChatResponse = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "[memory_tree::extract::llm] response body not Ollama-shaped JSON: {e} — \
                     returning empty extraction"
                );
                return ExtractedEntities::default();
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
                return ExtractedEntities::default();
            }
        };

        parsed.into_extracted_entities(text, &self.cfg)
    }
}

// ── Prompt ───────────────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = "\
You are a named-entity extractor and importance rater. Return JSON only — \
no prose, no markdown, no commentary. Do not summarize. Extract every named \
entity mention you find, including duplicates, and rate the chunk's overall \
importance as a float in [0.0, 1.0].

Schema:
{
  \"entities\": [
    { \"kind\": \"person|organization|location|event|product\",
      \"text\": \"<exact surface form as it appears in the text>\" }
  ],
  \"importance\": 0.0,
  \"importance_reason\": \"<one short sentence explaining the rating>\"
}

Importance guide:
  0.9+  actionable decisions, key information, explicit commitments
  0.6+  substantive discussion, factual content, named entities
  0.3+  ambient context, low-density prose
  <0.3  reactions, acknowledgments, bots, trivial exchanges
";

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

        ExtractedEntities {
            entities,
            topics: Vec::new(),
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
