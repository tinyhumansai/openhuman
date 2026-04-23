# Life-Capture Layer — Design

**Date:** 2026-04-22
**Status:** Drafted, awaiting user review
**Scope:** Track 1 (unblock ship pipeline) + Track 2 (life-capture spine: continuous ingestion → local personal index → Today brief + retrieval-tuned chat). Tracks 3+ deferred.

## Problem

OpenHuman today is "another chat UI with skills + memory." Competent, not a buy. ChatGPT desktop is free and has memory; Hermes Agent (MIT, ~95k stars in 7 weeks) covers the same architectural pillars; the QuickJS skills runtime is a real edge but invisible to non-developer users. We need a product moment where someone installs it, uses it for a day, and tells a friend.

The user's framing: OpenHuman should be **"the layer that captures everything from your life"** — and the moments that prove it are (1) a morning brief that already knows what matters today, and (2) instant retrieval over everything you've ever read, written, or said ("what did Sarah say about the Ledger contract three weeks ago"). Both demos share the same spine, so we build one foundation and get two surfaces from it.

## Goals

1. **Ship a magical demo of (a) Today brief and (b) cross-source retrieval with citations** within 4 weeks.
2. **Continuous ingestion** of Gmail, Calendar, and iMessage (Mac); Slack deferred to v1.1 into a single local personal index — not on-demand tool calls.
3. **Three composable privacy modes** (Convenience / Hybrid / Fully local) made visible in a Privacy panel. Same code path, three swappable functions.
4. **Unblock the ship pipeline** so this work can land continuously.
5. **Reuse aggressively** from existing internal projects (`inbox/`, `ambient/`, `neocortex/`) instead of rebuilding.

## Non-goals

- OSS independence work beyond what falls out naturally (BYO LLM key, BYO embeddings provider). Full self-host story is a separate spec.
- Skills marketplace, registry, authoring UX (separate spec).
- Encrypted local index at rest (v2).
- Multi-device sync of the personal index (v2).
- Browser history, file system watching, screen recording (v2 — keep v1 to messaging + calendar + email).
- Direct per-provider OAuth (no BYO Google Cloud OAuth app). Composio remains the broker.
- Any change to the QuickJS skills runtime internals.

## The two demos we are designing backwards from

**Demo 1 — Today brief.** User opens the laptop in the morning. App focuses to a "Today" view that already shows: top 3 things to act on (with one-click drafted replies for the email-shaped ones), the day's meetings with a one-line "what to know" per attendee, and a "you said you'd…" section pulled from anything the user committed to in the last week.

**Demo 2 — "What was that."** User in chat asks "what did Sarah say about the Ledger contract three weeks ago?" The app retrieves across email + Slack + iMessage + calendar notes, returns a direct answer with inline citations, and clicking a citation opens the original message in a side panel.

Both are concrete and demoable. Both must work for the wedge to be real.

## Architecture — the life-capture spine

```
                         ┌──────────────────────────────────┐
  Gmail (Composio)  ───► │                                  │
  Calendar (Composio)─►  │   Ingestion workers (Rust)       │
  Slack (Composio)  ───► │   schedule + delta-sync per src  │ ──► normalize ──► embed ──► write
  iMessage (Mac SQLite)► │                                  │
                         └──────────────────────────────────┘
                                                                       │
                                                                       ▼
                            ┌──────────────────────────────────────────────┐
                            │  personal_index.db  (SQLite + sqlite-vec)    │
                            │  items(id, source, ts, author, subject,      │
                            │        text, metadata_json, vector blob)     │
                            │  refs(item_id, person_id), people(...)       │
                            └──────────────────────────────────────────────┘
                                                                       │
                                       ┌───────────────────────────────┴────────────────┐
                                       ▼                                                ▼
                          ┌──────────────────────┐                     ┌──────────────────────────┐
                          │ Today.compose_brief()│                     │ chat.retrieve_with_cites │
                          │ (scheduled + on-open)│                     │ (RAG into agent loop)    │
                          └──────────────────────┘                     └──────────────────────────┘
                                       │                                                │
                                       ▼                                                ▼
                          new "Today" tab in app nav                       Conversations.tsx, with
                          (Cmd+T, app-focus jumps here)                    inline citations + side panel
```

### Core types (Rust)

```rust
// One canonical item across all sources.
struct Item {
    id: Uuid,
    source: Source,            // Gmail | Calendar | Slack | IMessage
    external_id: String,        // dedupe key per source
    ts: DateTime<Utc>,
    author: Option<Person>,
    subject: Option<String>,    // email subject, calendar title, etc.
    text: String,               // normalized, PII-redactable
    metadata: serde_json::Value,
    vector: Option<Vec<f32>>,   // present after embed step
}

trait Ingestor { async fn delta_sync(&self, since: DateTime<Utc>) -> Result<Vec<Item>>; }

trait Embedder { async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>; }
// Impls: HostedEmbedder { provider, key }, LocalEmbedder { fastembed_handle }

trait IndexWriter { async fn upsert(&self, items: &[Item]) -> Result<()>; }
trait IndexReader {
    async fn search(&self, q: &Query) -> Result<Vec<Hit>>;          // semantic + keyword hybrid
    async fn recent(&self, sources: &[Source], window: Duration) -> Result<Vec<Item>>;
}
```

Module placement (per repo rules — new functionality lives in its own subdirectory):

- `src/openhuman/life_capture/mod.rs` — public re-exports
- `src/openhuman/life_capture/ingestors/{gmail,calendar,slack,imessage}.rs` — one per source
- `src/openhuman/life_capture/index.rs` — SQLite + sqlite-vec store
- `src/openhuman/life_capture/embedder.rs` — Embedder trait + impls
- `src/openhuman/life_capture/brief.rs` — Today brief composer
- `src/openhuman/life_capture/retrieval.rs` — query pipeline used by chat
- `src/openhuman/life_capture/schemas.rs` — controller schema for CLI/JSON-RPC exposure
- `src/openhuman/life_capture/rpc.rs` — RpcOutcome controllers

### Three privacy modes (one code path, swappable functions)

| Mode | Embedder impl | Chat LLM | Raw text leaves the box |
|---|---|---|---|
| Convenience (default) | `HostedEmbedder` (OpenAI/Voyage) | hosted free / your key | Yes — to embeddings provider + LLM provider |
| Hybrid | `LocalEmbedder` (fastembed) | your LLM provider | Only retrieved snippets at chat time |
| Fully local | `LocalEmbedder` | Ollama | Nothing |

Independent of the mode, two local-only mitigations always run before any network call:

- **PII redaction** — local regex replaces emails, phone numbers, and SSN/credit-card-shaped tokens with `<EMAIL>` etc. before embedding. (Light NER is a future enhancement; only regex redaction is implemented today.)
- **Quoted-thread stripping** — index only the new content of an email, not the 14-deep reply quote.

A new Settings → Privacy panel renders the table above with the user's current selections highlighted, and a per-source toggle to disable ingestion entirely for any source.

## Surfaces

### Today tab (new, primary)

- New top-level route `/today`, new nav entry, `Cmd+T` shortcut.
- App-focus behavior: if last view of `/today` was >6h ago OR app cold-started today, route there on launch. Otherwise stay where the user left off.
- Layout: three sections, scannable in 10 seconds.
  1. **Act on** — top 3 items needing a reply or decision; each has a one-click drafted response (drafts live in Gmail's drafts folder, not auto-sent).
  2. **Today's meetings** — chronological list, each with a one-line "what to know" pulled from prior threads with attendees.
  3. **You said you'd…** — commitments parsed from the user's outgoing messages in the last 7 days that have a deadline within the next 7 ("I'll send you the doc by Friday" → surfaces Friday morning).
- Manual refresh button. Background refresh runs hourly while app is open.
- **Cold-start state** (first launch, nothing ingested yet): Today shows a one-screen onboarding that lists which sources are connected, which are still syncing, and a progress bar per source. First brief composes as soon as Gmail + Calendar finish their initial backfill (typically 60-120s).

### Conversations chat (existing surface, augmented)

- Always-RAG mode: every user message runs `retrieval.rs` first; top-k hits are injected into the agent prompt with stable `[1] [2] [3]` markers and a `sources` array.
- Inline citation chips render in the assistant message; clicking a chip opens a right-side panel showing the original Item (full email / message thread / calendar event).
- No new chat surface; this is an augmentation to `Conversations.tsx` and `chat_send_inner`.

## Track 1 — Unblock ship pipeline (week 1, parallel to Track 2 setup)

1. **Fix Ubuntu installer smoke test** in `scripts/install.sh`. Suspected: Python3 parser of `latest.json` racing GitHub release asset propagation, or wrong asset name match for x86_64 Linux. Add deterministic retry-with-backoff and fail loudly with the resolved URL on parse error.
2. **Land in-flight PRs** in order: #806, #786, retrigger #788, debug #797 (`resolve_dirs_uses_active_user_when_present` config dir mismatch).
3. **Wire Tauri auto-updater + signed builds.** Flip `tauri.conf.json` `updater.active`, configure endpoint + pubkey, sign macOS (Developer ID) and Windows in the release workflow.

Verification: smoke test green on 3 consecutive PR pushes; an installed v(N) auto-prompts and applies v(N+1) end-to-end on Mac and Windows.

## Track 2 — Life-capture spine (weeks 1-4)

### Week 1 — Foundation
- `personal_index.db` schema + migrations + `IndexWriter`/`IndexReader` impls over SQLite + sqlite-vec.
- `Embedder` trait + `HostedEmbedder` (OpenAI text-embedding-3-small as default, Voyage as alternative).
- Settings → AI panel extension: provider dropdown for embeddings (initially: Hosted default / OpenAI key / Voyage key).
- PII-redaction + quoted-thread-strip utility (local-only, well-tested).

### Week 2 — Ingestors
- `GmailIngestor` and `CalendarIngestor` reusing OAuth tokens already brokered through Composio. Delta-sync via Gmail history API and Calendar `updatedMin`.
- `IMessageIngestor` (Mac only) — read-only access to `~/Library/Messages/chat.db` via SQLite, requires Full Disk Access permission with an in-app explainer screen. **Reuse heavily from `~/projects/inbox/inbox_client.py` and `~/projects/inbox/mcp_backend.py`** — port the iMessage SQLite reader logic to Rust rather than reinventing it.
- Worker scheduler in the Rust core: runs each ingestor at its own cadence (Gmail/Slack: every 5 min; Calendar: every 15; iMessage: every 2 with a SQLite changefeed if cheap).

### Week 3 — Retrieval + brief
- `retrieval.rs` query pipeline: hybrid (vector top-k + BM25 over text) + recency boost + source filter + entity filter. Returns `Hit { item, score, snippet }`.
- `brief.rs` Today composer — three sections, each its own LLM call with retrieved context, run on a schedule and cached; `compose_now()` exposed for manual refresh.
- Conversations.tsx augmentation: always-RAG injection, citation chip rendering, source side-panel.

### Week 4 — Polish, privacy, demo
- `LocalEmbedder` impl using fastembed-rs (bundled in sidecar; ~100MB ONNX model, lazy-loaded on first use).
- Settings → Privacy panel with the modes table + per-source toggles.
- Today UI polish, empty states, the "you said you'd…" commitment parser (small dedicated prompt + manual-correction UI).
- Demo recording: 60-second cold-install-to-magic-moment screen capture for the README and landing page.

## Code reuse from existing projects

| Existing | What we lift | Where it lands |
|---|---|---|
| `~/projects/inbox/inbox_client.py` | iMessage SQLite reader logic, contact resolution | `life_capture/ingestors/imessage.rs` (port to Rust) |
| `~/projects/inbox/mcp_backend.py` | Gmail thread normalization, dedupe heuristics | `life_capture/ingestors/gmail.rs` |
| `~/projects/ambient/` | Voice dictate/transcribe (future input modality for Today brief — v2) | Out of scope for v1, noted as adjacent |
| `~/projects/neocortex/` | The memory backend OpenHuman already wraps via tinyhumansai SDK | No change — `personal_index.db` is a new local-only store separate from this |

## Testing

- Per-mode integration test (Convenience / Hybrid / Fully local) with mocked external endpoints — assert no network egress on Fully local.
- Per-ingestor unit tests against fixture data (real anonymized samples committed under `src/openhuman/life_capture/ingestors/fixtures/`).
- Retrieval quality test: a small fixed corpus + 20 hand-written "what did X say about Y" queries with expected top-1 hits. CI fails if precision@1 drops below threshold.
- iMessage ingestor: Mac-only test gated on `target_os = "macos"`, runs against a fixture chat.db.
- Manual cold-install matrix on Mac/Windows/Ubuntu: install → grant permissions → first brief composes within 5 minutes of first ingest.

## Open questions

1. **"Jarvis" disambiguation.** User referenced an existing "jarvis" assistant stack. I see `~/projects/{inbox, ambient, neocortex, ccv4, officeqa-local}` but nothing literally named jarvis. Confirm which project(s) we should pull from beyond the inbox + ambient + neocortex set already noted.
2. **Embedding provider default.** OpenAI text-embedding-3-small ($0.02/M tokens, 1536-dim) vs. Voyage voyage-3 (better recall, slightly more expensive). Default: OpenAI for ubiquity. OK?
3. **Background ingestion when desktop app is closed.** OpenHuman has tray + autostart on macOS, so the core sidecar can keep running headless. Should ingestion continue when the window is closed (so the morning brief is ready instantly), or only while the app is in the foreground? Default proposal: continue while the tray icon is present; pause entirely if the user quits from tray.

## Out of scope (named so we don't drift)

- Browser history, file system watcher, screen recording.
- Encrypted local index at rest.
- Multi-device sync of `personal_index.db`.
- Direct per-provider OAuth (BYO Google Cloud OAuth app etc.).
- Self-hostable backend image / docker-compose.
- Skills registry, marketplace, authoring UX.
- Voice input for Today brief (deferred — would reuse `~/projects/ambient/`).
- Auto-send of drafted replies. Drafts only, user always confirms.
