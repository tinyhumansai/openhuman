# Skill Sync Working Memory

This document describes how OpenHuman turns successful skill sync payloads into durable user working memory for agent personalization.

## Definition

- **User working memory**: persisted, user-scoped facts that remain useful across turns (preferences, goals, constraints, recurring entities).
- **Ephemeral chat context**: transient per-turn conversation state and prompt history; not persisted by this flow.
- **TTL policy**: no TTL by default (`ttl = "none"`), but growth is bounded with deterministic upsert keys.

## Hook location

- Sync entrypoint: `src/openhuman/skills/qjs_skill_instance/event_loop/rpc_handlers.rs` (`handle_sync`).
- Sync persistence worker: `src/openhuman/skills/qjs_skill_instance/event_loop/mod.rs` (`spawn_memory_write_worker`).
- Working-memory extraction: `src/openhuman/skills/working_memory.rs`.
- Agent recall/injection: `src/openhuman/agent/loop_/memory_context.rs` and `src/openhuman/agent/memory_loader.rs`.

Flow:
1. `skills.sync` triggers `skill/sync`.
2. On success, the event loop enqueues a memory write job.
3. The memory worker stores raw sync history and runs working-memory extraction.
4. Extracted working-memory documents are upserted into `global` with fixed keys:
   - `working.user.<skill>.preferences`
   - `working.user.<skill>.goals`
   - `working.user.<skill>.constraints`
   - `working.user.<skill>.entities`
   - `working.user.<skill>.summary`

Control switch:
- `OPENHUMAN_SKILLS_WORKING_MEMORY_ENABLED=false` disables this extraction/persistence path.

## Privacy and safety

- Sensitive keys (`token`, `secret`, `password`, `credential`, OAuth/auth fields, API keys, JWT/cookies) are skipped.
- Sensitive value heuristics are applied to avoid persisting secret-like blobs.
- Common PII patterns (email, phone) are redacted before persistence.

## Logging and observability

Per sync batch, the worker logs:
- scalar fields scanned
- sensitive fields skipped
- extracted preferences/goals/constraints/entities
- generated/persisted/failed working-memory docs

Log prefix: `[skills-working-memory]`.

## Agent usage (controlled)

- Agent context assembly now appends a bounded `[User working memory]` section.
- Only `working.user.*` keys are included, with relevance threshold + small caps.
- This keeps personalization available while preventing unbounded prompt growth.

## Extending for new skills

When a new integration needs better extraction quality:
1. Add or tune classification heuristics in `classify_into_buckets` and `looks_like_*` helpers in `src/openhuman/skills/working_memory.rs`.
2. Keep persistence bounded by reusing deterministic keys (do not introduce unbounded per-item keys by default).
3. Add/update tests with mocked sync payloads in `src/openhuman/skills/working_memory.rs`.
4. Verify degraded behavior remains non-fatal (sync success should not fail due to memory extraction issues).
