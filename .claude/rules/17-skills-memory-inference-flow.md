---
paths:
  - "**/skills/**"
  - "**/memory/**"
  - "src/openhuman/**"
  - "app/src/providers/SkillProvider.tsx"
  - "app/src/lib/ai/**"
---

# Skills → Memory Layer → Agent Inference: Full Flow

## Overview

This document traces the complete data flow from skill discovery and OAuth/sync events, through
the TinyHumans Neocortex memory layer (tinyhumansai SDK), and into the Rust-side agentic
inference loop — showing exactly how skill data is written to and read from memory and how it
reaches the LLM context at inference time.

---

## 1. Skill Discovery & Lifecycle (SkillProvider.tsx)

**File**: `src/providers/SkillProvider.tsx`

On app mount (when a JWT token is present) `SkillProvider` calls
`invoke('runtime_discover_skills')` → the Rust runtime scans
`skills/skills/{skill-id}/manifest.json` from the git submodule and returns a manifest list.

```
Token present
  → discoverSkills()
    → invoke('runtime_discover_skills')         // Rust: qjs_engine.rs:184
      → reads skills/skills/*/manifest.json
      → filters: is_javascript() && supports_current_platform()
      → returns manifests[]
  → skillManager.registerSkill(manifest)        // in-memory registry
  → for each manifest with setupComplete:
      skillManager.startSkill(manifest)          // starts V8/QuickJS instance
```

Two Tauri event listeners run continuously:

| Event                          | What it does                                                             |
| ------------------------------ | ------------------------------------------------------------------------ |
| `skill-state-changed`          | Dispatches `setSkillState` into Redux `skillsSlice.skillStates[skillId]` |
| `runtime:skill-status-changed` | Updates `skillsSlice.skills[skillId].status`; surfaces errors            |

---

## 2. Memory Client Initialisation

**File**: `src-tauri/src/memory/mod.rs`
**Tauri command**: `init_memory_client` (called by frontend after auth)

The `MemoryClient` wraps the `TinyHumansMemoryClient` from the `tinyhumansai` Rust crate.
It is constructed with the user's JWT (`authSlice.token`) and stored as `Arc<MemoryClient>`
in `MemoryState` (a `Mutex<Option<MemoryClientRef>>`).

Base URL resolution (in priority order):

1. `OPENHUMAN_BASE_URL` env var
2. `TINYHUMANS_BASE_URL` env var
3. SDK default

---

## 3. Skill → Memory Sync: Two Write Paths

Both trigger inside `src-tauri/src/runtime/qjs_skill_instance.rs`.

### 3a. OAuth Completion (`skill/oauth-complete`)

When a user connects a skill via OAuth, the JS runtime calls back with `skill/oauth-complete`.
After the OAuth flow completes the skill's **ops state** (data published via `state.set()`) is
snapshotted and stored to memory:

```
skill/oauth-complete handler
  → handle_js_call(rt, ctx, "onOAuthComplete", params)   // runs skill JS
  → ops_state.read().data.clone()                        // snapshot skill state
  → tokio::spawn (fire-and-forget):
      MemoryClient::store_skill_sync(
          skill_id       = e.g. "gmail"
          integration_id = params["integrationId"]       // e.g. user email
          title          = "{skill} OAuth sync — {integrationId}"
          content        = JSON snapshot of ops state
          namespace      = "skill:{skill_id}:{integration_id}"
      )
      → tinyhumansai::TinyHumansMemoryClient::insert_memory(InsertMemoryParams { ... })
      → HTTP POST to TinyHumans Neocortex API
```

### 3b. Periodic Sync (`skill/sync`)

The runtime fires `skill/sync` events on a cron schedule. The flow is identical to OAuth
completion but uses `integration_id = "default"` and title `"{skill} periodic sync"`:

```
skill/sync handler
  → handle_js_call(rt, ctx, "onSync", "{}")
  → ops_state.read().data.clone()
  → tokio::spawn:
      MemoryClient::store_skill_sync(
          skill_id       = e.g. "notion"
          integration_id = "default"
          namespace      = "skill:notion:default"
          content        = JSON snapshot
      )
```

### Namespace Pattern

All skill memories are stored under `skill:{skill_id}:{integration_id}`.
Examples: `skill:gmail:user@example.com`, `skill:notion:default`.

---

## 4. Memory Operations Reference

**File**: `src-tauri/src/memory/mod.rs`

| Method                      | SDK call        | When used                                             |
| --------------------------- | --------------- | ----------------------------------------------------- |
| `store_skill_sync(...)`     | `insert_memory` | OAuth complete, periodic sync                         |
| `query_skill_context(...)`  | `query_memory`  | RAG query — fetch relevant chunks for a user question |
| `recall_skill_context(...)` | `recall_memory` | Recall synthesised summary from Master node           |
| `clear_skill_memory(...)`   | `delete_memory` | OAuth revoke / disconnect                             |

---

## 5. Conversation → Inference: Two Code Paths

**File**: `src/pages/Conversations.tsx`

`useRustChat()` returns `true` when running in Tauri (desktop). This selects between two paths:

```
handleSendMessage(text)
  ├─ rustChat == true  → Rust path  (invoke chat_send)
  └─ rustChat == false → Web path   (handleSendMessageWeb — TypeScript loop)
```

### 5a. Rust Path (Desktop)

```
chatSend({ threadId, message, model, authToken, backendUrl, messages, notionContext })
  → invoke('chat_send')               // src-tauri/src/commands/chat.rs:359
    → spawns background task
    → chat_send_inner(...)
```

Completion events flow back over Tauri events:

| Event              | Frontend handler                      |
| ------------------ | ------------------------------------- |
| `chat:tool_call`   | shows active tool indicator           |
| `chat:tool_result` | clears tool indicator                 |
| `chat:done`        | `dispatch(addInferenceResponse(...))` |
| `chat:error`       | shows error, clears loading state     |

### 5b. Web/Fallback Path

Used when not running in Tauri (browser). Runs the agentic loop entirely in TypeScript:

- Calls `inferenceApi.createChatCompletion(request)` directly
- Executes tools via `skillManager.callTool(skillId, toolName, args)`
- Both paths share the same 5-round `MAX_TOOL_ROUNDS` limit and `{skillId}__{toolName}` naming convention

---

## 6. Rust Agentic Loop in Detail

**File**: `src-tauri/src/commands/chat.rs` — `chat_send_inner()`

```
Step 1: Load OpenClaw context
  → load_openclaw_context(app)
    → reads ai/SOUL.md, IDENTITY.md, AGENTS.md, USER.md, BOOTSTRAP.md, MEMORY.md, TOOLS.md
    → cached in static AI_CONFIG_CACHE (cleared on restart)
    → truncated to MAX_CONTEXT_CHARS (20,000 chars)

Step 2: Recall memory context
  → MemoryClient::recall_skill_context("conversations", thread_id, 10)
    → tinyhumansai recall_memory(namespace="skill:conversations:{thread_id}")
    → returns synthesised summary string or None

Step 3: Build processed user message
  processed = user_message
  if openclaw_context  → prepend as "## Project Context\n...\n\nUser message: {processed}"
  if memory_context    → prepend as "[MEMORY_CONTEXT]\n{mem}\n[/MEMORY_CONTEXT]\n\n{processed}"
  if notion_context    → prepend as "{notionContext}\n\n{processed}"

Step 4: Build messages array
  → history (ChatMessagePayload[]) + processed user message

Step 5: Discover tools
  → engine.all_tools()
    → returns all tools from running skills
    → namespaced: "{skill_id}__{tool_name}"
    → formatted as OpenAI function-calling schema

Step 6: Agentic loop (max 5 rounds)
  for round in 0..MAX_TOOL_ROUNDS:
    POST {backend_url}/openai/v1/chat/completions
      body: { model, messages, tools, tool_choice: "auto" }
      timeout: 120s
      auth: Bearer {auth_token}

    if finish_reason == "tool_calls":
      emit chat:tool_call
      engine.call_tool(skill_id, tool_name, args)   // 60s timeout
        → QuickJS/V8 runtime executes skill JS tool handler
      emit chat:tool_result
      append tool result to messages
      continue loop

    else (finish_reason == "stop"):
      emit chat:done { full_response, rounds_used, token_counts }
      return Ok(())
```

---

## 7. Tool Execution: Rust → Skill JS

**File**: `src-tauri/src/runtime/qjs_engine.rs` — `call_tool(skill_id, tool_name, args)`

The Rust runtime routes `call_tool` into the running QuickJS/V8 skill instance:

- Serialises `args` as JSON
- Calls the JS tool handler registered by the skill
- Returns `ToolCallResult { content: Vec<ToolContent>, is_error: bool }`
- Text content is extracted and appended as the `tool` role message in the loop

---

## 8. End-to-End Flow Summary

```
User types message (Conversations.tsx)
  │
  ├─ [Web path only] invoke('recall_memory')      → TinyHumans API (recall)
  │                                                  returns synthesised context
  ├─ buildNotionContext()                          → from Redux skillStates.notion
  │
  ▼
invoke('chat_send')                                [Rust/desktop path]
  │
  ▼
Rust: chat_send_inner()
  ├─ load_openclaw_context()                       → ai/*.md files (cached)
  ├─ recall_skill_context("conversations", tid)    → TinyHumans API (recall)
  ├─ Build prompt: [OpenClaw] + [MEMORY] + [Notion] + user_message
  ├─ discover_tools()                              → all running skill tools
  │
  └─ Agentic loop (≤5 rounds):
       POST /openai/v1/chat/completions             → Backend LLM (neocortex-mk1)
         │
         ├─ tool_calls → engine.call_tool()         → QuickJS/V8 skill instance
         │                  └─ JS tool handler
         │                  └─ returns result string
         │                  └─ appended as tool message
         │
         └─ stop → emit chat:done
                     └─ Frontend: dispatch(addInferenceResponse(...))

Separately (async, fire-and-forget):
  Skill onSync() / onOAuthComplete()
    → MemoryClient::store_skill_sync()
      → TinyHumans insert_memory (namespace: skill:{id}:{integrationId})
```

---

## Key Files Reference

| File                                          | Role                                               |
| --------------------------------------------- | -------------------------------------------------- |
| `src/providers/SkillProvider.tsx`             | Discovery, lifecycle, Redux state sync             |
| `src/pages/Conversations.tsx`                 | Message send, both code paths, Notion context      |
| `src/services/chatService.ts`                 | `chatSend()`, `chatCancel()`, `useRustChat()`      |
| `src-tauri/src/commands/chat.rs`              | Rust agentic loop, context assembly, tool dispatch |
| `src-tauri/src/commands/memory.rs`            | `recall_memory` Tauri command                      |
| `src-tauri/src/memory/mod.rs`                 | `MemoryClient` wrapping tinyhumansai SDK           |
| `src-tauri/src/runtime/qjs_skill_instance.rs` | Skill sync → memory write triggers                 |
| `src-tauri/src/runtime/qjs_engine.rs`         | `discover_skills()`, `call_tool()`, `all_tools()`  |
| `skills/skills/*/manifest.json`               | Skill metadata (git submodule)                     |
