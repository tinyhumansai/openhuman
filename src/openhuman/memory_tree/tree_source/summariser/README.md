# Summariser

Summariser trait and implementations used by the bucket-seal cascade. A summariser folds N buffered items into one sealed [`SummaryOutput`]; the seal machinery (bucket budgeting, persistence, label resolution) lives in [`super::bucket_seal`] and is unaffected by the choice of implementation.

## Public surface

- `pub trait Summariser` / `pub struct SummaryInput` / `pub struct SummaryContext` / `pub struct SummaryOutput` — `mod.rs` — async trait + IO types.
- `pub fn build_summariser` — `mod.rs` — picks the implementation based on `Config::memory_tree.llm_summariser_*`. Returns the LLM summariser when both endpoint and model are set, otherwise the inert fallback.
- `pub struct InertSummariser` — `inert.rs` — deterministic concat-and-truncate fallback. `entities` and `topics` are intentionally empty (an honest stub — derived labels are an LLM concern).
- `pub struct LlmSummariser` / `pub struct LlmSummariserConfig` — `llm.rs` — Ollama `/api/chat` peer of `score::extract::llm`. Soft-falls-back to inert on every error so seal cascades never abort.

## Files

- `mod.rs` — trait, IO types, and the `build_summariser` factory.
- `inert.rs` — deterministic fallback, used in tests and when no LLM is configured.
- `llm.rs` — Ollama-backed implementation with prompt construction, per-input clamping for `num_ctx` safety, and post-generation budget enforcement.
