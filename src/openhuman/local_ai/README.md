# Local AI

On-device inference stack. Owns the bundled Ollama runtime, whisper.cpp speech-to-text, Piper text-to-speech, sentiment scoring, vision-embedding routing, the model preset / device-profile chooser, asset download + install management, the GIF-decision heuristic, and the per-session `LocalAiService` singleton. Does NOT own remote-provider HTTP transport (`providers/`) or the agent tool loop (`agent/`).

## Public surface

- `pub struct LocalAiService` — `service/mod.rs` — singleton holding Ollama / whisper / Piper handles.
- `pub fn global(config: &Config) -> Arc<LocalAiService>` — `core.rs` — singleton accessor.
- `pub fn model_artifact_path(config: &Config) -> PathBuf` — `core.rs` — resolve on-disk model path.
- `pub struct DeviceProfile` — `device.rs` — RAM / VRAM / CPU classification used for preset selection.
- `pub struct ModelPreset` / `pub enum ModelTier` / `pub enum VisionMode` — `presets.rs` — bundled preset matrix.
- `pub struct SentimentResult` — `sentiment.rs` — polarity + magnitude scoring.
- `pub struct GifDecision` / `pub struct TenorGifResult` / `pub struct TenorSearchResult` — `gif_decision.rs`.
- Status / progress / result types: `pub struct LocalAiStatus`, `LocalAiAssetStatus`, `LocalAiAssetsStatus`, `LocalAiDownloadProgressItem`, `LocalAiDownloadsProgress`, `LocalAiEmbeddingResult`, `LocalAiSpeechResult`, `LocalAiTtsResult` — `types.rs`.
- `pub mod ops` (re-exported as `rpc`) — `ops.rs` — typed Rust wrappers around each capability (`agent_chat`, `agent_chat_simple`, `summarize`, `prompt`, `vision_prompt`, `embed`, `transcribe`, `tts`, `should_react`, `analyze_sentiment`, `should_send_gif`, `tenor_search`).
- RPC `local_ai.{agent_chat, agent_chat_simple, local_ai_status, local_ai_download, local_ai_download_all_assets, local_ai_summarize, local_ai_prompt, local_ai_vision_prompt, local_ai_embed, local_ai_transcribe, local_ai_transcribe_bytes, local_ai_tts, local_ai_assets_status, local_ai_downloads_progress, local_ai_download_asset, local_ai_device_profile, local_ai_presets, local_ai_apply_preset, local_ai_diagnostics, local_ai_set_ollama_path, local_ai_chat, local_ai_should_react, local_ai_analyze_sentiment, local_ai_should_send_gif, local_ai_tenor_search}` — `schemas.rs`.

## Calls into

- `src/openhuman/config/` — model paths, Ollama URL override, device-profile inputs.
- `src/openhuman/encryption/` — Tenor / asset keys at rest.
- Bundled binaries: Ollama (HTTP `OLLAMA_BASE_URL`), whisper.cpp, Piper.
- HTTP for Tenor GIF search.
- Filesystem under `~/.openhuman/local-ai/` for downloaded model artifacts.

## Called by

- `src/openhuman/agent/` — `local_ai::rpc::agent_chat` / `agent_chat_simple` are the primary chat backends; triage uses `agent::triage::routing` to decide local vs remote.
- `src/openhuman/voice/{streaming,postprocess,ops,types}.rs` — speech-to-text + text-to-speech.
- `src/openhuman/screen_intelligence/processing_worker.rs` — vision embedding + summarisation.
- `src/openhuman/autocomplete/core/engine.rs` — local-AI completions.
- `src/openhuman/tree_summarizer/ops.rs` — summarisation backend.
- `src/openhuman/app_state/ops.rs` — `LocalAiStatus` snapshot.
- `src/core/all.rs` — registers `all_local_ai_*`.

## Tests

- Unit: `ops_tests.rs`, `schemas_tests.rs`, plus `service/ollama_admin_tests.rs`, `service/public_infer_tests.rs`.
- Domain mutex: `LOCAL_AI_TEST_MUTEX` (`mod.rs:4`) serializes tests that mutate the singleton or env vars.
- Routing: `agent/triage/routing_tests.rs` covers local-vs-remote escalation.
