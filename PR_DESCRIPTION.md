## Summary

- Fix stacked socket listeners causing duplicate AI message bubbles on reconnect
- Route chat through local Ollama model when active — zero cloud tokens on the local path
- Multi-bubble delivery: assistant replies split into 2–5 natural chat bubbles with typing pauses
- Contextual emoji guidance in system prompts — replaces overuse with intentional, human-like usage
- 402 unit tests passing

## Problem

**Duplicate messages** — `subscribeChatEvents` was declared `async` with no actual `await` inside. The cleanup function returned by `.then()` lands in a microtask; React's synchronous cleanup had already run by then, so socket listeners were never removed. Each reconnect stacked another listener set, causing `chat:done` to fire N times and produce N copies of every reply.

**Responses felt like AI slop** — replies arrived as a single wall of text. No typing feel, no natural pacing.

**Ollama was wired for utilities but not chat** — `local_ai_status`, `suggest_questions`, TTS, and Whisper all used Ollama already, but the main conversation loop always hit the cloud socket regardless of whether a local model was running.

**Generic emoji overuse** — the SOUL prompt said "Enthusiastic / Witty / Genuinely human" and the only emoji rule was "Minimal — match user's style". The model interpreted this as license to add 😄🔥✨ on every message.

## Solution

### 1 — Socket listener race condition

`subscribeChatEvents` no longer uses `async` — the cleanup fn is returned synchronously so React can always call it. The `useEffect` stores it with `const cleanup = subscribeChatEvents(...)` and returns it directly.

### 2 — Rust: `openhuman.local_ai_chat` RPC

- `ollama_api.rs` — `OllamaChatMessage` / `OllamaChatRequest` / `OllamaChatResponse` for `/api/chat`
- `service/public_infer.rs` — `LocalAiService::chat_with_history()`: sends full message history to Ollama, updates latency/TPS status
- `ops.rs` — `local_ai_chat` async op
- `schemas.rs` — registered controller: schema, handler, param deserialization

### 3 — Frontend: local chat gate + multi-bubble delivery

When `isLocalModelActive` is true, `handleSendMessage` takes the local path:

```
User sends message
       │
       ├── isLocalModelActive = true
       │     openhumanLocalAiChat(history)  ──►  Ollama /api/chat
       │     deliverLocalResponse()               (no socket, zero cloud tokens)
       │       segmentMessage() → 2–5 bubbles
       │       dispatch each bubble with typing pause (500–1 400 ms)
       │
       └── isLocalModelActive = false
             chatSend() ──► socket chat:start ──► backend ──► cloud API
             (unchanged)
```

Socket-connected guard is skipped on the local path so offline use works.

### 4 — Emoji rules in system prompts

`SOUL.md` adds an explicit **Emoji Rules** section:

- Hard cap: one emoji max per message (none is always fine)
- Contextual, not decorative — must reinforce the specific content
- Never as a sentence opener; only at the end of a clause
- Skip entirely in errors, warnings, technical content, lists, or replies > 3 sentences
- Mirror the user's own emoji usage
- Concrete bad/good examples for model calibration

`BOOTSTRAP.md` Communication Preferences entry updated to reference these rules.

## Files Changed

| File | Change |
|---|---|
| `app/src/services/chatService.ts` | Remove `async` from `subscribeChatEvents`; synchronous cleanup |
| `app/src/pages/Conversations.tsx` | Local chat gate, `deliverLocalResponse`, updated socket effect |
| `app/src/hooks/useLocalModelStatus.ts` | Polls `local_ai_status` every 12 s; returns `true` when `state === "ready"` |
| `app/src/utils/messageSegmentation.ts` | `segmentMessage()` + `getSegmentDelay()` |
| `app/src/utils/tauriCommands.ts` | `openhumanLocalAiChat()` RPC wrapper |
| `app/src/store/threadSlice.ts` | `addReaction` reducer; `activeThreadId` tracking |
| `src/openhuman/local_ai/ollama_api.rs` | Ollama `/api/chat` types |
| `src/openhuman/local_ai/service/public_infer.rs` | `chat_with_history()` method |
| `src/openhuman/local_ai/ops.rs` | `local_ai_chat` op |
| `src/openhuman/local_ai/schemas.rs` | Controller registration |
| `src/openhuman/agent/prompts/SOUL.md` | Emoji Rules section |
| `src/openhuman/agent/prompts/BOOTSTRAP.md` | Updated emoji preference line |

## Submission Checklist

- [x] **Bug fix** — duplicate message root cause identified and fixed
- [x] **Local-only gate** — cloud path entirely unchanged; no increase in billed token usage
- [x] **Rust compiles** — `cargo check --manifest-path Cargo.toml` clean
- [x] **TypeScript** — `tsc --noEmit` 0 errors
- [x] **Tests** — 402 passing (`yarn test`); new files: `messageSegmentation.test.ts` (14 tests), `localChatGating.test.ts` (9 tests)
- [x] **Commits** — 4 atomic commits, one per concern

## Test validation

```bash
cargo check --manifest-path Cargo.toml
cd app && yarn test
yarn tsc --noEmit
```

## Related

- Follow-up: server-side reaction sync for multi-user channel modes
- Follow-up: streaming Ollama responses (currently non-streaming for simplicity)
