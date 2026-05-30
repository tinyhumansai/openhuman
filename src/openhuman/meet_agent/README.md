# meet_agent

The **listening + speaking loop for a live Google Meet call**. Where `meet/` is single-shot pure validation (mint a `request_id`, hand it to the shell to open a window), `meet_agent/` is the long-lived per-call session that actually makes the bot listen and talk. While a call is open, the Tauri shell streams captured audio (PCM frames) and scraped caption lines into the core; the core runs VAD-segmented STT (or wake-word matching on captions), routes the utterance through the user's full orchestrator agent, synthesizes TTS, and streams PCM back out for the shell's virtual-mic pump. All state is keyed by the same `request_id` `meet/` mints.

## Responsibilities

- Maintain a process-wide registry of per-call sessions keyed by `request_id` (open / poll / close, idempotent restart).
- Accept inbound PCM16LE @ 16 kHz mono frames and run a crude RMS energy VAD with hangover to detect end-of-utterance.
- Accept live caption lines from Meet's captions DOM and run a wake-word state machine (`"hey openhuman"` and brand-mangle variants).
- Enforce a **privacy gate**: only the configured call owner (or owner-granted allowlist members) may wake the bot for tool-backed turns. Non-owners get a friendly greeting or a polite refusal; the bot never wakes on its own caption echo.
- Orchestrate a turn: STT (audio path) or caption text → full orchestrator Agent (with tools/memory/integrations) → TTS → enqueue outbound PCM. Falls back to a bare toolless chat-completions path / deterministic stubs when the backend token is missing.
- Handle barge-in (cancel/flush outbound on a new turn), pre-roll filler acks for slow integration paths, dedup + cooldown + min-turn-gap rate limiting against Meet's caption re-emits.
- Persist a recent-calls record (JSONL) on `stop_session` and expose a `list_calls` history read.
- Strip markdown / chain-of-thought / reasoning preamble from LLM output so TTS reads a clean spoken sentence; hard-cap reply length.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/meet_agent/mod.rs` | Module docstring + exports; re-exports controller schema pair and session registry types. Export-focused only. |
| `src/openhuman/meet_agent/types.rs` | Serde request/response types for all RPC endpoints + `SessionEvent` / `SessionEventKind` transcript record. Audio crosses the boundary as base64 PCM16LE. |
| `src/openhuman/meet_agent/ops.rs` | Pure, tokio-free helpers: sample-rate validation, `request_id` sanitization, `frame_rms`, and the stateful `Vad` (energy + hangover). |
| `src/openhuman/meet_agent/session.rs` | `MeetAgentSession` (ring buffers, VAD state, transcript log, counters, wake-word state machine, owner/allowlist privacy gate) + `MeetAgentSessionRegistry` + the `SESSION_REGISTRY` `OnceLock` singleton. |
| `src/openhuman/meet_agent/brain.rs` | Turn orchestration: STT → LLM → TTS. Owns the per-meet cached orchestrator `Agent`, the audio `run_turn` and caption `run_caption_turn`, soft-deny / grant turns, system prompts, and speech-cleanup helpers. |
| `src/openhuman/meet_agent/rpc.rs` | JSON-RPC handlers (deserialize-validate-dispatch only); spawns brain turns, persists the call record on stop. |
| `src/openhuman/meet_agent/schemas.rs` | Controller schema definitions + `handle_*` futures delegating to `rpc.rs`; the registry `all_controller_schemas` / `all_registered_controllers`. |
| `src/openhuman/meet_agent/store.rs` | `MeetCallRecord` type + append-only JSONL persistence (`append_record`, `read_recent`) under the workspace data dir. |
| `src/openhuman/meet_agent/wav.rs` | `pack_pcm16le_mono_wav` — minimal RIFF/WAVE header wrapper so raw PCM batches can be posted to the cloud Whisper endpoint as `audio/wav`. |
| `src/openhuman/meet_agent/ops_tests.rs` | Sibling test suite for `ops.rs` (`#[path]`-included). |

## Public surface

- `MeetAgentSession`, `MeetAgentSessionRegistry`, `SESSION_REGISTRY` (re-exported from `session`).
- `all_meet_agent_controller_schemas` / `all_meet_agent_registered_controllers` (re-exported from `schemas`).
- All `types::*` (request/response structs, `SessionEvent`, `SessionEventKind`).
- Module-internal-but-`pub`: `session::registry()`, `session::CaptionOutcome`; `brain::run_turn`, `run_caption_turn`, `run_soft_deny_turn`, `run_grant_turn`, `forget_session_agent`; `store::{MeetCallRecord, append_record, read_recent, MAX_RECENT_CALLS, meet_calls_jsonl_path}`; `ops::{Vad, VadEvent, validate_sample_rate, sanitize_request_id, frame_rms, REQUIRED_SAMPLE_RATE}`; `wav::pack_pcm16le_mono_wav`.

## RPC / controllers

Namespace `meet_agent` (dispatched as `openhuman.meet_agent_<function>`), wired into `src/core/all.rs`:

| Function | Purpose |
| --- | --- |
| `start_session` | Open a session for a `request_id`; installs owner/bot identities + meet URL. Inputs: `request_id` (required), `sample_rate_hz`, `owner_display_name`, `bot_display_name`, `meet_url`. |
| `push_listen_pcm` | Shell pushes captured PCM frames; spawns a brain turn on VAD end-of-utterance. Returns `turn_started`. |
| `push_caption` | Shell pushes a scraped caption line; runs the wake-word/privacy gate and spawns a normal, soft-deny, or no-op turn. Returns `turn_started`. |
| `poll_speech` | Pull synthesized outbound PCM (`pcm_base64`, `utterance_done`, `flush_pending` for barge-in). |
| `stop_session` | Close session, drop the cached Agent, persist a `MeetCallRecord`; returns `listened_seconds` / `spoken_seconds` / `turn_count`. |
| `list_calls` | Return recent completed calls (newest first) from the JSONL log. Optional `limit` (default 50, capped at `MAX_RECENT_CALLS` = 200). |

## Agent tools

None. This domain does not own any agent tools (no `tools.rs`). It is a *consumer* of the orchestrator agent — it builds and drives an `agent::harness::session::Agent` per meet, inheriting that agent's tool surface.

## Events

None via the event bus. There is no `bus.rs` and no `DomainEvent` publish/subscribe. The orchestrator turn sets a per-meet event context (`agent.set_event_context("meet_{request_id}", "meet_agent")`) for harness observability scoping, but `meet_agent` itself neither publishes nor subscribes to `DomainEvent`s.

## Persistence

- **Recent calls log** (`store.rs`): append-only JSONL at `{workspace_dir}/meet_agent/calls.jsonl`, one `MeetCallRecord` per closed call. `read_recent` reads, drops malformed lines, sorts newest-first by `started_at_ms`, and caps at `MAX_RECENT_CALLS`. Chosen over sqlite for low-cardinality, write-rarely data.
- **In-memory session state** (`session.rs`): the `SESSION_REGISTRY` `OnceLock<Mutex<HashMap>>` holds live `MeetAgentSession`s (ring buffers, VAD, transcript events, wake state, allowlist) for the duration of a call. Dropped on `stop_session`; not persisted.
- **Per-meet Agent cache** (`brain.rs`): `AGENT_CACHE` `OnceLock<TokioMutex<HashMap<request_id, Arc<TokioMutex<Agent>>>>>` reuses one orchestrator Agent across a call's turns (accumulating history); dropped by `forget_session_agent` on stop.

## Dependencies

- `crate::openhuman::agent::harness::session::Agent` — builds/drives the full orchestrator (tools, memory tree, MCP, skills) per meet turn (`brain.rs`).
- `crate::openhuman::voice::cloud_transcribe` — cloud STT (`transcribe_cloud`) for the audio listen path.
- `crate::openhuman::voice::reply_speech` — cloud TTS (`synthesize_reply`, `pcm_16000` output) for spoken replies.
- `crate::openhuman::config` — `Config` / `config::ops::load_config_with_timeout` for backend URL, token, and workspace dir resolution.
- `crate::api::{BackendOAuthClient, config::effective_backend_api_url, jwt::get_session_token}` — the bare chat-completions fallback path in `brain.rs`.
- `crate::core::all::{ControllerFuture, RegisteredController}` and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry contract (`schemas.rs`).
- `crate::rpc::RpcOutcome` — RPC response envelope (`rpc.rs`).
- External crates: `base64`, `serde`/`serde_json`, `tokio`, `chrono`, `reqwest`.

## Used by

- `src/core/all.rs` — registers the controllers/schemas and routes the `"meet_agent"` namespace.
- `src/openhuman/mod.rs` — declares the domain module.
- `src/openhuman/about_app/catalog.rs` — capability catalog entry.
- `src/openhuman/desktop_companion/pipeline.rs` — reuses `meet_agent::wav::pack_pcm16le_mono_wav` and is modelled on `meet_agent::brain`'s STT/LLM/TTS pipeline.

## Notes / gotchas

- **Single sample rate.** Only 16 kHz is accepted (`REQUIRED_SAMPLE_RATE`); `validate_sample_rate` hard-rejects anything else. The shell must resample CEF's native rate down before pushing. WAV packing and duration math assume this constant.
- **Privacy gate fails closed.** With no `owner_display_name` configured, *no* wake fires — a misconfigured launch can never expose the user's tool surface to a remote participant. Owner match is light-normalised (lowercase, trim, single trailing parenthetical strip like `(host)`/`(you)`). Bot-self captions are dropped first so the bot never re-wakes on its own TTS echo.
- **Two listen paths.** Audio (`push_listen_pcm` → VAD → `run_turn` → STT → LLM → TTS) and captions (`push_caption` → wake-word gate → `run_caption_turn`, which skips STT since captions are already text). The caption path is the primary in-product flow.
- **No toolless fallback on agentic failure.** When the orchestrator path fails/times out (`AGENTIC_TURN_TIMEOUT_SECS` = 90s), the bot speaks a canned "Let me get back to you" rather than the bare LLM — a toolless model confidently hallucinates "I don't have access" to calendar/Slack/Gmail, which is worse than a deferral.
- **Heavy rate-limiting against Meet's caption churn.** Meet re-emits the same caption row every ~250 ms with punctuation/case jitter; the session layers per-speaker dedup, a `turn_in_progress` gate, a 60s min-turn-gap, a caption-ts wake cooldown, and a 20s unauthorized soft-deny cooldown to prevent "sorry, sorry, sorry" / multi-fire loops.
- **Owner grant flow.** A non-owner wake records a pending grantee for `PENDING_GRANT_WINDOW_MS` (2 min); the owner saying "allow"/"go ahead"/"let them in" (`looks_like_grant_intent`) adds the speaker to the per-call allowlist via `run_grant_turn`. The allowlist resets when the session is dropped on stop.
- **Reasoning-model output is scrubbed.** `strip_for_speech` removes `<think>` blocks, markdown, and untagged reasoning preamble, then `cap_for_speech` truncates to `MAX_TTS_CHARS` (400) at a sentence boundary so TTS stays interruptible.
- **Persistence is best-effort.** A failed `append_record` on stop logs a warning but never blocks the `stop_session` response.
