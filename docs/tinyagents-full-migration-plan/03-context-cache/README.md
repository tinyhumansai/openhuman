# 03 — Context management & caching

Live context reduction already runs on `ContextCompressionMiddleware` +
`MessageTrimMiddleware` + `tinyagents/summarize.rs`; microcompact and
cache-align live as TinyAgents middlewares. This workstream deletes the superseded
originals and adopts the crate cache layer.

Current status (2026-07-02): the legacy reducers are gone. `context/` is 1237
lines total, including 1051 lines across prompt assembly (`prompt.rs`, `channels_prompt.rs`),
session-memory bookkeeping (`session_memory.rs`), and stats/config state
(`manager.rs`, `stats.rs`). The inert legacy compaction-breaker stats have also
been removed. Those files remain product-owned; cache correctness work now targets
TinyAgents middleware and crate cache events.

Target SDK surface: `ResponseCache`/`InMemoryResponseCache` + `cache_key`,
`CachePolicy { response_cache_enabled, protect_prompt_prefix }`,
`PromptCacheLayout` + `PromptCacheGuardMiddleware` + `CacheLayoutEvent`,
`AgentEvent::{CacheHit, CacheMiss, Compressed}`, `SummaryRecord`,
`ModelRequest::cache_segments(Vec<PromptSegment>)`.

Steps:

1. `01-delete-legacy-context.md` — remove dead reducers, consolidate stats.
2. `02-cache-layer.md` — response cache + prompt-prefix protection.

Done when: `context/` contains only OpenHuman-specific state (prompt
assembly inputs, session-memory bookkeeping, stats projection) and cache
correctness is asserted by crate events, not warn-logs.

Keep (product): `context/manager.rs` prompt-assembly surface,
`session_memory.rs`, `channels_prompt.rs`, `context/stats` for the UI
utilisation footer.
