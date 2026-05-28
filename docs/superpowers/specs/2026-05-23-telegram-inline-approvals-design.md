# Telegram remote-control phase 2 — inline approvals

**Issue**: [#1805](https://github.com/tinyhumansai/openhuman/issues/1805) (phase 2 of N)
**Date**: 2026-05-23
**Status**: Approved for implementation
**Phase 1**: [PR #2249](https://github.com/tinyhumansai/openhuman/pull/2249) (merged) — `/status`, `/sessions`, `/new`, `/help`

---

## Goal

When `ApprovalGate` parks a tool call, every Telegram chat that has a session binding receives an inline-button prompt; whoever (or whatever surface) decides first wins, and all chats' prompts are edited to reflect the final state with attribution. Add a `/pending` command for on-demand recovery when the auto-broadcast was missed.

## Non-goals

- **Agent Q&A elicitation.** Free-form "ask the agent a question" already works via plain reply to the bound thread. This slice does not introduce a structured question/answer primitive.
- **Active expiry sweeping.** Buttons may go stale; tapping a stale button is the path that surfaces "expired" to the user. No background sweep task.
- **Real Telegram automation in CI.** Closest deterministic harness lives in the existing Rust integration test against `mockito`.
- **Other slices in #1805** (live activity ticks, `/abort`/`/detach`, scheduled tasks, file browsing, mode/model switching, worktree). Each will be its own spec → plan → PR cycle.

---

## Architecture

```
                     ┌──────────────────┐
                     │  ApprovalGate    │  intercepts external-effect tool calls
                     │   (existing)     │  parks future on oneshot
                     └────────┬─────────┘
                              │ publishes
                              ▼
                ┌───────────────────────────────┐
                │ DomainEvent::ApprovalRequested│
                └─────────────┬─────────────────┘
                              │ event bus
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
┌────────────────┐  ┌──────────────────────┐  ┌────────────────┐
│  Desktop UI    │  │ TelegramApproval-    │  │ (future        │
│  (existing)    │  │  Subscriber (new)    │  │  providers)    │
└───────┬────────┘  └──────────┬───────────┘  └────────────────┘
        │                      │
        │                      │ for every bound chat:
        │                      │   sendMessage + inline_keyboard
        │                      │   record (chat_id, msg_id) in pending_map
        │                      ▼
        │             ┌─────────────────┐
        │             │  Telegram Bot   │
        │             │     API         │
        │             └────────┬────────┘
        │                      │ user taps button
        │                      ▼
        │             ┌─────────────────────────┐
        │             │ channel_recv.rs:        │
        │             │  callback_query handler │  ← allowlist re-check
        │             │  (new branch)           │
        │             └────────────┬────────────┘
        │                          │ approval_decide(
        │                          │   request_id, decision,
        │                          │   actor=@user, surface="telegram")
        ▼                          ▼
       approval_decide RPC  ───────┐
                                   │ publishes
                                   ▼
                 ┌──────────────────────────────────┐
                 │ DomainEvent::ApprovalDecided      │
                 │  (extended w/ decided_by_actor + │
                 │   decided_by_surface)            │
                 └────────────────┬─────────────────┘
                                  │
                                  ▼
                 TelegramApprovalSubscriber.handle()
                   for each (chat_id, msg_id) in pending_map:
                     editMessageText("✓ decided by @who via X")
                     [remove inline_keyboard]
                   pending_map.remove(request_id)
```

### Key design choices

| Choice | Decision | Rationale |
|---|---|---|
| **Routing** | Broadcast to all chats with a session binding | Mirrors multi-device approval UX; first-decision-wins; `pending_map` carries the chat list for the follow-up edit. |
| **Pending → message-ids map** | In-memory inside `TelegramApprovalSubscriber` | After a core restart, taps on still-visible buttons fail gracefully ("Already decided" toast). Persistence would cost IO + schema churn for an edge case. |
| **Stale tap UX** | Toast via `answerCallbackQuery` + best-effort message edit | Operator never sees a "dead button" with no feedback. |
| **Cross-chat sync** | Subscribe to `ApprovalDecided`; edit every recorded posted message | Visibility into who acted and from where. |
| **Decider identity** | Extend `ApprovalDecided` event + `approval_decide` RPC + `ApprovalAuditEntry` with optional `decided_by_actor` / `decided_by_surface` | Without this, "decided by @who via X" is fiction. Surfaces other than Telegram default to `surface="desktop"`. |
| **Telegram API surface** | New `pub(crate)` methods on `TelegramChannel`: `send_with_inline_keyboard`, `edit_message_text`, `answer_callback_query` | Telegram-specific primitives stay in the provider; the generic `Channel` trait isn't dragged into approvals. |
| **Subscriber → callback bridge** | `static GLOBAL_APPROVAL_SUB: OnceLock<Arc<TelegramApprovalSubscriber>>` | Same singleton pattern as `ApprovalGate::try_global`; `channel_recv.rs` reaches the subscriber without restructuring the long-poll loop. |
| **`/pending` command** | One chat, on demand; reuses the same send + button + map registration path | Critical when the auto-broadcast was missed. Negligible extra code. |
| **Expiry handling** | Render `_Expires in Xm_` when `expires_at` is set; no background sweep; stale taps self-correct | Adding a sweep is its own bucket of work. |

---

## Files

### New

| File | Purpose | Approx LOC |
|---|---|---|
| `src/openhuman/channels/providers/telegram/approval_bus.rs` | `TelegramApprovalSubscriber` — `EventHandler` on `approval` domain. Holds `Arc<TelegramChannel>`, `workspace_dir`, `pending_map: parking_lot::Mutex<HashMap<String, Vec<PostedPrompt>>>`. Provides `decide_from_callback(...)` for `channel_recv.rs`. Exposes `set_global`/`try_global` over a `OnceLock`. | ~250 |
| `src/openhuman/channels/providers/telegram/approval_bus_tests.rs` | Unit tests for the subscriber (see Testing). | ~300 |

### Modified — Telegram provider

| File | Change |
|---|---|
| `src/openhuman/channels/providers/telegram/mod.rs` | `mod approval_bus;` + `pub use approval_bus::TelegramApprovalSubscriber;` + `#[cfg(test)] #[path = "approval_bus_tests.rs"] mod approval_bus_tests;`. |
| `src/openhuman/channels/providers/telegram/channel_send.rs` | Three new `pub(crate) async` methods on `TelegramChannel`: `send_with_inline_keyboard(chat_id, text, keyboard) -> Result<i64>`, `edit_message_text(chat_id, message_id, text, keyboard: Option<&Value>) -> Result<()>`, `answer_callback_query(callback_query_id, text, show_alert) -> Result<()>`. |
| `src/openhuman/channels/providers/telegram/channel_recv.rs` | New top-of-loop branch: `if let Some(cb) = update.get("callback_query") { self.handle_callback_query(cb).await; return; }`. New `handle_callback_query` method (allowlist re-check, parse `appr:<o\|a\|d>:<rid>`, dispatch). |
| `src/openhuman/channels/providers/telegram/remote_control.rs` | Add `TELEGRAM_CMD_PENDING = "/pending"` const + parser arm + `build_pending_response(...)` async builder. The builder uses the subscriber (`TelegramApprovalSubscriber::try_global()`) to broadcast prompts for currently-pending approvals into the calling chat only, registering them in `pending_map`. Help text updated. |

### Modified — core / approval domain

| File | Change |
|---|---|
| `src/core/event_bus/events.rs` | `DomainEvent::ApprovalDecided` gains `decided_by_actor: Option<String>`, `decided_by_surface: Option<String>`. |
| `src/openhuman/approval/types.rs` | `ApprovalAuditEntry` gains the same two optional fields. |
| `src/openhuman/approval/store.rs` | SQLite migration: `ALTER TABLE approval_audit ADD COLUMN decided_by_actor TEXT; ALTER TABLE approval_audit ADD COLUMN decided_by_surface TEXT;`. Read/write paths updated. |
| `src/openhuman/approval/gate.rs` | `decide()` gains `actor: Option<String>`, `surface: Option<String>`; threaded into the audit insert and the published event. |
| `src/openhuman/approval/rpc.rs` | `approval_decide` RPC gains two optional params. When both are absent, default `surface = Some("desktop")` for back-compat with older app builds. |
| `src/openhuman/approval/schemas.rs` | Schema definitions updated to reflect the optional params. |

### Modified — runtime wiring

| File | Change |
|---|---|
| `src/openhuman/channels/runtime/startup.rs` | After the existing `_telegram_remote_handle` block: if `channels_by_name.get("telegram")` downcasts to `TelegramChannel`, construct + `set_global` + subscribe the new `TelegramApprovalSubscriber`. On downcast failure, log error + continue (approvals fall back to desktop). |

A small surface change may be needed to make the `Arc<dyn Channel>` → `Arc<TelegramChannel>` conversion work. Acceptable options, ordered by preference:

1. **Helper free function** in `src/openhuman/channels/providers/telegram/mod.rs`:
   ```rust
   pub fn try_downcast(arc: &Arc<dyn Channel>) -> Option<Arc<TelegramChannel>> { ... }
   ```
   No trait changes; only the call site at `startup.rs` knows about it.
2. **`as_any` on `Channel` trait**, then `Arc::downcast` via `Arc<dyn Any>`. Slightly larger blast radius.

The plan should pick (1) unless implementation reveals a blocker.

### Modified — capability catalog

| File | Change |
|---|---|
| `src/openhuman/about_app/catalog.rs` | Extend the existing Telegram remote-control entry with `"Inline approval / deny / approve-always buttons"` + `"/pending"` capability strings. |

### Modified — frontend

| File | Change |
|---|---|
| `app/src/services/api/approvalApi.ts` (or wherever the `approval_decide` wrapper lives) | Add optional `actor` / `surface` params; default `surface: "desktop"` on desktop callers. |
| `app/src/services/api/__tests__/approvalApi.test.ts` | One test asserting the wrapper passes `surface: "desktop"` when caller omits both. |
| `app/src/components/channels/TelegramConfig.tsx` | A single status-line addition under existing remote-control copy: "Remote approvals: enabled (always-on when Telegram is connected)". No new toggle. |
| `app/src/components/channels/__tests__/TelegramConfig.test.tsx` | Snapshot/render assertion for the new copy. |

### Modified — E2E / integration

| File | Change |
|---|---|
| `tests/json_rpc_e2e.rs` | New scenario `approval_decide_accepts_actor_and_surface_optional_params` — call with both fields present and absent. |
| `src/openhuman/channels/tests/telegram_integration.rs` | New scenario `approval_round_trip_via_callback_query` — boot TelegramChannel + subscriber against mockito; bind one chat; publish synthetic `ApprovalRequested`; assert mock receives `sendMessage` with inline keyboard; feed synthetic `callback_query`; assert `approval_decide` → `ApprovalDecided` → mock receives `editMessageText` + `answerCallbackQuery`. |

---

## Data flow

### Subscriber registration (startup)

`src/openhuman/channels/runtime/startup.rs`, immediately after `_telegram_remote_handle`:

```rust
let _telegram_approval_handle = match channels_by_name.get("telegram")
    .and_then(crate::openhuman::channels::providers::telegram::try_downcast)
{
    Some(tg) => {
        let sub = Arc::new(TelegramApprovalSubscriber::new(
            tg,
            config.workspace_dir.clone(),
        ));
        TelegramApprovalSubscriber::set_global(Arc::clone(&sub));
        let handle = bus.subscribe(sub);
        tracing::debug!("[telegram-approval] registered TelegramApprovalSubscriber");
        Some(handle)
    }
    None => {
        if channels_by_name.contains_key("telegram") {
            tracing::error!("[telegram-approval] channel present but downcast failed; approvals via Telegram disabled");
        }
        None
    }
};
```

### `ApprovalRequested` → broadcast

```rust
async fn handle(&self, event: &DomainEvent) {
    match event {
        DomainEvent::ApprovalRequested { request_id, tool_name, action_summary, .. } => {
            let bindings = self.load_bindings_snapshot();
            if bindings.is_empty() {
                tracing::debug!("[telegram-approval] no bound chats; skip id={request_id}");
                return;
            }

            // Pre-insert empty vec so a racing ApprovalDecided event finds a
            // (possibly partial) list rather than misses entirely.
            self.pending_map.lock().insert(request_id.clone(), Vec::new());

            let text     = render_prompt(tool_name, action_summary, *expires_at);
            let keyboard = approval_keyboard(request_id, tool_name);

            for binding in bindings {
                match self.channel.send_with_inline_keyboard(&binding.chat_id, &text, &keyboard).await {
                    Ok(message_id) => {
                        self.pending_map.lock()
                            .get_mut(request_id)
                            .map(|v| v.push(PostedPrompt { chat_id: binding.chat_id.clone(), message_id }));
                    }
                    Err(err) => tracing::warn!(
                        "[telegram-approval] send failed chat={} id={} err={}",
                        binding.chat_id, request_id, err,
                    ),
                }
            }
        }
        DomainEvent::ApprovalDecided { request_id, decision, decided_by_actor, decided_by_surface, .. } => {
            let posted = match self.pending_map.lock().remove(request_id) {
                Some(p) => p,
                None => {
                    tracing::debug!("[telegram-approval] decided event for unknown id={request_id}");
                    return;
                }
            };
            let final_text = render_decided(decision, decided_by_actor.as_deref(), decided_by_surface.as_deref());
            for p in posted {
                if let Err(err) = self.channel.edit_message_text(&p.chat_id, p.message_id, &final_text, None).await {
                    tracing::warn!(
                        "[telegram-approval] edit failed chat={} msg={} err={}",
                        p.chat_id, p.message_id, err,
                    );
                }
            }
        }
        _ => {}
    }
}
```

`approval_keyboard(request_id, tool_name)` returns three rows:

```json
[
  [{"text": "✅ Approve once",           "callback_data": "appr:o:<rid>"}],
  [{"text": "♻ Always for <tool>",       "callback_data": "appr:a:<rid>"}],
  [{"text": "❌ Deny",                   "callback_data": "appr:d:<rid>"}]
]
```

`callback_data` is limited to 64 bytes by Telegram. `appr:<o|a|d>:<uuid>` is comfortably under.

### Inbound `callback_query` → decide

In `channel_recv.rs`:

```rust
if let Some(cb) = update.get("callback_query") {
    if let Err(err) = self.handle_callback_query(cb).await {
        tracing::warn!("[telegram][callback] handler error: {err}");
    }
    return;
}
// (existing message handler follows)
```

`handle_callback_query`:

1. Parse `id`, `from.username`, `from.id`, `message.chat.id`, `message.message_id`, `data`.
2. **Re-check allowlist** with `is_any_user_allowed([username_norm, id_norm])`. Non-allowlisted → `answer_callback_query(id, "Not authorized", show_alert=true)`; log; return.
3. Parse `data` as `appr:<o|a|d>:<rid>`. Malformed → toast "Invalid action"; return.
4. Map to `ApprovalDecision::{ApproveOnce, ApproveAlwaysForTool, Deny}`.
5. Call `approval_decide(rid, decision, actor=Some(@username), surface=Some("telegram"))`.
6. On `Err` whose source resolves to "no pending approval found" → toast "Already decided or expired"; best-effort `edit_message_text(chat_id, message_id, "⏱ already decided or expired", None)`.
7. On `Ok(_)` → toast "Decision recorded". The bus-driven edit (`ApprovalDecided` branch) updates message text in all bound chats.

### `/pending` command

In `remote_control.rs`:

1. `approval_list_pending().await?` → `Vec<PendingApproval>`.
2. Empty → reply `_No approvals pending._`.
3. Non-empty → for each row (capped at 5):
   - **Dedupe per chat**: if `pending_map[request_id]` already has a `PostedPrompt` whose `chat_id` matches the calling chat, skip — the operator already has a live prompt in this chat from the auto-broadcast.
   - Otherwise, construct prompt + keyboard exactly like the auto-broadcast path; call `subscriber.channel.send_with_inline_keyboard(...)` against **this chat only**; on success push `PostedPrompt` into `pending_map[request_id]` (creating the entry if absent — `/pending` may run for a request still parked but never broadcast, e.g. across a core restart).
4. If `> 5` rows: append a `_… and N more (open the desktop app to see all)_` line.

### Decider identity propagation

- `approval_decide` JSON-RPC body extends from `{ request_id, decision }` to `{ request_id, decision, actor?, surface? }`.
- `ApprovalGate::decide(...)`: signature gains `actor: Option<String>, surface: Option<String>`; values are passed through to `pending_approvals` row mutation + `approval_audit` row insert + the published `ApprovalDecided` event.
- SQLite migration: two new nullable TEXT columns on `approval_audit`. Existing rows keep `NULL` (which renders as "decided" with no "by" suffix — back-compat for any historical UI that re-reads audit).

---

## Error handling

| Failure | Behavior |
|---|---|
| `send_with_inline_keyboard` fails for one chat during broadcast | Log + skip; other chats still get a working prompt; `pending_map` records only successes. |
| All sends fail | Log error. Desktop UI still functions. No retry — single event, single attempt. |
| `edit_message_text` fails on a posted prompt | Log + continue editing the rest. Stale message remains; the next tap triggers the "already decided" self-correction path. |
| `approval_decide` returns "gate not installed" | Toast "Approvals disabled" + log. Defensive; subscriber would not have been registered if the gate is absent in a normal boot. |
| `approval_decide` returns "no pending approval found" | Toast "Already decided or expired" + best-effort edit on the tapped message. |
| Callback `data` malformed | Toast "Invalid action" + log. Poll loop continues. |
| Channel downcast fails at startup | Log error; subscriber not registered. Approvals fall back to desktop-only. Core continues. |
| SQLite migration fails (existing prod row count > 0, column add fails) | Surfaces during gate boot via the existing migration runner. No new logic here — leverage the existing migration error path. |

## Edge cases

| Case | Behavior |
|---|---|
| Chat binds via `/new` *after* `ApprovalRequested` was published | Not in bindings snapshot at broadcast time → no prompt. `/pending` covers the gap. |
| Chat is unbound between request and decision | Edits still post to the recorded `(chat_id, message_id)`; Telegram doesn't care about our `bindings` state. |
| Group chat: bound by allowlisted operator + has non-allowlisted members | Callback re-checks `from.username` / `from.id`; only allowlisted taps proceed. |
| Two operators tap simultaneously | First `approval_decide` wins; second hits "no pending approval found" → toast + self-correcting edit. |
| Same operator taps Approve-once then Deny in quick succession | Same path — second tap loses. |
| Core restart between request and decision | `pending_map` is lost. Old buttons in Telegram still call back. Allowlist still works (loaded from config). `approval_decide` returns "no pending approval found" → toast + tap-local edit only. Other chats stay stale. **Acceptable**: the parked future itself was lost on restart already; this slice doesn't change that. |
| `expires_at` reached while message is live | No active expiry sweep. The next tap returns "no pending approval found" and we render `⏱ already decided or expired`. |
| `mention_only` mode is on | Does not apply to `callback_query` — those are direct button taps, not message mentions. Always processed if allowlist passes. |
| `pairing_code_active` (no chats bound yet) | Broadcast path early-returns (`bindings.is_empty()`). Future approvals reach chats once binding completes. |
| Concurrent `ApprovalRequested` + `ApprovalDecided` ordering | Mitigated by pre-inserting an empty `Vec` into `pending_map` *before* awaited sends, so a `Decided` arriving mid-broadcast still finds an entry (possibly partial) to drain and remove. |

## Security

| Concern | Mitigation |
|---|---|
| Non-allowlisted user taps a button | Re-check `from.username` + `from.id` against the allowlist on every callback. Non-allowlisted → toast "Not authorized" + log. Identical to the message-path check. |
| `callback_data` spoofing | Telegram guarantees `from` on `callback_query` is the real user; allowlist re-check covers spoofed senders. `request_id` is still validated against the gate, which checks session + existence. |
| PII / secrets in prompt body | `action_summary` and `args_redacted` are already scrubbed by `approval/redact.rs`. The prompt body uses only `action_summary` — no raw args, no chat content. |
| Logging | Stable prefix `[telegram-approval]`. Log `request_id`, `tool_name`, `decision`, `chat_id`, `actor`, `surface`. Never log message bodies, args, or full `action_summary`. |
| Bot token in logs | Existing telegram code redacts; new send helpers reuse the existing `api_url(...)` helper, which already keeps the token out of error strings. |
| Callback flood from a misbehaving client | Allowlist gates this. No per-user rate limiting in this slice — out of scope. |

---

## Testing

Target: ≥80% diff coverage on changed lines. Behavior over implementation. No real network.

### Rust unit — `approval_bus_tests.rs` (new)

Built on a fake `TelegramChannel` (constructor takes a `mockito` URL; send helpers wrap `reqwest` against it — same pattern in existing `channel_tests.rs`). Each test uses an isolated `tempdir` for `workspace_dir` + `TelegramSessionStore`.

| Test | What it proves |
|---|---|
| `broadcasts_to_all_bound_chats` | Two bound chats; publish `ApprovalRequested`; mock receives two `sendMessage` calls with matching inline_keyboard; `pending_map` has 2 entries. |
| `no_bound_chats_skips_broadcast` | Zero bindings → zero sends, no map entry, no panic. |
| `partial_send_failure_still_records_successes` | First chat 200, second chat 500. `pending_map` has only the successful chat. Warn logged. |
| `pre_insert_avoids_lost_decided_race` | Confirm `pending_map.insert(rid, Vec::new())` runs **before** awaited sends; simulate `ApprovalDecided` mid-broadcast and assert no missed-entry log. |
| `edits_all_chats_on_decided` | Two bound; broadcast; `ApprovalDecided{actor=Some("@op"), surface=Some("telegram")}` → two `editMessageText` calls; body contains "approved by @op via telegram"; keyboard removed. |
| `decided_event_for_unknown_id_is_noop` | `Decided` for an id not in `pending_map` → zero sends, debug log. |
| `decided_event_renders_surface_unknown_gracefully` | `actor=None, surface=None` → body says "approved" with no "by" suffix. |
| `expires_in_rendered_when_set` | `ApprovalRequested` with `expires_at = now+300s` → outgoing text contains `Expires in 5m`. With `expires_at=None` → no expiry line. |
| `prompt_text_does_not_leak_args` | Pass `action_summary` containing only a redacted summary; assert outgoing text contains *only* that, no `args_redacted`. |

### Rust unit — `channel_recv` callback_query (extended `channel_tests.rs`)

| Test | What it proves |
|---|---|
| `callback_query_from_allowed_user_dispatches_decide` | Build update with `data="appr:o:<rid>"`, allowed user; stub `approval_decide`; assert one call with `decision=ApproveOnce, actor=Some("@user"), surface=Some("telegram")`. |
| `callback_query_from_unauthorized_user_is_rejected` | Same but unauthorized `from`; assert zero `approval_decide` calls, one `answerCallbackQuery` text≈"Not authorized". |
| `callback_query_malformed_data_toasts_invalid` | `data="garbage"` → no decide, one toast text≈"Invalid". |
| `callback_query_already_decided_toasts_and_attempts_edit` | Stub `approval_decide` → `Err("no pending approval found")` → one toast + one `editMessageText` on the tapped (chat_id, message_id). |
| `callback_query_does_not_fall_through_to_message_handler` | Update with `callback_query` and no `message.text` → existing inbound-message path not invoked; no `ChannelMessageReceived`. |
| `callback_query_allowlist_lookup_uses_id_and_username` | Username not in allowlist but `from.id` is → permitted. Mirrors message-path behavior. |

### Rust unit — `remote_control` `/pending` (extended `remote_control_tests.rs`)

| Test | What it proves |
|---|---|
| `pending_command_with_no_approvals_replies_empty` | Empty `approval_list_pending` → reply ≈"No approvals pending". |
| `pending_command_renders_each_row_with_buttons` | Three pending → three sends with full keyboard; each recorded in `pending_map`. |
| `pending_command_caps_at_5_rows` | Ten pending → first 5 + `_… and 5 more_` line. |
| `pending_command_is_chat_local_not_broadcast` | Calling `/pending` from chat A while B is bound → only A receives prompts. |
| `pending_command_dedupes_per_chat` | `pending_map[rid]` already has a PostedPrompt for chat A; `/pending` in chat A → that row is skipped (zero new sends for that rid), other rids still rendered. |

### Approval-domain tests (extended)

| File | Test |
|---|---|
| `src/openhuman/approval/rpc.rs` (or new `rpc_tests.rs`) | `approval_decide_threads_actor_and_surface_into_event` — call with `Some` values; capture `ApprovalDecided`; assert both propagate. |
| `src/openhuman/approval/store.rs` | `audit_row_round_trips_actor_and_surface`; `audit_row_round_trips_nullable_actor_and_surface`. |
| `src/openhuman/approval/gate.rs` tests | `decide_records_actor_and_surface_to_audit` — end-to-end through the gate. |

### JSON-RPC E2E — `tests/json_rpc_e2e.rs`

| Scenario | What it proves |
|---|---|
| `approval_decide_accepts_actor_and_surface_optional_params` | Hit `approval_decide` over JSON-RPC with both fields, `Some` and absent. Both succeed. Existing response shape preserved (additive only). |

### Frontend tests

| File | Test |
|---|---|
| `app/src/services/api/__tests__/approvalApi.test.ts` | `approval_decide_passes_surface_desktop_by_default` — caller omits both → wrapper sends `surface: "desktop"`. |
| `app/src/components/channels/__tests__/TelegramConfig.test.tsx` | New "Remote approvals: enabled …" copy renders when Telegram is connected. |

### Deterministic Telegram E2E — `telegram_integration.rs`

`approval_round_trip_via_callback_query`:

1. Boot TelegramChannel + TelegramApprovalSubscriber against `mockito`.
2. Bind one chat via `/new` (mock returns success).
3. Publish synthetic `ApprovalRequested`.
4. Assert mock received `sendMessage` with inline_keyboard.
5. Feed synthetic `callback_query` with valid `data` from allowed user.
6. Assert `approval_decide` invoked → `ApprovalDecided` published → mock received `editMessageText` + `answerCallbackQuery`.

This is the closest deterministic equivalent to real Telegram automation the project supports today.

### Coverage estimate

| Area | New lines (approx) | Tested lines |
|---|---|---|
| `approval_bus.rs` | ~250 | ≥240 |
| `channel_send.rs` (3 new helpers) | ~120 | ≥110 |
| `channel_recv.rs` callback branch | ~80 | ≥75 |
| `remote_control.rs` `/pending` | ~60 | ≥55 |
| `approval/gate.rs` + `store.rs` + `rpc.rs` + event + schemas | ~70 | ≥65 |

Diff coverage should land comfortably above 80%; the uncovered tail is defensive error-logging arms.

---

## Acceptance criteria (mapping to #1805)

This slice satisfies the following acceptance criteria from #1805:

- ☑ "Users can answer agent questions and **approve/deny permission requests** from Telegram inline." (permission half only — Q&A is non-goal for this slice.)
- ☑ "Core functionality is exposed through registered controllers and schemas, not Telegram-specific branches in transport adapters." — Telegram only adds an event-bus subscriber that calls existing `approval_decide` RPC; no Telegram branches in `approval/` domain.
- ☑ "`src/openhuman/about_app/` is updated so the capability catalog reflects the expanded Telegram feature set."
- ☑ "New/changed flows include substantial debug logging in Rust and app code with grep-friendly prefixes and no secret leakage." — `[telegram-approval]` prefix throughout.
- ☑ "Rust unit/integration coverage is added for the new Telegram control flows and JSON-RPC/controller surfaces."
- ☑ "App unit tests are added for Telegram configuration/management UI changes." (minimal, scoped to actual changes.)
- ☑ "Desktop E2E coverage … or the nearest deterministic harness equivalent" — covered by the `telegram_integration.rs` scenario.
- ☑ "Diff coverage ≥ 80%."

Out of scope for this slice (to be addressed in later phases):
- live activity ticks during runs / `/abort` / `/detach`
- `/task` / `/tasklist`
- `/files` / `/ls`
- inbound attachments / structured outbound file delivery
- `/models` / `/mode`
- `/projects` / `/worktree`

---

## Implementation order (for the plan)

1. **Core/approval changes first** — extend event, RPC, gate, audit, store + tests. Independently mergeable.
2. **Telegram send helpers** — `send_with_inline_keyboard`, `edit_message_text`, `answer_callback_query` + tests against mockito.
3. **`TelegramApprovalSubscriber`** + unit tests.
4. **`channel_recv.rs` callback branch** + unit tests.
5. **`/pending` command** + unit tests.
6. **Wiring in `startup.rs`** + downcast helper.
7. **Catalog update** + frontend touch + tests.
8. **`telegram_integration.rs` round-trip** + `json_rpc_e2e.rs` scenario.

---

## Risks

| Risk | Mitigation |
|---|---|
| Existing desktop frontend calls `approval_decide` without the new params | Server defaults `surface = Some("desktop")` when both are absent. Frontend update is additive. |
| `Arc<dyn Channel>` → `Arc<TelegramChannel>` downcast may need a small trait change | Plan covers a `try_downcast` helper option first; falls back to `as_any` only if needed. |
| Real Telegram users tap stale buttons after core restart | Acceptable — "already decided or expired" toast + best-effort edit covers it. |
| `callback_data` length cap (64 bytes) is exceeded by long request_ids | Request ids are uuids (36 bytes); `appr:<o|a|d>:` adds 7. Total 43. Comfortable headroom. Validate at construction time in `approval_keyboard` to fail loud if this ever changes. |

---

## Open questions

None at design approval. Implementation may surface:
- Exact mockito URL injection pattern in the new `approval_bus_tests.rs` — defer to plan-writing to confirm the existing test harness shape carries over.
- Whether the frontend `approval_decide` wrapper currently lives in `services/api/` or somewhere else — defer to a brief grep at plan-writing time.
