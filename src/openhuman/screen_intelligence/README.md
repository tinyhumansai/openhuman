# screen_intelligence

macOS-focused screen capture, accessibility automation, and on-device vision summarization. The module boots an in-process `AccessibilityEngine` singleton that runs consent-gated capture sessions: it polls the foreground window, screenshots the active window via `screencapture -l <windowID>`, runs a 3-pass OCR + local-LLM pipeline over each frame, and persists synthesized "what the user is doing right now" markdown documents into unified memory. It also exposes permission detection/requests, manual capture/diagnostics, input-action automation, autocomplete suggestions, and a macOS Globe/Fn hotkey listener. Capture is macOS-only in V1; on other platforms session start returns a `macOS-only` error and the embedded server autostart is skipped.

## Responsibilities

- Own the process-global `AccessibilityEngine` (state in `state.rs`) and its session lifecycle (enable / `start_session` with consent + TTL clamp 30..3600s / disable / TTL expiry).
- Detect and request macOS privacy permissions (Screen Recording, Accessibility, Input Monitoring, cross-platform Microphone) and open the relevant System Settings panes.
- Run a capture worker (`capture_worker.rs`) that polls foreground context at `baseline_fps`, captures the active window (window-ID only — never fullscreen fallback), applies allow/deny policy, optionally saves PNGs to `{workspace}/screenshots/`, and feeds frames to the vision worker.
- Run a processing worker (`processing_worker.rs`) that drains to the latest frame, compresses the image, runs Apple Vision OCR → vision LLM → synthesis LLM (Ollama), and persists a `VisionSummary` to memory.
- Provide manual / diagnostic capture (`capture_now`, `capture_image_ref_test`, `capture_test`), vision queries (`vision_recent`, `vision_flush`), and input automation (`input_action`) including a `panic_stop` action.
- Maintain in-memory autocomplete context and produce heuristic suggestions.
- Host a standalone `SiServer` (`server.rs`) that drives a capture+vision session in a monitoring loop, for embedded core autostart or CLI use.
- Expose the `openhuman screen-intelligence` CLI (`cli/`) and the `screen_intelligence.*` JSON-RPC controller surface.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused. Re-exports `ops as rpc`, all `ops::*`, schema pair, `global_engine`/`AccessibilityEngine`, and `types::*`. |
| `ops.rs` | RPC/CLI handler functions returning `RpcOutcome<T>` (status, permissions, sessions, capture, vision, globe listener) + `accessibility_doctor_cli_json`. |
| `schemas.rs` | `ControllerSchema`s, `all_controller_schemas`, `all_registered_controllers`, and `handle_*` fns delegating to `ops`. |
| `types.rs` | Serde domain types (`AccessibilityStatus`, `SessionStatus`, `CaptureFrame`, `VisionSummary`, `InputActionParams`, params/results). Re-exports permission types from `accessibility`. |
| `state.rs` | `EngineState`, `SessionRuntime`, `AccessibilityEngine`, and the `ACCESSIBILITY_ENGINE` lazy singleton / `global_engine()`. |
| `engine.rs` | Core `impl AccessibilityEngine`: config apply, session lifecycle, status, capture actions, screenshot-to-disk, allow/deny policy. |
| `input.rs` | `impl AccessibilityEngine` for `input_action`, `autocomplete_suggest`, `autocomplete_commit`. |
| `vision.rs` | `impl AccessibilityEngine` for `vision_recent`, `vision_flush`, `analyze_and_persist_frame`. |
| `capture_worker.rs` | Background screenshot loop (foreground poll, window capture, disk save, frame enqueue). |
| `processing_worker.rs` | Background vision pipeline: drain-to-latest, OCR (Apple Vision via `swift`), vision LLM, synthesis LLM, persist. |
| `image_processing.rs` | PNG→resized JPEG compression for the vision LLM (`compress_screenshot`, defaults 1024px / quality 72). |
| `helpers.rs` | Input validation, ephemeral ring-buffer pushes, vision-output parsing, memory persistence (`persist_vision_summary` → `background` namespace), suggestion generation, `truncate_tail`. |
| `limits.rs` | Buffer/string caps (max 120 frames/summaries, 256-char context, etc.). |
| `capture.rs` | `now_ms()` timestamp helper. |
| `permissions.rs` | Empty stub — permission detection moved to the `accessibility` middleware; retained for module-tree compatibility. |
| `server.rs` | `SiServer` runtime, `ServerState`/`SiServerStatus`/`SiServerConfig`, global singleton, `start_if_enabled`, `run_standalone`, benign-failure classifiers. |
| `cli/mod.rs` | `openhuman screen-intelligence` dispatch + shared opts/bootstrap helpers. |
| `cli/{capture,doctor,server,session}.rs` | CLI subcommand impls (`capture`/`vision`, `doctor`, `run`, `status`/`start`/`stop`). |
| `tests.rs`, `engine_tests.rs`, `schemas_tests.rs` | Test suites (sibling-file `#[path]` for engine/schemas). |

## Public surface

- `global_engine() -> Arc<AccessibilityEngine>` and `AccessibilityEngine` (the session engine; impl blocks split across `engine.rs`/`input.rs`/`vision.rs`).
- `ops::*` (re-exported as `rpc`): async handlers e.g. `accessibility_status`, `accessibility_request_permission(s)`, `accessibility_refresh_permissions`, `accessibility_start_session`, `accessibility_stop_session`, `accessibility_capture_now`, `accessibility_capture_image_ref`, `accessibility_capture_test`, `accessibility_input_action`, `accessibility_vision_recent`, `accessibility_vision_flush`, `accessibility_globe_listener_{start,poll,stop}`, `accessibility_doctor_cli_json`.
- `all_screen_intelligence_controller_schemas` / `all_screen_intelligence_registered_controllers` (registry wiring).
- `types::*` — `AccessibilityStatus`, `SessionStatus`, `AccessibilityFeatures`, `CaptureFrame`, `CaptureNowResult`, `CaptureImageRefResult`, `CaptureTestResult`, `VisionSummary`, `VisionRecentResult`, `VisionFlushResult`, `InputActionParams`/`Result`, `StartSessionParams`, `StopSessionParams`, `PermissionRequestParams`, `AppContextInfo`, autocomplete types, plus re-exported `PermissionKind`/`PermissionState`/`PermissionStatus`/`GlobeHotkey*`.
- `server::{SiServer, SiServerConfig, SiServerStatus, ServerState, global_server, try_global_server, start_if_enabled, run_standalone}`.
- `cli::run_screen_intelligence_command`.

## RPC / controllers

Namespace `screen_intelligence` (registered via `all_screen_intelligence_registered_controllers` in `src/core/all.rs`). Functions: `status`, `request_permissions`, `request_permission`, `refresh_permissions`, `start_session`, `stop_session`, `capture_now`, `capture_image_ref`, `input_action`, `vision_recent`, `vision_flush`, `capture_test`, `globe_listener_start`, `globe_listener_poll`, `globe_listener_stop`. All `handle_*` fns delegate to `ops` and return CLI-compatible JSON via `RpcOutcome::into_cli_compatible_json`.

## Agent tools

None. This module owns no `tools.rs` / agent-tool impls.

## Events

None. No `bus.rs` / `EventHandler` impls; the module is driven by RPC/CLI/in-process workers, not the event bus.

## Persistence

- **Vision summaries → unified memory** (`helpers::persist_vision_summary`): writes via `MemoryClient::from_workspace_dir(...).put_doc_light(...)` (light path — no vectors/graph). Namespace `background`, source type `screenshot`, category/tag `screen_intelligence`, key `screen_intelligence_{captured_at_ms}_{fnv_hash(id)}`. Body is markdown with YAML frontmatter (app, window, captured ts, confidence, id).
- **Screenshots → disk** (`engine::save_screenshot_to_disk`): `{workspace_dir}/screenshots/{captured_at_ms}_{app_slug}.png` when `keep_screenshots` is set (otherwise a temp file is written for OCR and deleted).
- **In-memory only**: `SessionRuntime` holds ring buffers of recent `CaptureFrame`s and `VisionSummary`s (capped at 120 each) plus autocomplete context — not durably persisted.

## Dependencies

- `crate::openhuman::accessibility` — permission detection/requests, `foreground_context`, `AppContext`, window screenshot (`capture_screen_image_ref_for_context`), macOS privacy-pane opener, and the Globe/Fn hotkey listener (`globe_listener_*`). Permission types are re-exported from here.
- `crate::openhuman::config` — loads `Config` / `ScreenIntelligenceConfig` (enabled, vision_enabled, use_vision_model, keep_screenshots, baseline_fps, session_ttl_secs, panic_stop_hotkey, policy_mode, allow/deny lists, autocomplete_enabled) and `workspace_dir`; `local_ai` provider/model settings gate vision.
- `crate::openhuman::inference::local` (as `local_ai`) — Ollama vision + synthesis LLM calls in the processing worker.
- `crate::openhuman::memory_store` — `MemoryClient` / `NamespaceDocumentInput` for persisting vision summaries.
- `crate::core::all` — `ControllerFuture`/`RegisteredController` for the RPC registry; `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` for schemas; `crate::core::logging` for CLI logging; `crate::rpc::RpcOutcome`.
- `crate::openhuman::embeddings::NoopEmbedding` — test-only (`tests.rs`).

## Used by

- `src/core/all.rs` — registers controllers/schemas and the namespace description.
- `src/core/cli.rs` — dispatches the `screen-intelligence` CLI subcommand.
- `src/openhuman/config/ops.rs` — applies config changes to the engine.
- `src/openhuman/app_state/ops.rs`, `src/openhuman/credentials/ops.rs` — referenced for lifecycle/state.
- `src/openhuman/tools/local_cli.rs` — referenced from the local CLI tool surface.

## Notes / gotchas

- **macOS-only V1.** `enable`/`start_session` return an error off macOS; `start_if_enabled` no-ops on non-macOS and must not initialize the global server (enforced by test).
- **Permission cache per-process.** macOS TCC grants are per-executable and per-process; the running core never sees a freshly-granted permission until it restarts. `refresh_permissions` re-detects (status always calls `detect_permissions()`), but a `restart_core_process` is required to pick up new grants — see GH #133. `AccessibilityStatus` carries `permission_check_process_path` and `core_process` (pid/started_at) so the UI can verify a restart actually happened.
- **Window-ID required.** Capture only proceeds when the foreground context has a `window_id`; there is no fullscreen fallback.
- **Vision pipeline requires local AI.** `analyze_frame` errors unless `local_ai.runtime_enabled=true` and provider is `ollama`. OCR shells out to `swift -e <Vision code>` with a 30s timeout and `kill_on_drop`. `--no-vision-model` / `--ocr-only` (CLI) sets `use_vision_model=false` at runtime (not persisted) to skip the vision LLM pass.
- **Drain-to-latest.** The processing worker discards stale queued frames and only analyzes the most recent, deduping by `captured_at_ms`; vision-summary pushes also dedupe on `captured_at_ms` so `vision_flush` and the worker channel don't double-store.
- **Lock discipline.** Workers read all needed state under the `Mutex`, then drop the lock before slow I/O (screencapture, disk, LLM); session task handles are aborted/awaited outside the lock to avoid deadlocks.
- **Benign start-session failures** (`macOS-only` off-macOS, `session already active`) are classified down to `info!` so they don't surface as Sentry errors; the `SiServer` cancellation token is swappable so it can restart after logout→login within one process.
- `permissions.rs` is an intentional empty stub; YAML frontmatter escaping in `persist_vision_summary` is best-effort (only quotes/newlines).
