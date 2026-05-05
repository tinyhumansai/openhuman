# Memory tree — score extract

Entity extraction for the scoring pipeline. Pluggable via the `EntityExtractor` trait so the scorer can run a deterministic regex pass plus an optional LLM pass and merge their outputs. Also surfaces the LLM-derived importance rating consumed by the `llm_importance` signal.

## Public surface

- `pub trait EntityExtractor` — `extractor.rs` — async `extract(text) -> ExtractedEntities` contract.
- `pub struct RegexEntityExtractor` / `pub struct CompositeExtractor` — `extractor.rs` — built-in implementations.
- `pub struct LlmEntityExtractor` / `pub struct LlmExtractorConfig` — `llm.rs` — Ollama-backed semantic NER + importance rater.
- `pub fn build_summary_extractor` — `mod.rs` — composes regex + LLM (with `emit_topics: true`) for seal-time summary labelling.
- `pub enum EntityKind` / `pub struct ExtractedEntity` / `pub struct ExtractedTopic` / `pub struct ExtractedEntities` — `types.rs`.

## Files

- `mod.rs` — module surface and `build_summary_extractor` for the seal path.
- `types.rs` — output types and the `EntityKind` enum (mechanical kinds `Email/Url/Handle/Hashtag` + semantic kinds `Person/Organization/Location/...` + `Topic`). `ExtractedEntities::merge` deduplicates entities and combines LLM importance by max.
- `extractor.rs` — `EntityExtractor` trait, `RegexEntityExtractor` adapter, `CompositeExtractor` (runs a sequence of extractors and tolerates per-extractor failures).
- `regex.rs` — once-compiled regex patterns for email, URL, handle (`@alice` and Discord-style `alice#1234`), and hashtag. UTF-8 safe — spans are char offsets, not bytes.
- `llm.rs` — Ollama `/api/chat` client that asks the model for NER + an importance rating in one structured-JSON call, with span recovery via `text.find(...)` and a soft fallback (warn + empty) on transport failure.
- `llm_tests.rs` — unit tests for the LLM extractor.
