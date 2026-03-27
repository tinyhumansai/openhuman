# Conscious Loop — Implementation Plan

## Context

The app has a memory layer (TinyHumans Neocortex) where skills (gmail, telegram, notion, etc.) store synced data. Currently this data sits passively — it's only recalled when the user sends a chat message. The **Conscious Loop** is a periodic background process that proactively digests all skill memory into structured **actionable items**, matching the `ActionableItem` format already used in the Intelligence UI (`src/components/intelligence/mockData.ts`). This replaces mock data with real, LLM-extracted intelligence.

**Flow:** Recall all skill memory → LLM extracts actionables → Log response → Insert back into memory under `conscious` namespace.

---

## Files to Create

### 1. `src-tauri/ai/CONSCIOUS_LOOP.md` — LLM prompt template

The structured prompt that tells the LLM how to extract actionable items from recalled memory. Loaded at runtime via `find_ai_directory()` (same pattern as `SOUL.md`).

### 2. `src-tauri/src/commands/conscious_loop.rs` — Core implementation (~250 lines)

**Structs:**
- `ConsciousLoopStartedEvent` — emitted when a run begins
- `ConsciousLoopCompletedEvent` — emitted on success (includes actionable count, duration)
- `ConsciousLoopErrorEvent` — emitted on failure

**Functions:**

#### `conscious_loop_run` (Tauri command)
- Params: `app: AppHandle`, `auth_token`, `backend_url`, `model`, `memory_state: State<MemoryState>`
- Spawns `conscious_loop_run_inner` as background task, returns immediately
- Enables manual triggering from frontend

#### `conscious_loop_run_inner` (core logic)
1. **Emit** `conscious_loop:started`
2. **Get skill IDs** — call `engine.all_tools()` to obtain the set of active `skill_ids` (same source used by `chat_send_inner`). No separate API call needed — each skill ID is directly its memory namespace
3. **Recall memory** for each skill via `memory_client.recall_skill_context(&skill_id, &skill_id, 10)`. Collect as `Vec<(skill_id, context_text)>`. Skip skills that return `None`
4. **Load prompt** from `ai/CONSCIOUS_LOOP.md` via `find_ai_directory(app)`. Hardcoded fallback if file missing
5. **Build messages** — `[{role: "system", content: prompt}, {role: "user", content: assembled_contexts}]`
6. **Call inference** — POST to `{backend_url}/openai/v1/chat/completions` with Bearer auth, 120s timeout (same `reqwest` pattern as `chat_send_inner`)
7. **Log full response** — `log::info!("[conscious_loop] LLM response: {}", response)`
8. **Parse JSON array** — deserialize into `Vec<ExtractedActionable>` (title, description, source, priority, actionable, requires_confirmation, source_label, has_complex_action)
9. **Insert into memory** — for each item, call `memory_client.store_skill_sync("conscious", "actionables", &title, &json_content, ...)`. Use deterministic `document_id` (hash of title+source) for deduplication
10. **Emit** `conscious_loop:completed`

#### `conscious_loop_timer` (periodic runner)
- Spawned from `lib.rs` setup, runs on `tokio::time::interval(Duration::from_secs(300))` (5 min)
- 60s initial delay for skills to boot and memory to initialize
- Checks: memory client initialized? auth token present? If not, skip silently
- Calls `conscious_loop_run_inner` on each tick
- Uses `gpt-4o-mini` as default model (configurable via `OPENHUMAN_CONSCIOUS_MODEL` env var)

**Error handling:**
- Memory client not initialized → skip (timer) or return Err (manual command)
- No skill IDs found → emit completed with 0 items (not an error)
- Individual skill recall fails → log warning, skip, continue
- Inference call fails → emit `conscious_loop:error`
- JSON parse fails → log raw response, emit error
- Individual memory insert fails → log warning, continue with remaining items

---

## Files to Modify

### 3. `src-tauri/src/commands/memory.rs`
- Make `extract_namespaces_from_documents` **`pub(crate)`** (currently private `fn`)

### 4. `src-tauri/src/commands/mod.rs`
- Add `pub mod conscious_loop;`
- Add `pub use conscious_loop::*;`

### 5. `src-tauri/src/lib.rs`
- **Desktop handler list** (~line 1088): Add `conscious_loop_run` command
- **Mobile handler list** (~line 1095): Add no-op stub
- **Setup block** (~line 928): Spawn `conscious_loop_timer` after memory state is registered:
  ```rust
  let app_for_conscious = app.handle().clone();
  tauri::async_runtime::spawn(async move {
      commands::conscious_loop::conscious_loop_timer(app_for_conscious).await;
  });
  ```

---

## Key Design Decisions

| Decision | Rationale |
|---|---|
| **Rust-side timer** (not frontend setInterval) | App runs in tray mode where webview is hidden. Tokio interval survives this. Matches `watch_daemon_health_file` pattern |
| **`recall_skill_context`** (via `engine.all_tools()`) | Consistent with `chat_send_inner`. No extra API call — skill IDs are already available from the runtime. `integration_id` is passed as `skill_id` (same convention) |
| **`gpt-4o-mini` default** | Background summarization, not conversational. Faster + cheaper. Configurable via env var |
| **5-minute interval** | Frequent enough for time-sensitive items, conservative on tokens |
| **Deterministic document_id** | Hash of title+source enables dedup on repeated runs |
| **`conscious` namespace** | Clean separation from skill data. Avoids polluting skill namespaces |

---

## Event Protocol (Rust → Frontend)

| Event | Payload | Purpose |
|---|---|---|
| `conscious_loop:started` | `{ run_id, timestamp, namespaces[] }` | UI can show loading state |
| `conscious_loop:completed` | `{ run_id, actionable_count, duration_ms }` | UI can refresh Intelligence view |
| `conscious_loop:error` | `{ run_id, message, error_type }` | UI can show error indicator |

---

## ActionableItem Output Format

The LLM will output items matching this structure (from `src/types/intelligence.ts`):

```json
[
  {
    "title": "Reply to 2 critical emails expecting response within 24hrs",
    "description": "Messages from john@coinbase.com and sarah@ethereum.org about partnership proposals",
    "source": "email",
    "priority": "critical",
    "actionable": true,
    "requires_confirmation": false,
    "has_complex_action": true,
    "source_label": "Gmail"
  }
]
```

**Source values:** `email` | `calendar` | `telegram` | `ai_insight` | `system` | `trading` | `security`
**Priority values:** `critical` | `important` | `normal`

---

## CONSCIOUS_LOOP.md Prompt (for `src-tauri/ai/`)

```markdown
# Conscious Loop — Actionable Extraction

You are the conscious awareness layer of OpenHuman. You periodically review all
memory contexts from the user's connected integrations and extract actionable
items that deserve attention.

## Your Task

Analyze the recalled memory contexts provided below. For each context, identify
items that are:

1. **Time-sensitive** — deadlines, expiring offers, meetings, scheduled events
2. **Requires response** — unanswered emails, pending messages, open requests
3. **Opportunity** — insights, patterns, or suggestions the user may benefit from
4. **Risk/Alert** — security issues, anomalies, overdue tasks, budget warnings

## Output Format

Return a JSON array of actionable items. Each item must have this exact structure:

{
  "title": "Short descriptive title (under 80 chars)",
  "description": "1-2 sentence explanation with context",
  "source": "email|calendar|telegram|ai_insight|system|trading|security",
  "priority": "critical|important|normal",
  "actionable": true,
  "requires_confirmation": false,
  "has_complex_action": false,
  "source_label": "Human-readable source name (e.g. Gmail, Telegram, Notion)"
}

## Rules

- Return ONLY the JSON array, no markdown fences, no commentary
- Deduplicate: if the same item appears in multiple sources, merge into one
- Limit to 20 items maximum per run — prioritize the most important
- Use "ai_insight" as source when the item is a synthesized observation
- Use "system" for maintenance, sync status, or technical alerts
- Map integration sources: gmail -> "email", telegram -> "telegram", notion -> "system", google_calendar -> "calendar"
- Set priority "critical" only for truly urgent items (expiring today, security breach)
- Set priority "important" for items needing attention within 24-48 hours
- Set "has_complex_action" to true when the item requires multi-step user action
- Set "requires_confirmation" to true when the item involves financial transactions or irreversible actions
- If no actionable items are found, return an empty array: []
```

---

## Verification

1. **Compile check**: `cargo check --manifest-path src-tauri/Cargo.toml`
2. **Rust formatting**: `cargo fmt --manifest-path src-tauri/Cargo.toml`
3. **Manual test**: Call `invoke('conscious_loop_run', { authToken, backendUrl, model })` from frontend console
4. **Log verification**: Check Rust logs for `[conscious_loop]` entries showing recall data, LLM response, and insert results
5. **Memory verification**: Call `invoke('memory_list_documents', { namespace: 'conscious' })` to see stored actionables
6. **Timer verification**: Watch logs for periodic `[conscious_loop]` entries every 5 minutes after app startup

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────┐
│                    Rust Backend                       │
│                                                       │
│  ┌──────────────────┐    every 5 min                 │
│  │ conscious_loop_   │◄──────────────────┐           │
│  │ timer()           │                   │           │
│  └────────┬──────────┘          tokio::interval      │
│           │                              │           │
│           ▼                              │           │
│  ┌──────────────────┐                    │           │
│  │ conscious_loop_   │                               │
│  │ run_inner()       │                               │
│  │                   │                               │
│  │  1. engine.all_tools() ──► get skill_ids          │
│  │  2. recall_skill_context() per skill_id           │
│  │  3. Load CONSCIOUS_LOOP.md prompt                 │
│  │  4. POST /openai/v1/chat/completions              │
│  │  5. Log full LLM response                        │
│  │  6. Parse JSON → Vec<ExtractedActionable>         │
│  │  7. store_skill_sync("conscious", "actionables")  │
│  │  8. Emit events to frontend                       │
│  └──────────────────┘                               │
│           │                                          │
│           ▼                                          │
│  ┌──────────────────┐                               │
│  │ TinyHumans API    │  (recall + insert)            │
│  └──────────────────┘                               │
│           │                                          │
│           ▼                                          │
│  ┌──────────────────┐                               │
│  │ Backend LLM       │  (inference)                  │
│  └──────────────────┘                               │
└─────────────────────────────────────────────────────┘
           │
           ▼ Tauri events
┌─────────────────────────────────────────────────────┐
│                  React Frontend                       │
│                                                       │
│  listen('conscious_loop:completed')                  │
│     → Refresh Intelligence UI with real actionables  │
└─────────────────────────────────────────────────────┘
```
