# VC Capabilities — PR #2261

Six production-grade AI modules covering voice, captions, actions, email triage,
data analytics, and guided onboarding. All implemented as Rust-core domains with
controller-registry RPC exposure, LLM fallback paths, and safety validation.

---

## Modules

| Module | Issue | Purpose |
|--------|-------|---------|
| `voice_assistant` | #1831 | Standalone voice session (mic→STT→LLM→TTS→speaker) |
| `live_captions` | #1832 | Real-time captioning + transcript store + diarization |
| `voice_actions` | #1833 | Voice-triggered commands with safety levels |
| `operator_inbox` | #1834 | Email triage + IMAP/SMTP + draft generation |
| `chat_with_data` | #1835 | NL→SQL + anomaly detection + proactive insights |
| `guided_flows` | #1836 | Branching quiz/recommendation state machine |

---

## 1. Voice Assistant (`voice_assistant`) — Issue #1831

### RPC Endpoints (namespace: `voice_assistant`, 6 total)

| Method | Description |
|--------|-------------|
| `openhuman.voice_assistant_start_session` | Open session with STT/TTS provider selection |
| `openhuman.voice_assistant_push_audio` | Feed PCM16LE audio (auto barge-in + VAD) |
| `openhuman.voice_assistant_poll_response` | Pull synthesized TTS PCM + text |
| `openhuman.voice_assistant_get_status` | Query session state, turn count, providers |
| `openhuman.voice_assistant_interrupt` | Manual barge-in (clear outbound buffer) |
| `openhuman.voice_assistant_stop_session` | Close session + return summary counters |

### Key Features

- **Barge-in / Interruption** (`session.rs`): Auto-detects speech during TTS playback (energy > -40dBFS threshold). Clears outbound buffer, transitions to Listening.
- **Streaming STT** (`brain.rs`): LocalAgreement-2 chunked approach for audio > 4s. Processes in 2s overlapping windows, emits confirmed partial transcripts. ~3s latency vs 30s for batch.
- **Multi-Language Detection** (`brain.rs`): Trigram-based detection via `whatlang` crate — 69 languages with confidence scoring. Auto-switches session language when confidence > 0.5.
- **Emotion Detection** (`brain.rs`): Keyword heuristics: urgent/negative/confused/positive. Stored on session as `detected_emotion`.
- **WebSocket Streaming** (`ws_transport.rs`): Binary bidirectional WebSocket at `/ws/voice/{session_id}`. Client sends PCM16LE frames, server sends TTS PCM + JSON status. Eliminates polling overhead (~10ms vs ~100ms).
- **Wake Word Detection** (`wake_word.rs`): Energy-based keyword spotting for hands-free activation.

### Limits

- 32 max concurrent sessions (LRU eviction)
- 10 min idle timeout
- 30s max PCM buffers
- 50 turn history (LLM uses last 10)

### Security

- Session ID validation (charset + length)
- Bearer auth: `OPENHUMAN_CORE_TOKEN`
- Audio data not persisted by default

### Providers

- STT: `whisper` (local, default) or `cloud`
- TTS: `piper` (local, default) or `cloud`
- Language hint: BCP-47 (e.g. `en`)

---

## 2. Live Captions (`live_captions`) — Issue #1832

### RPC Endpoints (namespace: `live_captions`, 11 total)

| Method | Description |
|--------|-------------|
| `openhuman.live_captions_start_transcript` | Start a new live caption transcript session |
| `openhuman.live_captions_append_segment` | Append a caption segment to an active transcript |
| `openhuman.live_captions_complete_transcript` | Mark a transcript as completed |
| `openhuman.live_captions_summarize_transcript` | Generate a summary for a completed transcript |
| `openhuman.live_captions_get_transcript` | Get transcript details (state, segment count, duration) |
| `openhuman.live_captions_list_transcripts` | List all transcripts |
| `openhuman.live_captions_search_transcripts` | Search transcripts by text content |
| `openhuman.live_captions_transcribe_audio` | Transcribe PCM audio and append as a caption segment |
| `openhuman.live_captions_pause_transcript` | Pause an active transcript |
| `openhuman.live_captions_resume_transcript` | Resume a paused transcript |
| `openhuman.live_captions_export_transcript` | Export transcript as SRT, VTT, or markdown |

### Key Features

- **Transcript Store** (`store.rs`): In-memory transcript storage with segment append, state management (active/paused/completed), and metadata tracking.
- **Speaker Diarization** (`diarize.rs`): Energy-based speaker change detection. 500ms windows, 250ms hop. Features: RMS + ZCR + spectral centroid. Threshold: 0.35.
- **Persistence** (`persist.rs`): Transcript serialization and file-based persistence for completed transcripts.
- **Summarization**: LLM-backed summary generation for completed transcripts.
- **Search**: Full-text search across transcript segments.
- **Audio Transcription**: Direct PCM→text pipeline that auto-appends segments.
- **Pause/Resume**: Transcript sessions can be paused and resumed without data loss.

### Limits

- Segments include: text, start_ms, end_ms, optional speaker label, optional confidence, optional is_final flag
- Sources: `microphone`, `system_audio`, `meet_call`

### Security

- Bearer auth required for all RPC calls
- No audio data persisted unless explicitly completed and saved

---

## 3. Voice Actions (`voice_actions`) — Issue #1833

### RPC Endpoints (namespace: `voice_actions`, 5 total)

| Method | Description |
|--------|-------------|
| `openhuman.voice_actions_recognize` | Recognize intent from utterance, map to controller action |
| `openhuman.voice_actions_confirm` | Confirm a pending voice action intent for execution |
| `openhuman.voice_actions_reject` | Reject a pending voice action intent |
| `openhuman.voice_actions_get_intent` | Get voice intent details by ID |
| `openhuman.voice_actions_list_mappings` | List all registered voice action mappings |

### Key Features

- **Intent Recognition** (`engine.rs`): Dual-path recognition:
  - Pattern matching: keyword substring matching against built-in action mappings
  - LLM fallback (`llm_intent.rs`): For utterances that don't match patterns, delegates to LLM for intent classification
- **Safety Tiers**: Three levels — `safe` (auto-dispatch), `requires_confirmation` (user must confirm), `destructive` (requires confirmation, logged)
- **Confirmation Flow**: Intents with `requires_confirmation` or `destructive` safety enter `pending` status. Must be explicitly confirmed via `confirm` RPC before execution.
- **Auto-Dispatch**: Safe intents are automatically dispatched to the target controller action.
- **Built-in Action Mappings** (10 default):
  - `open settings` → `config.get` (safe)
  - `search` → `memory.search` (safe)
  - `start voice` → `voice_assistant.start_session` (safe)
  - `stop voice` → `voice_assistant.stop_session` (safe)
  - `create draft` → `channels.create_draft` (safe)
  - `send message` → `channels.send` (requires_confirmation)
  - `delete` → `memory.delete` (destructive)
  - `check health` → `health.check` (safe)
  - `list skills` → `skills.list` (safe)
  - (additional mappings in engine.rs)

### Limits

- 200 max stored intents before eviction
- Pattern matching is case-insensitive substring

### Security

- Destructive actions always require explicit confirmation
- Intent execution is routed through the controller registry (no bypass)
- All intent state transitions are logged

---

## 4. Operator Inbox (`operator_inbox`) — Issue #1834

### RPC Endpoints (namespace: `operator_inbox`, 10 total)

| Method | Description |
|--------|-------------|
| `openhuman.operator_inbox_triage_message` | Triage an incoming message and score priority |
| `openhuman.operator_inbox_generate_draft` | Generate a reply draft with tone selection |
| `openhuman.operator_inbox_schedule_followup` | Schedule a follow-up at a given timestamp |
| `openhuman.operator_inbox_get_triage` | Get triage record by ID |
| `openhuman.operator_inbox_list_triage` | List all triage records |
| `openhuman.operator_inbox_archive` | Archive a triage record |
| `openhuman.operator_inbox_fetch_inbox` | Fetch new emails from IMAP and auto-triage |
| `openhuman.operator_inbox_send_reply` | Send a drafted reply via SMTP |
| `openhuman.operator_inbox_start_poller` | Start background IMAP polling loop |
| `openhuman.operator_inbox_stop_poller` | Stop background IMAP polling loop |

### Key Features

- **Triage Engine** (`engine.rs`): Dual-path priority scoring:
  - Keyword-based: scans subject/body for urgency indicators
  - LLM-backed: external priority classification for ambiguous messages
- **Priority Levels**: `urgent`, `high`, `normal`, `low` — with reason string explaining the classification
- **Draft Generation**: Tone-aware reply drafting (`professional`, `casual`, `formal`) based on triage context
- **Follow-up Scheduling**: Unix-timestamp-based follow-up scheduling per triage record
- **IMAP Client** (`imap_client.rs`): Async IMAP fetch for UNSEEN messages using `async-imap` + `tokio-rustls`
- **Connection Management** (`connection.rs`): TLS-secured IMAP/SMTP connection handling, matches existing `email_channel.rs` pattern. Fetches UNSEEN, parses with `mail-parser`, sends via SMTP with `lettre`.
- **Message Parser** (`parser.rs`): Email body extraction and metadata parsing
- **Bulk Operations**: List and archive for batch triage management

### Limits

- Body preview truncated to 200 characters in triage records
- Sources: `email`, `chat`, `social`, `webhook`
- Statuses: `pending` → `drafted` → `sent` → `archived`

### Security

- IMAP passwords encrypted at rest
- Bearer auth required for all RPC calls
- No raw email bodies stored beyond preview

---

## 5. Chat with Data (`chat_with_data`) — Issue #1835

### RPC Endpoints (namespace: `chat_with_data`, 9 total)

| Method | Description |
|--------|-------------|
| `openhuman.chat_with_data_register_dataset` | Register a dataset for querying |
| `openhuman.chat_with_data_query` | Ask a natural-language question over a dataset |
| `openhuman.chat_with_data_generate_insight` | Generate a proactive insight for a dataset |
| `openhuman.chat_with_data_list_datasets` | List registered datasets |
| `openhuman.chat_with_data_list_insights` | List generated insights |
| `openhuman.chat_with_data_get_dataset` | Get dataset details (columns, metadata) |
| `openhuman.chat_with_data_ingest_rows` | Ingest rows into a dataset for in-memory querying |
| `openhuman.chat_with_data_scan_anomalies` | Proactively scan all datasets for anomalies |
| `openhuman.chat_with_data_delete_dataset` | Remove a registered dataset |

### Key Features

- **Dataset Registration**: Register datasets with name, source type, column schema, and row count
- **NL→SQL Generation** (`sql_gen.rs`): Dual-path query generation:
  - Pattern matching: common question patterns mapped to SQL templates
  - LLM fallback: for complex questions, delegates to LLM for SQL generation
- **SQL Safety Validation** (`sql_gen.rs`): Uses `sqlparser` AST analysis to reject unsafe queries (DROP, DELETE, ALTER, INSERT, UPDATE, TRUNCATE, CREATE)
- **In-Memory Execution** (`engine.rs`): Ingested rows can be queried in-memory without external database
- **Anomaly Detection** (`anomaly.rs`): Statistical anomaly detection across dataset columns (z-score based), generates insight records
- **Proactive Insights**: Automated insight generation with title, description, and confidence scoring
- **Built-in Sample Dataset**: `sample_metrics` with columns: date, metric, value, category (1000 rows)

### Limits

- Sources: `csv`, `json`, `sqlite`, `api`
- Only SELECT queries allowed (enforced by sqlparser AST)
- In-memory execution requires prior `ingest_rows` call
- Confidence scores range 0.0–1.0

### Security

- SQL injection prevention via `sqlparser` AST validation + double-quoted identifiers
- Only read-only queries (SELECT) pass safety check
- Bearer auth required for all RPC calls
- No external database connections in default mode (in-memory only)

---

## 6. Guided Flows (`guided_flows`) — Issue #1836

### RPC Endpoints (namespace: `guided_flows`, 5 total)

| Method | Description |
|--------|-------------|
| `openhuman.guided_flows_list_flows` | List all available guided recommendation flows |
| `openhuman.guided_flows_start_flow` | Start a new guided flow session, returns first step |
| `openhuman.guided_flows_submit_answer` | Submit an answer for the current step, advance flow |
| `openhuman.guided_flows_get_session` | Get current state of a guided flow session |
| `openhuman.guided_flows_register_flow` | Register a custom flow definition |

### Key Features

- **Flow Definitions**: Declarative flow structure with steps, branching, and answer types
- **Branching State Machine** (`engine.rs`): Steps can branch based on answer values (HashMap<answer_value, next_step_id>) or follow linear `next` pointer
- **Session Management**: LRU eviction at 64 concurrent sessions. Evicts completed sessions first, then oldest by `created_at`.
- **Answer Validation** (`engine.rs`): Per-step validation based on answer type:
  - `single_choice`: must be one of defined choices
  - `multi_choice`: array of valid choices
  - `boolean`: must be JSON boolean
  - `number`: must be JSON number
  - `free_text`: optional regex validation pattern
- **Recommendation Generation** (`engine.rs` + `scoring.rs`): Tag-based scoring system:
  - Choice→tag mappings accumulate a user profile vector
  - Catalog items are ranked by cosine-like similarity to profile
  - Top match becomes the recommendation with confidence score and next actions
- **Built-in Flows** (2):
  - `onboarding_setup` — "OpenHuman Setup Guide" (4 steps, 1 branch)
  - `tool_recommendation` — "Tool Recommendation Quiz" (3 steps, linear)

### Limits

- 64 max concurrent sessions (LRU eviction)
- Sessions have states: `active`, `completed`, `abandoned`
- Completed sessions reject further answers
- Step ID must match current step (no skipping)

### Security

- Session ID validation
- Bearer auth required for all RPC calls
- No external data access — all flow logic is in-memory

---

## Integration

All 6 modules are registered in `src/core/all.rs` (lines 252–265) and are
callable over the standard JSON-RPC surface at `http://127.0.0.1:<port>/rpc`
with bearer auth.

RPC method naming convention: `openhuman.<namespace>_<function>`

## Testing

245+ unit tests across all modules. Each module has:
- Schema registration tests (handlers match schemas, correct namespace)
- Engine logic tests (happy path + error cases)
- Type serialization round-trip tests

E2E: `cargo test --test json_rpc_e2e`

---

## Infrastructure Modules

### Noise Cancellation (`voice_assistant/noise_cancel.rs`)

Neural noise suppression via `nnnoiseless` (pure-Rust RNNoise port) + NLMS adaptive echo cancellation.
Configurable strength (0.0–1.0), filter length, and step size.
Maintains per-session state for continuous noise floor estimation.

### Voice Profiles (`live_captions/voice_profiles.rs`)

Speaker identification via MFCC-like audio embeddings (13-dim).
Register profiles from >= 1s audio, identify speakers via cosine similarity.
Running average updates for profile refinement. Max 50 profiles.

### IMAP Background Poller (`operator_inbox/poller.rs`)

Tokio background task that periodically fetches UNSEEN emails from IMAP
and auto-triages them. Configurable interval (default: 2 min).
Start/stop control via `start_polling()` / `stop_polling()`.

### Webhook Notifications (`chat_with_data/webhooks.rs`)

Register HTTP webhook endpoints for anomaly/insight events.
Fires async POST with JSON payload (event type, insight details, timestamp).
Max 20 registered webhooks. Non-blocking — spawns tokio tasks.

### Database Connector (`chat_with_data/db_connector.rs`)

SQLite read-only query execution via rusqlite. Schema introspection
(table listing, column types). Row limit (1000). Rejects non-SELECT queries.

### Multi-Turn Context (`voice_actions/engine.rs`)

Per-session intent history with 5-intent sliding window and 5-minute
inactivity timeout. Enables contextual follow-up commands.

### Streaming TTS (`voice_assistant/brain.rs`)

Sentence-level chunking of LLM replies (via `unicode-segmentation` UAX#29 boundaries) with progressive TTS synthesis
and enqueue. Playback starts before full reply is synthesized.
Barge-in detection between chunks stops synthesis early.

### Per-Stage Latency Tracking (`voice_assistant/brain.rs`)

Every voice turn logs per-stage latency: STT ms, LLM ms, TTS ms, and total.
Enables performance profiling and SLA monitoring without external APM.
Format: `[voice-assistant-brain] turn completed session=X latency: stt=Nms llm=Nms tts=Nms total=Nms`

### WebSocket Route Builder (`voice_assistant/ws_transport.rs`)

`ws_router()` returns a mountable Axum Router with the `/ws/voice/{session_id}`
upgrade endpoint. Merge into any Axum app for real-time bidirectional audio
streaming (PCM16LE binary frames + JSON status messages).

---

## Known Limitations & Future Work

### No Rust Solution Currently

| Capability | Status | Notes |
|-----------|--------|-------|
| **Voice Cloning** | No Rust crate | Closest: sherpa-onnx VITS/Kokoro TTS with voice selection. No pure-Rust voice cloning exists. |
| **Neural Speaker Diarization** | Skipped | `speakrs 0.4` requires MKL/OpenBLAS (~200MB), uses `ort 2.x` which conflicts with other crates using `ort 1.x`. Energy-based diarization used instead. |
| **Neural Translation** | LLM pipeline | `rust-bert` uses `ort 1.x`, incompatible with `ort 2.x` in same binary. Translation uses LLM inference (GPT-4/Claude/local) via `translate.rs` — better quality than offline models. |

### Architecture Decisions

- **In-memory stores**: All modules use `LazyLock<Mutex<HashMap>>`. Voice profiles persist to JSON. Full persistence (SQLite/sled) is future work.
- **Energy-based diarization**: 13-dim band energy features. Adequate for demo, not production speaker ID. Future: sherpa-onnx speaker embedding models.
- **Emotion detection**: Keyword heuristics only. Future: acoustic/prosodic analysis via ML model.
- **WebSocket transport**: Infrastructure-ready (`ws_router()`) but not mounted in Tauri desktop app (uses IPC). For future HTTP server mode.

### Security Hardening Applied

- SQL identifiers double-quoted in all `sql_gen.rs` paths (prevents injection)
- Atomic file writes for voice profiles (write-to-tmp + rename)
- Per-session processing locks prevent concurrent brain turns
- Input validation on all RPC endpoints (required field checks)
