# Boost VC AI Capability Plan

## Commercial Inspirations

| Capability | Inspiration | What we replicate |
|-----------|-------------|-------------------|
| Voice Foundation | Siri, Google Assistant | Local STT (Whisper) + TTS (Piper) desktop assistant |
| Live Captions | Otter.ai, Microsoft Teams captions | Real-time transcription with saved transcripts |
| Voice Actions | Alexa Skills, Siri Shortcuts | Utterance → controller-backed action routing |
| Operator Inbox | Front, Superhuman | Triage, draft replies, follow-up scheduling |
| Chat-with-Data | Julius AI, ChatGPT Code Interpreter | NL queries over local/connected datasets |
| Guided Recommendations | Typeform, Intercom Product Tours | Quiz-style intake flows with branching logic |

## Features Replicated (v1)

### Voice Foundation (#1831)
- Session lifecycle (start/stop/status)
- PCM buffering with VAD (voice activity detection)
- STT via whisper-rs (local, open-source)
- TTS via Piper (local, open-source)
- LLM turn orchestration (STT → LLM → TTS)
- Conversation history context

### Live Captions (#1832)
- Transcript lifecycle (start/pause/resume/complete)
- Real-time segment appending with timestamps
- Extractive summarization on completed transcripts
- Source-agnostic (microphone or desktop audio)

### Voice Actions (#1833)
- Action registration with trigger phrases
- Fuzzy intent recognition (word overlap scoring)
- Safety levels (safe/confirmation_required/destructive)
- Confirmation flow for non-safe actions
- Execution tracking with status

### Operator Inbox (#1834)
- Priority scoring (urgent/high/medium/low)
- Multi-tone draft generation (professional/casual/formal)
- Follow-up scheduling
- Archive workflow

### Chat-with-Data (#1835)
- Dataset registration (CSV, database, API sources)
- Natural language query routing
- Proactive insight generation (anomaly detection)
- Dataset listing and metadata

### Guided Recommendations (#1836)
- Flow definition with branching steps
- Answer validation (type checking, choice validation)
- State machine (active → completed)
- Recommendation generation based on answers
- Builtin onboarding setup flow

## Explicit Non-Goals (v1)

- **No real-time streaming STT** — batch transcription per VAD segment only
- **No speaker diarization** — single-speaker assumption for v1
- **No actual email/Slack integration** — operator inbox is schema-only, no transport
- **No real SQL execution** — chat-with-data generates mock query results
- **No ML-based intent recognition** — word overlap heuristic, not a trained model
- **No persistent storage** — all state is in-memory (process-lifetime)
- **No frontend components** — backend domain modules only, frontend wiring is follow-up
- **No multi-language TTS** — English-only for Piper in v1

## Architecture

All capabilities follow the same pattern:
- Rust domain module under `src/openhuman/<domain>/`
- `types.rs` — domain types with serde
- `engine.rs` — business logic + state machine
- `rpc.rs` — JSON-RPC handlers
- `schemas.rs` — controller registry schemas (Def pattern)
- Wired into `core/all.rs` (controller registry + namespace description)
- Catalog entry in `about_app/catalog.rs`
- Structured tracing (`debug!`/`info!`/`warn!`) at all state transitions
