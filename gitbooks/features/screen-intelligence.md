---
description: >-
  On-device screen capture, OCR + vision summarization, input automation, and
  inline autocomplete, gated behind explicit macOS privacy permissions.
icon: scan-eye
---

# Screen Intelligence

Screen Intelligence lets the agent see what you're working on. When you start a consent-gated session, OpenHuman periodically screenshots your **active window**, runs it through on-device OCR and a local vision model, and synthesizes a short "what the user is doing right now" note into memory. On top of that capture loop it offers **input automation** (text staging, keyboard actions, a panic stop) and a separate **system-wide autocomplete** that suggests inline text completions in any focused field.

Everything runs locally. Capture, OCR, and summarization happen on your machine using the local model (Ollama); nothing is sent to the cloud as part of this feature.

> **Platform:** Screen Intelligence is **macOS-only** in V1. On Windows and Linux the engine reports `platform_supported: false`, `start_session` returns a `macOS-only` error, and the embedded capture server does not autostart. Microphone permission detection is the only cross-platform piece.

***

## What it does

A session runs two cooperating background workers:

| Worker | Role |
| --- | --- |
| **Capture worker** | Polls the foreground window at `baseline_fps` (default 1 fps), screenshots the **active window only** via `screencapture -l <windowID>` (no fullscreen fallback), applies the allow/deny policy, optionally saves a PNG, and enqueues the frame. |
| **Processing worker** | Drains to the **latest** frame, compresses it (PNG → JPEG, longest edge ≤ 1024 px, quality 72), runs Apple Vision OCR, then a vision LLM, then a synthesis LLM, and persists a `VisionSummary` to memory. |

Capture only proceeds when the foreground context exposes a `window_id`. Stale queued frames are discarded. Only the most recent frame is analyzed, deduped by capture timestamp.

### Sessions

Sessions are explicitly consent-gated and time-boxed:

* You start a session from Settings (or the `screen_intelligence.start_session` RPC) with `consent: true`.
* TTL is clamped to **30 to 3600 seconds** (the Settings panel defaults to 300 s). When the TTL expires the session stops on its own.
* **Analyze now** flushes the pending frame through the vision pipeline immediately.
* **Stop** ends the session; the `panic_stop` input action force-stops it instantly.

***

## Input automation

While a session is active, `input_action` lets the agent stage text and signal keyboard intent into the session's autocomplete context. Actions are blocked when no session is active or when the foreground app is denylisted by the active policy. The special `panic_stop` action immediately tears down the session regardless of state. It is a hard kill switch.

The capture-side autocomplete helpers (`autocomplete_suggest` / `autocomplete_commit`) maintain a small in-memory context buffer (capped at 256 chars) and return heuristic suggestions; they're gated on `autocomplete_enabled`.

***

## Autocomplete

Separate from the capture session, OpenHuman ships a **system-wide inline autocomplete** engine (the `autocomplete` domain). It is also **macOS-only** at runtime.

* It captures your currently-focused text field through the macOS accessibility (AX) layer, runs **local** inference to generate a short single-line continuation, and renders it in a floating overlay badge.
* Press **Tab** to accept (it inserts the text and cleans up any stray indentation the app added), or **Escape** to reject.
* Accepted completions are saved as personalization examples in a local KV store and a local memory-doc namespace, and feed back into later suggestions.
* It special-cases terminals (extracting just the input line), skips blocked/disabled apps and OpenHuman's own window, and filters low-quality suggestions (too short, no alphanumerics, or an echo of what you just typed).
* Debounce is clamped between 50 and 2000 ms; the displayed/applied suggestion is capped at 64 characters. After 5 consecutive inference failures the engine auto-stops to avoid notification floods.

There is also an in-app path: the OpenHuman composer passes an explicit `context` to the engine, bypassing AX capture entirely.

***

## Permissions

Capture and automation require macOS privacy grants. OpenHuman detects each one and can open the relevant **System Settings → Privacy & Security** pane and trigger the system prompt.

| Permission | Why it's needed | Detected via |
| --- | --- | --- |
| **Screen Recording** | Screenshot the active window for OCR + vision. | `CGPreflightScreenCaptureAccess` |
| **Accessibility** | Read the foreground window/element, capture focused text, and insert text (required to start a session and to run autocomplete). | `AXIsProcessTrusted` |
| **Input Monitoring** | Listen for accept/reject key edges (Tab/Escape) and the Globe/Fn hotkey. | `IOHIDCheckAccess` |
| **Microphone** | Voice features (cross-platform; the only permission detected off macOS). | CPAL device probe |

> **Restart after granting.** macOS TCC grants are per-executable **and per-process**, so a running core never sees a freshly granted permission. After you grant a permission you must restart the core for it to take effect. The status payload carries `permission_check_process_path` and the core process pid/start time so the UI can confirm a restart actually happened. The panel exposes a "refresh with restart" action for this.

***

## Privacy considerations

* **Local-only processing.** OCR (Apple Vision) and both LLM passes run on-device via the local model. The vision pipeline requires `local_ai.runtime_enabled = true` with the `ollama` provider; without it, analysis errors out rather than falling back to a cloud call.
* **What's stored.** Each summary is written to **unified memory** in the `background` namespace (source type `screenshot`, tagged `screen_intelligence`) as a small markdown doc with the app name, window title, capture timestamp, and confidence. Raw frames are **not** persisted to memory.
* **Screenshots on disk are opt-in.** PNGs are only saved to `{workspace}/screenshots/` when **Keep screenshots** is enabled; otherwise a temp file is written for OCR and deleted immediately.
* **Allow/deny policy.** A policy mode (`all_except_blacklist` or `whitelist_only`) plus allow/deny app lists control which windows are ever captured.
* **Time-boxed + consent-gated.** Nothing captures until you start a session, and every session expires on its TTL.

***

## Enabling & configuring

Configure it under **Settings → Screen awareness** (the `ScreenIntelligencePanel`):

* **Permissions section**: per-permission status (granted / denied / unknown), request buttons, and the restart-to-refresh flow.
* **Enabled**: master toggle for the feature.
* **Mode**: `All except blacklist` or `Whitelist only`, backed by the allow/deny lists.
* **Screen monitoring**: toggle the capture loop for a session.
* **Session controls**: Start / Stop / **Analyze now**, with a live remaining-time countdown. Start is disabled until Accessibility is granted and the platform is supported.

Under the hood these map to the `[screen_intelligence]` config block (`enabled`, `vision_enabled`, `use_vision_model`, `keep_screenshots`, `baseline_fps`, `session_ttl_secs`, `policy_mode`, allow/deny lists, `autocomplete_enabled`) and the `screen_intelligence.*` JSON-RPC surface. The same capabilities are available from the `openhuman screen-intelligence` CLI (`status`, `start`, `stop`, `capture`, `doctor`, `run`).

***

## See also

* [Memory Tree](obsidian-wiki/memory-tree.md): where vision summaries land.
* [Privacy & Security](privacy-and-security.md): the broader permission and data model.
* [Voice](native-tools/voice.md): the other feature that uses the Microphone permission.
