# Agent-Harness E2E Plan: Channels + Prompt-Flow Coverage

Branch: `agent-harness-e2e-channels`

---

## 1. Current State (~300 words)

### Core: Telegram provider

The Telegram channel is a mature, production provider at `src/openhuman/channels/providers/telegram/`. It long-polls via `getUpdates` (`channel_ops.rs:307-380`), parses inbound messages/reactions (`channel_recv.rs`), sends outbound text/media/reactions (`channel_send.rs`), and supports draft streaming, remote-control slash commands (`remote_control.rs`), and pairing/allowlist auth (`channel_core.rs`).

The channel runtime (`src/openhuman/channels/runtime/startup.rs`) wires Telegram (and all channels) into the dispatch loop, which feeds inbound messages into the agent harness via `request_native_global("agent.run_turn", ...)`. The harness runs the full tool-call loop and returns a response that the channel sends back.

The RPC surface (`src/openhuman/channels/controllers/schemas.rs`) exposes `openhuman.channels_connect`, `channels_disconnect`, `channels_status`, `channels_test`, `telegram_login_start`, `telegram_login_check`, `channels_send_message`, and more.

**Critical blocker**: `api_url()` is hardcoded to `https://api.telegram.org/bot{token}/{method}` (`channel_core.rs:88-89`). There is no env-var override to redirect Telegram API calls to a mock server. This must be addressed in WS-B.

### Mock backend

The mock server (`scripts/mock-api/`) has mature LLM mocking (`routes/llm.mjs` with `llmStreamScript`, `llmForcedResponses`, `llmKeywordRules`), Composio integration mocking (`routes/integrations.mjs` with `composioConnections`, `composioAvailableTriggers`, `composioExecuteResponse_*`), cron mocking (`routes/cron.mjs`), and a full admin API (`admin.mjs`). Socket.IO event injection exists via `/__admin/socket/emit`. There are **no** Telegram Bot API mock routes whatsoever.

### E2E suite

Five `chat-harness-*.spec.ts` specs cover send+stream, cancel, scroll-render, subagent delegation, and wallet flows. `composio-triggers-flow.spec.ts` tests trigger CRUD via core RPC. `cron-jobs-flow.spec.ts` tests the cron panel UI. `webhooks-ingress-flow.spec.ts` tests webhook RPC surface stubs.

`telegram-flow.spec.ts` (1019 lines) is entirely `describe.skip`'d. It was written for the old skill system (references SkillsGrid, V8 runtime, `Connect Telegram` OAuth modal). None of its test IDs match the current channel system. It should be **deleted and replaced**, not salvaged.

---

## 2. Gaps

### Core (Rust)

- **No Telegram API base URL override**: `api_url()` always targets `api.telegram.org`. Need an env var (`OPENHUMAN_TELEGRAM_API_BASE`) or constructor parameter so the provider can be pointed at the mock server during E2E.
- **No webhook ingress endpoint**: Telegram long-polls via `getUpdates`; there is no HTTP endpoint where a mock Telegram could push updates. For E2E, the provider needs either: (a) the mock to serve `getUpdates` responses (preferred, since the provider already uses long-polling), or (b) a webhook receiver route on the core. Option (a) is simpler since it matches existing architecture.
- **`channels_connect` for bot_token auth mode** needs verification that it works end-to-end against the mock `getMe` endpoint.

### Mock backend

- **No Telegram Bot API routes**: no `/bot<token>/getMe`, `/bot<token>/getUpdates`, `/bot<token>/sendMessage`, etc.
- **No tool-call round-trip scripting for harness flows**: `llmKeywordRules` supports `toolCalls` but there is no multi-turn scripting (message 1 -> tool call -> tool result -> message 2 with final answer). Need `llmForcedResponses` queue patterns documented and possibly extended for chained tool-use turns.
- **No Composio action execution result fixtures** for E2E prompt-flow tests (only `composioExecuteResponse_<ACTION>` per-action overrides exist, which is actually sufficient).
- **No cron-creation mock for LLM-driven flows**: the mock LLM can return tool calls, but there is no mock for `openhuman.cron_create` being called as a tool result round-trip.

### E2E specs

- **`telegram-flow.spec.ts`**: 100% stale, references removed skill system. Delete.
- **No Telegram channel connect/disconnect E2E spec** for the current `channels_connect`/`channels_disconnect` RPC surface.
- **No prompt-flow E2E specs**: no tests exercise the harness processing a message that triggers a tool call (composio, search, cron) and returning a result.
- **No cross-channel bridge E2E**: no test sends a Telegram message that produces a cron job or composio action.

---

## 3. Workstream Breakdown

### WS-A: Mock Backend — Telegram Bot API + Harness Tool-Call Plumbing

**Goal**: Add mock Telegram Bot API routes and extend LLM mock scripting so downstream specs can drive deterministic Telegram + tool-call round-trips.

**Files to create/modify**:

| Action | Path |
|--------|------|
| CREATE | `scripts/mock-api/routes/telegram.mjs` |
| MODIFY | `scripts/mock-api/routes/llm.mjs` (document multi-turn forced response patterns; add `llmToolCallSequence` behavior key for chained turns) |
| MODIFY | `scripts/mock-api/server.mjs` (import and wire `handleTelegram` into the route chain) |
| MODIFY | `scripts/mock-api/state.mjs` (add `mockTelegramUpdates`, `mockTelegramSentMessages` state arrays with getters/setters/resetters) |
| MODIFY | `scripts/mock-api/admin.mjs` (add `GET /__admin/telegram/sent`, `POST /__admin/telegram/inject-update`, `POST /__admin/telegram/reset` endpoints) |
| MODIFY | `app/test/e2e/mock-server.ts` (re-export any new helpers needed by specs) |

**Mock-backend changes**:

New route handler `handleTelegram(ctx)` in `scripts/mock-api/routes/telegram.mjs`:

| Route pattern | Behavior |
|---------------|----------|
| `POST /bot<token>/getMe` | Returns `{ ok: true, result: { id: 123, is_bot: true, username: behavior.telegramBotUsername \|\| "e2e_test_bot" } }` |
| `POST /bot<token>/getUpdates` | Returns updates from `mockTelegramUpdates` queue. Supports long-poll simulation via `telegramPollDelayMs` behavior key. Each call drains the queue. |
| `POST /bot<token>/sendMessage` | Records to `mockTelegramSentMessages`, returns `{ ok: true, result: { message_id: <seq>, chat: {...}, text: <text> } }` |
| `POST /bot<token>/sendChatAction` | Returns `{ ok: true, result: true }` |
| `POST /bot<token>/deleteWebhook` | Returns `{ ok: true, result: true }` |
| `POST /bot<token>/setMessageReaction` | Returns `{ ok: true, result: true }` |
| `POST /bot<token>/sendPhoto`, `sendDocument`, `sendVideo`, `sendAudio`, `sendVoice` | Records to sent log, returns ok |

Behavior keys:
- `telegramBotUsername` — bot username returned by `getMe`
- `telegramBotToken` — expected token (for auth validation; default: accept any)
- `telegramPollDelayMs` — simulated long-poll delay for `getUpdates`
- `telegramGetMeFails` — if `"1"`, `getMe` returns 401
- `telegramSendFails` — if `"1"`, `sendMessage` returns 400

Admin endpoints:
- `POST /__admin/telegram/inject-update` — push a Telegram update JSON into the queue (spec calls this to simulate an inbound message)
- `GET /__admin/telegram/sent` — list all messages the bot "sent" (for assertion)
- `POST /__admin/telegram/reset` — clear queues

LLM mock extension — `llmToolCallSequence` behavior key:
```json
[
  {
    "match": "create a cron",
    "response": {
      "toolCalls": [{"name": "cron_create", "arguments": {"schedule": "0 9 * * *", "prompt": "morning briefing"}}],
      "content": ""
    }
  },
  {
    "match": "cron_create-result",
    "response": {
      "content": "Done! I created a daily 9am cron job for your morning briefing."
    }
  }
]
```
This is actually already achievable with the existing `llmKeywordRules` + `llmForcedResponses` mechanisms. The work here is documenting the pattern and adding one convenience: a `llmToolCallScript` behavior key that accepts a sequence of `[{toolCalls, content}, {content}]` entries that auto-advance after each provider call, replacing `llmForcedResponses` for multi-turn scenarios. This avoids specs needing to manually queue and manage the forced response array.

**Test scenarios** (unit tests for mock routes):
1. `getMe` returns bot info with default and custom username
2. `getUpdates` returns empty when no updates queued
3. `getUpdates` returns injected updates and drains queue
4. `sendMessage` records message and returns success
5. `sendMessage` returns error when `telegramSendFails=1`
6. Admin inject-update + sent-list round-trip
7. `llmToolCallScript` auto-advances through multi-turn sequence

**Acceptance criteria**:
- A spec can: (1) set `telegramBotUsername`, (2) inject a Telegram update via admin, (3) observe the bot's reply in `/__admin/telegram/sent`, (4) configure LLM to return tool calls on specific keywords.
- All existing mock-api tests pass (`scripts/mock-api/routes/__tests__/`).

**Dependencies**: None. This is foundational infrastructure.

---

### WS-B: Core Wiring — Telegram API Base URL Override

**Goal**: Allow the Telegram provider to target a mock server instead of `api.telegram.org` via an environment variable, enabling E2E testing of the full Telegram channel loop.

**Files to create/modify**:

| Action | Path |
|--------|------|
| MODIFY | `src/openhuman/channels/providers/telegram/channel_core.rs` — `api_url()` reads `OPENHUMAN_TELEGRAM_API_BASE` env var; defaults to `https://api.telegram.org` |
| MODIFY | `src/openhuman/channels/providers/telegram/channel_types.rs` — add `api_base: String` field to `TelegramChannel` struct |
| MODIFY | `src/openhuman/channels/providers/telegram/channel_core.rs` — constructor reads env var, stores in `api_base` |
| MODIFY | `src/openhuman/channels/runtime/startup.rs` — no changes needed if env var is read in constructor |
| MODIFY | `.env.example` — document `OPENHUMAN_TELEGRAM_API_BASE` |
| MODIFY | `app/scripts/e2e-run-spec.sh` — export `OPENHUMAN_TELEGRAM_API_BASE=http://127.0.0.1:${E2E_MOCK_PORT}` when running Telegram specs |
| CREATE | `src/openhuman/channels/providers/telegram/channel_core_tests.rs` or extend existing `channel_tests.rs` — test that `api_url()` respects the override |

**Detailed changes**:

`channel_types.rs` — add field:
```rust
pub struct TelegramChannel {
    // ... existing fields ...
    api_base: String,  // NEW: base URL for Telegram Bot API
}
```

`channel_core.rs` — constructor:
```rust
pub fn new(bot_token: String, allowed_users: Vec<String>, mention_only: bool) -> Self {
    let api_base = std::env::var("OPENHUMAN_TELEGRAM_API_BASE")
        .unwrap_or_else(|_| "https://api.telegram.org".to_string());
    // ... rest unchanged, but store api_base ...
}
```

`channel_core.rs` — `api_url()`:
```rust
pub(crate) fn api_url(&self, method: &str) -> String {
    format!("{}/bot{}/{method}", self.api_base, self.bot_token)
}
```

**Test scenarios**:
1. `api_url()` returns `https://api.telegram.org/bot<token>/<method>` by default
2. With `OPENHUMAN_TELEGRAM_API_BASE=http://localhost:18473`, `api_url()` returns `http://localhost:18473/bot<token>/<method>`
3. Trailing slash in env var is stripped
4. `cargo check` and `cargo test` pass

**Acceptance criteria**:
- `api_url()` respects `OPENHUMAN_TELEGRAM_API_BASE` env var
- Default behavior unchanged (still `api.telegram.org`)
- Unit test covers the override
- `e2e-run-spec.sh` exports the env var for Telegram specs

**Dependencies**: None. Can run in parallel with WS-A.

---

### WS-C: Telegram E2E Spec Rewrite

**Goal**: Replace the stale `telegram-flow.spec.ts` with a new spec that tests the current `channels_*` RPC surface and the full Telegram bot setup + message round-trip.

**Files to create/modify**:

| Action | Path |
|--------|------|
| DELETE | `app/test/e2e/specs/telegram-flow.spec.ts` (1019 lines, 100% stale) |
| CREATE | `app/test/e2e/specs/telegram-channel-flow.spec.ts` |
| MODIFY | `app/test/e2e/helpers/chat-harness.ts` — add `injectTelegramUpdate()` and `getTelegramSentMessages()` helpers that call mock admin endpoints |
| MODIFY | `app/scripts/e2e-run-spec.sh` — ensure `OPENHUMAN_TELEGRAM_API_BASE` is set for telegram specs (may overlap with WS-B) |

**Test scenarios** (numbered):

1. **Channel list includes telegram**: `callOpenhumanRpc('openhuman.channels_list')` returns a channel with `id: "telegram"` and `authModes` including `bot_token`.

2. **Channel describe returns telegram definition**: `callOpenhumanRpc('openhuman.channels_describe', { channel: 'telegram' })` returns capabilities, auth modes, and field schemas.

3. **Bot token connect — happy path**: `callOpenhumanRpc('openhuman.channels_connect', { channel: 'telegram', authMode: 'bot_token', credentials: { botToken: '<token>' } })` succeeds. Mock `getMe` returns bot info. `channels_status` shows telegram as connected.

4. **Bot token connect — invalid token**: Mock `getMe` returns 401 (`telegramGetMeFails=1`). Connect RPC returns error.

5. **Inbound message round-trip**: After connecting, inject a Telegram update via `/__admin/telegram/inject-update` with a user message. Configure `llmForcedResponses` with a canned reply. Wait for the bot's reply to appear in `/__admin/telegram/sent`. Assert the reply content matches.

6. **Inbound message from unauthorized user**: Inject an update from a user not in the allowlist. Assert the bot sends the "operator approval required" message (visible in `/__admin/telegram/sent`).

7. **Group message with mention-only**: Connect with `mentionOnly: true`. Inject a group message without bot mention — no reply. Inject a group message with `@e2e_test_bot` — reply appears.

8. **Channel disconnect**: `callOpenhumanRpc('openhuman.channels_disconnect', { channel: 'telegram', authMode: 'bot_token' })` succeeds. `channels_status` shows telegram as disconnected.

9. **Reconnect after disconnect**: Connect again with a different token. Status shows connected.

10. **Remote command /status**: Inject a message with text `/status`. Assert the bot sends a status response (contains "Thread:" and "Provider:").

**Acceptance criteria**:
- All 10 scenarios pass against the mock backend
- No references to the old skill system
- Spec uses `resetApp()` + `callOpenhumanRpc()` pattern from existing specs
- Spec runs via `pnpm debug e2e test/e2e/specs/telegram-channel-flow.spec.ts telegram`

**Dependencies**: WS-A (mock Telegram routes), WS-B (API base URL override). Must wait for both.

---

### WS-D: Prompt-Flow Harness E2E Specs

**Goal**: Add a battery of E2E specs that drive the chat harness through prompts exercising tool calls (composio, search, cron) and cross-channel bridges.

**Files to create/modify**:

| Action | Path |
|--------|------|
| CREATE | `app/test/e2e/specs/harness-composio-tool-flow.spec.ts` |
| CREATE | `app/test/e2e/specs/harness-cron-prompt-flow.spec.ts` |
| CREATE | `app/test/e2e/specs/harness-search-tool-flow.spec.ts` |
| CREATE | `app/test/e2e/specs/harness-channel-bridge-flow.spec.ts` |
| MODIFY | `app/test/e2e/helpers/chat-harness.ts` — add `waitForToolCallInMockLog(toolName)`, `waitForAssistantReplyContaining(text)` helpers |

**Spec 1: `harness-composio-tool-flow.spec.ts`**

Scenarios:
1. **Gmail composio tool call**: Configure `llmKeywordRules` so "check my email" triggers a `GMAIL_GET_MAIL` tool call. Configure `composioExecuteResponse_GMAIL_GET_MAIL` with a canned inbox result. Send "check my email" in chat. Assert: (a) mock LLM received the tool call, (b) composio execute endpoint was called, (c) final assistant reply references the email content.
2. **GitHub composio tool call**: "list my repos" triggers `GITHUB_LIST_REPOS`. Assert tool-use round-trip.
3. **Composio action failure**: Set `composioExecuteFails=400`. Send prompt. Assert the assistant reply acknowledges the error gracefully.
4. **Linear composio tool call**: "create a linear issue" triggers `LINEAR_CREATE_ISSUE`. Assert creation result in reply.

**Spec 2: `harness-cron-prompt-flow.spec.ts`**

Scenarios:
1. **Create cron via natural language**: Configure LLM keyword rules so "remind me every morning at 9am" triggers a `cron_create` tool call with `{ schedule: "0 9 * * *", prompt: "morning reminder" }`. Assert: cron_create RPC was called, reply confirms creation.
2. **List cron jobs after creation**: Send "what are my scheduled tasks". LLM keyword rule returns content listing the jobs (no tool call needed, just checks the harness can relay cron state). Verify via `openhuman.cron_list` oracle RPC.
3. **Edit cron schedule**: "change my morning reminder to 8am" triggers `cron_update` tool call. Assert schedule changed via oracle RPC.

**Spec 3: `harness-search-tool-flow.spec.ts`**

Scenarios:
1. **Memory search tool call**: "what did we discuss about project X" triggers `memory_search` tool call. Mock returns canned memory results. Assert reply cites the memory.
2. **Web search tool call**: "search the web for Rust async patterns" triggers `web_search` tool call. Mock returns canned search results. Assert reply includes search results.
3. **File read tool call**: "read the README" triggers `file_read` tool call. Assert reply includes file content summary.

**Spec 4: `harness-channel-bridge-flow.spec.ts`**

Scenarios:
1. **Telegram message triggers cron creation**: Inject a Telegram update "set up a daily standup reminder at 9am". LLM keyword rules return a `cron_create` tool call. Assert: (a) cron created via oracle RPC, (b) Telegram reply confirms creation.
2. **Telegram message triggers composio action**: Inject "check my gmail inbox" via Telegram. LLM triggers `GMAIL_GET_MAIL`. Assert: (a) composio execute called, (b) Telegram reply contains email summary.
3. **Chat prompt references channel state**: In the web chat, ask "what messages came in on Telegram today". LLM returns a canned summary. This is a lightweight check that the harness can receive prompts referencing channels.

**Acceptance criteria**:
- All specs pass against the mock backend with zero real LLM calls
- Each spec uses `resetApp()` for isolation
- Tool call round-trips are verified via both mock request logs and UI/RPC assertions
- Specs are independently runnable via `pnpm debug e2e`

**Dependencies**:
- `harness-composio-tool-flow.spec.ts`: Needs existing mock composio routes (already in `integrations.mjs`) + LLM keyword rules (already in `llm.mjs`). **No blocker.**
- `harness-cron-prompt-flow.spec.ts`: Needs LLM keyword rules + cron RPC surface (already exists). **No blocker.**
- `harness-search-tool-flow.spec.ts`: Needs LLM keyword rules. **No blocker.**
- `harness-channel-bridge-flow.spec.ts`: Depends on **WS-A** (mock Telegram routes) and **WS-B** (API base override). Scenarios 1-2 must wait. Scenario 3 can ship independently.

---

## 4. Recommended Subagent Fan-Out

### WS-A -> CodeCrusher agent

**Briefing**: You are implementing the mock backend Telegram Bot API layer. Create `scripts/mock-api/routes/telegram.mjs` with a `handleTelegram(ctx)` function that serves Telegram Bot API endpoints (`/bot<token>/getMe`, `/bot<token>/getUpdates`, `/bot<token>/sendMessage`, `/bot<token>/sendChatAction`, `/bot<token>/deleteWebhook`, `/bot<token>/setMessageReaction`, and media send endpoints). Add state arrays `mockTelegramUpdates` and `mockTelegramSentMessages` to `scripts/mock-api/state.mjs` with standard getter/setter/reset exports. Add admin endpoints in `scripts/mock-api/admin.mjs`: `POST /__admin/telegram/inject-update`, `GET /__admin/telegram/sent`, `POST /__admin/telegram/reset`. Wire into `scripts/mock-api/server.mjs`. Follow the exact patterns used by existing route handlers (see `routes/integrations.mjs`, `routes/cron.mjs`). Use `behavior()` for dynamic behavior keys (`telegramBotUsername`, `telegramGetMeFails`, `telegramSendFails`, `telegramPollDelayMs`). Token is extracted from the URL path (`/bot<token>/...`). Write unit tests in `scripts/mock-api/routes/__tests__/telegram.test.mjs` following the pattern in existing test files in that directory.

### WS-B -> Dev agent (Rust)

**Briefing**: You are adding a `OPENHUMAN_TELEGRAM_API_BASE` environment variable override to the Telegram channel provider. In `src/openhuman/channels/providers/telegram/channel_types.rs`, add an `api_base: String` field to `TelegramChannel`. In `channel_core.rs`, read `std::env::var("OPENHUMAN_TELEGRAM_API_BASE")` in the constructor (default `"https://api.telegram.org"`, strip trailing slash), store in `self.api_base`. Change `api_url()` from `format!("https://api.telegram.org/bot{}/{method}", self.bot_token)` to `format!("{}/bot{}/{method}", self.api_base, self.bot_token)`. Add a unit test in `channel_tests.rs` that sets the env var (use a `serial_test` guard or `temp_env` crate if available, otherwise test with a direct constructor that takes the base URL). Update `.env.example` with a comment. Update `app/scripts/e2e-run-spec.sh` to export `OPENHUMAN_TELEGRAM_API_BASE=http://127.0.0.1:${E2E_MOCK_PORT:-18473}` alongside the other E2E env vars. Run `cargo check` and `cargo test` to verify.

### WS-C -> Test agent (E2E)

**Briefing**: You are rewriting the Telegram E2E spec. Delete `app/test/e2e/specs/telegram-flow.spec.ts` entirely (it is 100% stale, references removed skill system). Create `app/test/e2e/specs/telegram-channel-flow.spec.ts`. Follow the exact patterns from `chat-harness-send-stream.spec.ts` and `composio-triggers-flow.spec.ts`: use `resetApp()`, `callOpenhumanRpc()`, `startMockServer()`/`stopMockServer()`, `setMockBehavior()`. The spec tests the `openhuman.channels_*` RPC surface against the mock backend. Add helpers to `app/test/e2e/helpers/chat-harness.ts` for `injectTelegramUpdate(update)` (POST to `/__admin/telegram/inject-update`) and `getTelegramSentMessages()` (GET `/__admin/telegram/sent`). Test scenarios: channels_list includes telegram, channels_describe returns definition, connect with bot_token (happy + error), inbound message round-trip, unauthorized user rejection, mention-only group filtering, disconnect, reconnect, remote /status command. Each test uses `callOpenhumanRpc` for setup and oracle checks, mock admin endpoints for Telegram simulation. Set `OPENHUMAN_TELEGRAM_API_BASE` and `telegramBotUsername` behavior key in `before()`. This spec depends on WS-A and WS-B being merged first.

### WS-D -> Test agent (E2E, prompt-flow)

**Briefing**: You are creating four new E2E specs that exercise the agent harness through prompt-driven tool-call flows. Follow the pattern from `chat-harness-send-stream.spec.ts`: `resetApp()`, navigate to `/chat`, type into composer, send, wait for reply. Use `llmKeywordRules` behavior key to configure deterministic tool-call triggers (see `scripts/mock-api/routes/llm.mjs` lines 430-456 for the keyword rule format). Use `llmForcedResponses` for multi-turn sequences where the first response is a tool call and the second is the final answer. Specs: (1) `harness-composio-tool-flow.spec.ts` — "check my email" triggers GMAIL_GET_MAIL tool, composio execute returns canned result, assistant relays it. (2) `harness-cron-prompt-flow.spec.ts` — "remind me every morning" triggers cron_create tool call, verify cron created via oracle RPC. (3) `harness-search-tool-flow.spec.ts` — "what did we discuss about X" triggers memory_search tool call. (4) `harness-channel-bridge-flow.spec.ts` — Telegram inbound triggers tool calls (depends on WS-A/B). For specs 1-3, no dependency on other workstreams. For spec 4, wait for WS-A+B. Add helpers to `chat-harness.ts`: `waitForToolCallInMockLog(toolName, timeoutMs)` polls `getRequestLog()` for a POST to the composio execute or LLM endpoint containing the tool name.

---

## 5. Blocking Unknowns

1. **Telegram API base URL override**: Does any config-loading code cache the URL before the env var is set? Need to verify `TelegramChannel::new()` is called after env is loaded. Likely fine since `start_channels()` runs after config load, but WS-B agent should verify.

2. **Channel connect via RPC in E2E**: Does `openhuman.channels_connect` with `authMode: "bot_token"` actually start the long-polling loop against the mock? If so, `getUpdates` requests will immediately start hitting the mock server. The mock must handle rapid polling gracefully (return empty `[]` by default). WS-A agent should ensure `getUpdates` returns `{ ok: true, result: [] }` when the queue is empty without blocking.

3. **In-process core + mock Telegram**: The E2E app runs the core in-process. The core's Telegram provider will poll `http://127.0.0.1:18473/bot<token>/getUpdates`. The mock server must be ready before the channel connects. Spec `before()` must call `startMockServer()` before `channels_connect`.

4. **Tool execution in E2E harness**: When the mock LLM returns a tool call, does the in-process core actually execute the tool (e.g., call composio execute endpoint, call cron_create)? This depends on the tool being registered in the agent's tool registry. If tools are not available in E2E mode, WS-D specs may need to assert at the LLM mock level only (verifying the tool call was attempted, not executed). The WS-D agent should test this empirically and adapt.

---

## 6. Parallelism Summary

```
WS-A (mock backend)  ──────────────────────────┐
                                                 ├──► WS-C (telegram E2E spec)
WS-B (Rust API base override) ─────────────────┘
                                                 ├──► WS-D spec 4 (channel bridge)
WS-D specs 1-3 (composio/cron/search prompts) ──── independent, no blockers
```

WS-A and WS-B can run fully in parallel.
WS-D specs 1-3 can run in parallel with WS-A and WS-B.
WS-C and WS-D spec 4 must wait for both WS-A and WS-B.
