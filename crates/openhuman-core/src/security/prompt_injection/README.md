# prompt_injection

Centralized, deterministic prompt-injection screening. Given a user-provided prompt and a small enforcement context, it normalizes the text, scores it against a set of regex rules plus a couple of heuristics, and returns a verdict (`Allow` / `Review` / `Block`) and a corresponding enforcement action. Callers run this **before** any model or tool execution path so adversarial prompts (instruction overrides, role hijacks, system-prompt / credential exfiltration, tool-abuse coercion) are caught up front. There is no persisted state, no RPC surface, and no event subscriber — it is a pure synchronous analysis library exposed via a single function.

## Responsibilities

- Normalize incoming prompts to defeat common obfuscation: lowercasing, leet-speak (`0→o`, `1→i`, `@→a`, …), Cyrillic homoglyph folding, fullwidth-ASCII folding, zero-width / bidi / formatting-character stripping, and whitespace collapse (plus a whitespace-stripped `compact` variant).
- Score the prompt against deterministic `DETECTION_RULES` (compiled once into a single `RegexSet` DFA, matched across the `lowered` / `collapsed` / `compact` variants) and two inline heuristics (`has_instruction_override`, `has_exfiltration_intent`).
- Optionally apply an env-gated `HeuristicClassifier` that adds a small bounded score for suspicious trait combinations.
- Map the summed score to a verdict via fixed thresholds: `Block ≥ 0.70`, `Review ≥ 0.55`, else `Allow`.
- Produce a `PromptEnforcementDecision` carrying verdict, score, reason codes/messages, action, a SHA-256 prompt hash, and prompt char count, and emit a structured `tracing::info!` audit line (PII-safe: logs the hash, not the prompt text).

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/security/prompt_injection/mod.rs` | Module docstring + re-exports of the public surface. No logic. |
| `crates/openhuman-core/src/security/prompt_injection/detector.rs` | All logic: types, normalization, detection rules + `RegexSet`, heuristics, optional classifier, scoring/thresholds, and the `enforce_prompt_input` entry point. |
| `crates/openhuman-core/src/security/prompt_injection/prompt_injection_tests.rs` | `#[cfg(test)]` suite (~24 cases) covering allow/review/block verdicts, obfuscation handling, and known false-positive regressions (TAURI-140, issue #1940). |

## Public surface

Re-exported from `mod.rs` (all defined in `detector.rs`):

- `enforce_prompt_input(input: &str, context: PromptEnforcementContext) -> PromptEnforcementDecision` — the entry point for user-facing prompts.
- `scan_tool_definition(field: &str, text: &str) -> Option<ToolDefinitionScanHit>` — same rules applied to remote MCP tool descriptions/titles; any non-`Allow` verdict is a hit and the registry rejects the tool.
- `PromptEnforcementContext<'a>` — borrowed `source` plus optional `request_id` / `user_id` / `session_id` for the audit log.
- `PromptEnforcementDecision` — `{ verdict, score: f32, reasons, action, prompt_hash: String, prompt_chars: usize }`.
- `PromptEnforcementAction` — `Allow` / `Blocked` / `ReviewBlocked`.
- `PromptInjectionVerdict` — `Allow` / `Block` / `Review` (serde `lowercase`).
- `PromptInjectionReason` — `{ code: String, message: String }`.

Internal-only (not exported): `DetectionRule`, `NormalizedPrompt`, `HeuristicClassifier`, `OptionalClassifier`, `analyze_prompt`, `normalize_prompt`, `prompt_hash`.

## Configuration

- `OPENHUMAN_PROMPT_INJECTION_CLASSIFIER` (env) — resolved once via `Lazy`. `"heuristic"` enables `HeuristicClassifier`; anything else (default `"off"`) disables the optional classifier. The active choice is logged at `debug`.

## Persistence

None. The module is stateless aside from `Lazy` statics (compiled regexes, the classifier selection).

## Dependencies

No `crate::*` or `crate::core::*` dependencies — the module is self-contained. External crates only:

- `regex` (`Regex`, `RegexSet`) — pattern matching / batched DFA.
- `once_cell::sync::Lazy` — compile-once statics.
- `serde` — derive on the verdict/reason types.
- `sha2` + `hex` — SHA-256 prompt hashing for audit logs.
- `tracing` — structured audit and debug logging.
- `std::env` — classifier selection.

## Used by

Consumers call `enforce_prompt_input` and treat any non-`Allow` action as a rejection (returning a user-facing guard message):

- `crates/openhuman-core/src/agent/session_host/runtime.rs` — gates agent session turns; emits `prompt_injection_blocked`.
- `crates/openhuman-core/src/agent/bus.rs` — screens inbound prompts on the agent event path.
- `crates/openhuman-core/src/agent/tinyagents/host/security_gate.rs` — `screen_input` stage of the tinyagents security gate.
- `crates/openhuman-core/src/mcp/registry/mod.rs` — `scan_tool_definition` on remote tool metadata at registration.
- `crates/openhuman-core/src/web_chat/` — screens chat payloads at the web channel ingress (`start_chat`).
- `crates/openhuman-core/src/inference/local/ops.rs` — rejects injected prompts before local-AI runtime execution.
- `crates/openhuman-core/src/platform/about_app/catalog.rs` — surfaces the `conversation.prompt_injection_guard` capability entry.
- `crates/openhuman-core/src/core/observability.rs` — classifies `prompt_injection_blocked` error messages for telemetry (string-based, not a code dependency).

## Notes / gotchas

- **Three-variant matching is load-bearing**: rules are matched against `lowered`, `collapsed`, and the whitespace-stripped `compact` strings, so spacing-obfuscated attacks (`j a i l b r e a k`, `j w t`) still contribute to score/reasons.
- **Threshold history is encoded in comments**: `Review` was tuned 0.45 → 0.50 → 0.55 and the obfuscated-instruction signal bumped to 0.56 to eliminate a false-positive band while keeping spaced-out overrides at Review level (TAURI-140).
- **Deliberately conservative verb list**: `exfiltrate.credentials_with_intent` excludes high-false-positive verbs (`show`, `give`, `tell`, `fetch`, `return`, `output`) and requires a determiner within a bounded window, so benign technical questions ("show me the password reset flow", "reveal how to set my api key") do not trip it (issue #1940).
- `is_obfuscation_char` is the single source of truth shared between the `had_zwsp` flag and the stripping step to prevent drift.
- `Review` and `Block` both yield non-`Allow` actions; every current caller treats them identically (rejection), so the distinction is informational/audit-only at the call sites.
