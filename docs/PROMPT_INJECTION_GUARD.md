# Prompt Injection Guard

This document describes the end-to-end prompt injection detection and enforcement flow added in the OpenHuman core and app.

## Scope

- Backend enforcement is authoritative.
- Frontend checks are advisory UX only.
- Guarding runs before model inference or agent/tool loop execution for user-supplied prompts.

## Detection Layers

Detection is implemented in `src/openhuman/prompt_injection/` with layered analysis:

1. Normalization
- Lowercasing and whitespace collapse.
- Obfuscation cleanup (zero-width chars, punctuation noise, basic leetspeak substitutions).
- Compact-string pass for spaced-out attacks (`i g n o r e ...`).

2. Pattern rules
- Instruction override patterns (`ignore/disregard/forget previous instructions`).
- Role hijack patterns (`you are now`, `developer mode`, `jailbreak`).
- Prompt/system exfiltration patterns (`reveal system prompt`, `show developer instructions`).
- Secret exfiltration patterns (`api key`, `token`, `password`, etc.).
- Unsafe tool coercion patterns.

3. Optional classifier
- Enabled with `OPENHUMAN_PROMPT_INJECTION_CLASSIFIER=heuristic`.
- Adds score for suspicious combinations (obfuscation + override/exfiltration intent).

## Verdict Contract

Detector returns:

- `verdict`: `allow | block | review`
- `score`: normalized `0.0..1.0`
- `reasons`: stable reason codes/messages
- `action`: enforcement action (`allow`, `block`, `review_blocked`)

Current threshold policy:

- `score >= 0.70` -> `block`
- `0.45 <= score < 0.70` -> `review`
- `< 0.45` -> `allow`

## Enforcement Flow

Server-side checks are wired before LLM/tool execution in:

- `src/openhuman/channels/providers/web.rs` (`start_chat`)
- `src/openhuman/local_ai/ops.rs` (`agent_chat`, `agent_chat_simple`, `local_ai_chat`, `local_ai_prompt`, `local_ai_vision_prompt`, `local_ai_summarize`)
- `src/openhuman/agent/harness/session/runtime.rs` (`Agent::run_single`)
- `src/openhuman/agent/bus.rs` (`agent.run_turn` native bus handler)

If action is `block` or `review_blocked`, request processing is stopped and no prompt is sent to provider/tool loop.

## Frontend Advisory UX

- Advisory pre-submit validation in `app/src/chat/promptInjectionGuard.ts`.
- Composer integration in `app/src/pages/Conversations.tsx`.
- `block` verdict: advisory warning is shown client-side; backend remains authoritative for final enforcement.
- `review` verdict: advisory warning shown; backend still enforces final decision.

## Logging and Privacy

Each backend decision logs:

- `request_id`
- `user_id`
- `session_id`
- `source`
- `verdict`
- `score`
- `reasons` (codes)
- `action`
- `prompt_hash` (SHA-256)
- `prompt_chars`

Raw prompt text is not logged by this guard.

## Tests

- Unit tests:
  - `src/openhuman/prompt_injection/tests.rs`
  - `src/openhuman/channels/providers/web_tests.rs`
  - `src/openhuman/local_ai/ops_tests.rs`
  - `app/src/chat/__tests__/promptInjectionGuard.test.ts`
- Integration test:
  - `tests/json_rpc_e2e.rs` (`json_rpc_prompt_injection_is_rejected_before_model_call`)

## Extending Rules

1. Add/adjust regex rules in `src/openhuman/prompt_injection/detector.rs` (`DETECTION_RULES`).
2. Keep reason codes stable for observability and tests.
3. Add unit tests for both positive and negative cases (including obfuscated variants).
4. If introducing new classifier logic, gate it behind config/env and ensure deterministic fallback behavior when disabled.
