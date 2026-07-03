# 03.2 — Crate cache layer

## Steps

1. **Prompt-prefix protection:** install `PromptCacheGuardMiddleware` in
   `assemble_turn_harness`; declare the stable prefix with
   `ModelRequest::cache_segments` (system prompt + tool schemas as
   `PromptSegment`s from the prompt builder). Route `CacheLayoutEvent`s to
   the event bridge as warnings — replaces the old cache_align warn-log with
   structured events. Set `CachePolicy.protect_prompt_prefix = true` on the
   turn `RunPolicy`.
2. **Response cache:** attach a `ResponseCache` via
   `AgentHarness::with_response_cache` for deterministic internal calls
   (summarizer/triage/subconscious-style runs; NOT interactive chat).
   Start with `InMemoryResponseCache`; a `Store`-backed impl over
   `FileStore` is a follow-up if hit rates justify it. Gate per-request
   with `ModelRequest::with_cache_policy`.
3. Assert prompt-prefix stability across turns in a fixture test:
   `PromptCacheLayout::from_request(...).is_prefix_stable_against(prev)` —
   volatile content (timestamps, memory, steering) must land in the tail.
   This formalizes the prompt-cache-stability decisions currently embedded
   in session prompt assembly.
4. Surface `CacheHit`/`CacheMiss` counts in the cost footer projection.

## Deletions

- `CacheAlignMiddleware`'s bespoke detector once `PromptCacheGuardMiddleware`
  + layout events cover it (keep OpenHuman's volatile-token *vocabulary* as
  test fixtures).

## Acceptance

- Prefix-stability fixture green over a 3-turn session with memory injection.
- Response-cache hit serves an identical deterministic request (test with
  `MockModel::call_count`).
