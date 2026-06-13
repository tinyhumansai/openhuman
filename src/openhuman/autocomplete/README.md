# autocomplete

System-wide inline text autocomplete for macOS. The engine captures the user's currently-focused text field via the accessibility (AX) middleware, runs **local** (on-device) inference to generate a short single-line continuation, renders it in a floating overlay badge, and applies it on Tab (or rejects on Escape). Accepted completions are persisted as personalisation examples that feed back into later inference. macOS-only at runtime; on other platforms `start` returns an error and most pieces compile to no-ops. There is also an in-app path (context override) used by the OpenHuman composer that bypasses AX capture.

## Responsibilities

- Run a background tokio loop that, on each debounce tick, captures focused-field context, calls local inference, and updates engine state with a sanitized suggestion.
- Detect Tab (accept) and Escape (reject) key edges and apply/clear the suggestion via the accessibility middleware (with focused-target re-validation before insertion).
- Render/hide the overlay badge anchored to the focused element, with an osascript notification fallback.
- Special-case terminal apps (extract just the input line) and skip blocked/disabled apps and OpenHuman's own window (the in-app React path handles that).
- Filter low-quality suggestions (too short, no alphanumerics, echo of typed tail).
- Auto-stop after `MAX_CONSECUTIVE_ERRORS` (5) consecutive failures to avoid notification floods; self-heal stuck `generating` phase; short-circuit Apple Events automation denial (`-1743`) to avoid re-firing the macOS consent popup.
- Persist accepted completions to the local KV store and a local memory-doc namespace; surface, list, and clear that history.
- Expose status/start/stop/current/debug_focus/accept/set_style/history/clear_history over the JSON-RPC + CLI controller registry, plus a CLI serve/spawn runner.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/autocomplete/mod.rs` | Export-focused root. Re-exports `core::*`, history helpers, `ops` (also aliased as `rpc`), and the controller schema/registry pair. |
| `src/openhuman/autocomplete/ops.rs` | Controller/CLI surface: `autocomplete_*` async fns returning `RpcOutcome<T>` with structured `[autocomplete]` logs; defines history params/results; `autocomplete_start_cli` (spawn/serve/foreground modes). Holds the ops unit tests. |
| `src/openhuman/autocomplete/schemas.rs` | `all_controller_schemas`, `all_registered_controllers`, per-function `ControllerSchema`, and `handle_*` thunks delegating to `ops`. |
| `src/openhuman/autocomplete/history.rs` | Persistence + personalisation: `save_accepted_completion`, `save_completion_to_local_docs`, `query_relevant_examples`, `load_recent_examples`, `list_history`, `clear_history`, and the `AcceptedCompletion` type. |
| `src/openhuman/autocomplete/core/mod.rs` | Engine submodule root; re-exports the engine and public types. |
| `src/openhuman/autocomplete/core/engine.rs` | `AutocompleteEngine` + global singleton (`AUTOCOMPLETE_ENGINE`, `global_engine`, `start_if_enabled`). Owns `EngineState`, the background refresh loop, `refresh`, `accept`, Tab/Escape handlers, quality/tab-artifact heuristics. |
| `src/openhuman/autocomplete/core/types.rs` | Serde DTOs (`AutocompleteStatus`, `Autocomplete{Start,Stop,Current,Accept,SetStyle}Params/Result`, `AutocompleteSuggestion`, `AutocompleteDebugFocusResult`); re-exports `FocusedTextContext`; `MAX_SUGGESTION_CHARS = 64`. |
| `src/openhuman/autocomplete/core/focus.rs` | Thin re-export of accessibility focus/insert/key-probe fns. |
| `src/openhuman/autocomplete/core/overlay.rs` | `show_overflow_badge` / `overlay_helper_quit` — overlay rendering via accessibility middleware with osascript notification fallback (macOS-gated). |
| `src/openhuman/autocomplete/core/terminal.rs` | Re-exports accessibility terminal detection/extraction helpers. |
| `src/openhuman/autocomplete/core/text.rs` | `sanitize_suggestion`, `truncate_head`, `is_no_text_candidate_error` and re-exported `truncate_tail`. |
| `src/openhuman/autocomplete/core/engine_tests.rs` | Engine unit tests (`#[path]`-included). |

## Public surface

- Engine: `AutocompleteEngine`, `AUTOCOMPLETE_ENGINE`, `global_engine()`, `start_if_enabled(&Config)`.
- Types: `AutocompleteStatus`, `AutocompleteSuggestion`, and the `*Params`/`*Result` DTOs listed above.
- Ops (also re-exported as `rpc`): `autocomplete_status`, `autocomplete_start`, `autocomplete_stop`, `autocomplete_current`, `autocomplete_debug_focus`, `autocomplete_accept`, `autocomplete_set_style`, `autocomplete_history`, `autocomplete_clear_history`, `autocomplete_start_cli`.
- History: `AcceptedCompletion`, `list_history`, `clear_history`, `load_recent_examples`, `query_relevant_examples`, `save_accepted_completion`, `save_completion_to_local_docs`.
- Schema pair: `all_autocomplete_controller_schemas`, `all_autocomplete_registered_controllers`.

## RPC / controllers

Namespace `autocomplete` (RPC methods `openhuman.autocomplete_<function>` / CLI `autocomplete <function>`):

| Function | Inputs | Output type | Purpose |
| --- | --- | --- | --- |
| `status` | — | `AutocompleteStatus` | Engine status + latest suggestion metadata. |
| `start` | `debounce_ms?` | `AutocompleteStartResult` | Start engine (clamped 50–2000 ms). |
| `stop` | `reason?` | `AutocompleteStopResult` | Stop engine, abort loop, quit overlay. |
| `current` | `context?` | `AutocompleteCurrentResult` | Compute suggestion for captured or explicit context (in-app path). |
| `debug_focus` | — | `AutocompleteDebugFocusResult` | Inspect focused element/text diagnostics. |
| `accept` | `suggestion?`, `skip_apply?` | `AutocompleteAcceptResult` | Apply (or mark accepted) and persist a completion. |
| `set_style` | `enabled?`, `debounce_ms?`, `max_chars?`, `style_preset?`, `style_instructions?`, `style_examples?`, `disabled_apps?`, `accept_with_tab?`, `overlay_ttl_ms?` | `AutocompleteSetStyleResult` | Update `[autocomplete]` config; auto-starts engine when `enabled=true`. |
| `history` | `limit?` (default 20) | `{ entries: [AcceptedCompletion] }` | List recent accepted completions, newest first. |
| `clear_history` | — | `{ cleared: u64 }` | Delete all history (KV + local docs). |

Schemas/handlers wired into the registry via `src/core/all.rs` (no domain branches in `cli.rs`/`jsonrpc.rs`). CLI help/serve/spawn flags are adapted by `src/core/autocomplete_cli_adapter.rs`.

## Agent tools

None. This domain owns no agent tools (`tools.rs` absent).

## Events

None. No `bus.rs`; the engine drives itself via an internal tokio loop and does not publish/subscribe to `DomainEvent`s.

## Persistence

History (`history.rs`) writes through `MemoryClient::new_local()` (local KV / docs under the default OpenHuman dir), in two layers:

- **KV namespace `autocomplete`** — `AcceptedCompletion` JSON keyed by zero-padded timestamp (`accepted:{ts:018}`) so lexical order == reverse-chronological; trimmed to `MAX_HISTORY_ENTRIES` (50). Powers the settings list (`list_history`) and recency examples (`load_recent_examples`).
- **Doc namespace `autocomplete-memory`** — formatted `"[app] ...tail → suggestion"` documents (source_type `autocomplete`, priority `low`, category `daily`), trimmed to `MAX_DOC_ENTRIES` (200), queried semantically by `query_relevant_examples`.

Config (`[autocomplete]` in the TOML `Config`) is the durable settings store, mutated by `set_style`.

## Dependencies

- `crate::openhuman::accessibility` — focused-text capture, text insertion, Tab/Escape/modifier key probes, terminal detection, overlay rendering, Swift helper precompile, and Apple Events automation-denial flags. The core engine's platform muscle.
- `crate::openhuman::config` (`Config`, `AutocompleteConfig`) — load/save settings; the gate for `enabled`, debounce, style, disabled apps, overlay TTL.
- `crate::openhuman::inference::local` (`local_ai`) — on-device inference via `inline_complete_interactive` (interactive variant bypasses the scheduler LLM permit for low keystroke latency).
- `crate::openhuman::memory_store` (`MemoryClient`, `NamespaceDocumentInput`) — local KV + doc persistence for accepted-completion history.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` + `crate::rpc::RpcOutcome` — controller registry plumbing.

## Used by

- `src/core/all.rs` — registers the controller schemas/handlers.
- `src/core/autocomplete_cli_adapter.rs` — CLI parsing/help/serve-spawn adapter for the `autocomplete` namespace; `src/core/cli.rs` and `src/core/logging.rs` consume it (including the `--autocomplete-logs` run scope).
- `src/openhuman/app_state/ops.rs` — app-state/startup wiring.
- `src/openhuman/accessibility/*` and `src/openhuman/credentials/ops.rs` — adjacent references (automation/permission flow).

## Notes / gotchas

- **macOS-only at runtime.** `start` returns `Err("autocomplete is only supported on macOS")`; `platform_supported` in status is `cfg!(target_os = "macos")`. Overlay/notification/focus-validation code is `#[cfg(target_os = "macos")]`-gated.
- **Single process-global engine.** RPC and the embedded startup share `AUTOCOMPLETE_ENGINE`, so tests serialize on a mutex; history integration tests are `#[ignore]` because they hit the real on-disk KV store.
- **OpenHuman-self skip vs in-app override.** The background loop skips AX refresh when OpenHuman is frontmost (React composer handles it); passing `context` to `current` forces inference even for OpenHuman and skips the (latency-costly) history example lookups.
- **Tab artifact cleanup.** Before applying on Tab, the engine detects and backspaces a trailing tab/indentation the app may have inserted (`detect_tab_artifact_suffix`).
- **Confidence is a placeholder** (`0.75`) until `inline_complete` surfaces a real score.
- **Debounce clamped 50–2000 ms; `max_chars` clamped 64–2048; `overlay_ttl_ms` clamped 300–10000.** `MAX_SUGGESTION_CHARS` (64) caps the displayed/applied suggestion regardless of `max_chars` (which bounds *input* context).
- **`set_style` with `enabled=true` auto-starts** the engine and logs the result; disabling aborts the loop and quits the overlay.
- **Refresh has a 120 s timeout**; on timeout with metadata drift the stale suggestion is cleared. A stale `generating` phase older than 12 s self-heals.
