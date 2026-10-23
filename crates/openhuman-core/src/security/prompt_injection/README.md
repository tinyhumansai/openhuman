# prompt_injection

Prompt-injection screening and enforcement. The rules, weights and thresholds are the TinySecurity module's (`ScanPrompt2`: an additive score in thousandths, the `Allow` / `Review` / `Block` tier it falls in, and the content-free rule codes that fired; `ScanToolDefinition` for remote tool definitions). The host asks through the [`PromptScanner`](./scanner.rs) port and enforces the answer: callers run `enforce_prompt_input` before any model or tool execution path, and `Review` and `Block` both stop the input.

## Responsibilities

- Hold the **scanner port** (`scanner.rs`): production asks the module (`module_scanner.rs`); a build without the module asks nothing and every text is held; a test installs a stub with `with_scanner` (task scope) or `install` (process). No production path branches on `cfg(test)`.
- Bound the hot path. Every tool output and every web, channel and stored piece of content is screened (`agent/tinyagents/host/security_gate.rs::screen_input`), so each module call has a deadline (`CALL_DEADLINE`, 2 s), the whole screen of one text has one (`TOTAL_DEADLINE`, 10 s), and long text is scanned in overlapping windows (`WINDOW_BYTES` 48 000, `OVERLAP_BYTES` 4 096) because the module accepts 64 KiB per call. Text past `MAX_SCANNED_BYTES` (8 MiB) is held, never scanned in part.
- **Fail closed to Review.** A scanner that is absent, errors, times out or is handed too much answers `Review` with `screen.unavailable`, `screen.timeout` or `screen.too_large`. It never answers `Allow`. The screens are enforcing, not advisory, so a held text is refused.
- Map the tier to the host's action (`enforce.rs`): `Allow` to `Allow`, `Review` to `ReviewBlocked`, `Block` to `Blocked`, and describe each rule code in words a person is shown. A structured `tracing::info!` audit line carries the hash, never the prompt text.
- Screen remote MCP tool definitions through the same port (`screen_tool_definition`): `mcp::registry::tools_safe_for_agent` is async and drops a tool the module flags (`Suspicious` or `Blocked`) or that it could not scan.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Docstring and re-exports. |
| `types.rs` | `PromptEnforcementDecision`, `PromptEnforcementAction`, `PromptInjectionVerdict`, `PromptInjectionReason`, `PromptEnforcementContext`. |
| `scanner.rs` | The `PromptScanner` port, `screen`, `screen_tool_definition`, windowing, deadlines, the fail-closed merge. |
| `module_scanner.rs` | The production scanner over `ScanPrompt2` / `ScanToolDefinition`. |
| `enforce.rs` | `enforce_prompt_input`. |
| `scanner_stub.rs` | The test stub (`#[cfg(test)]` module, as `approval/engine_stub.rs`). |

The detector, its rule table and its test corpus moved into the module (`tinysecurity-redact`, `score.rs` and `score_tests.rs`), which is where what is an injection is decided and pinned.

## Public surface

- `enforce_prompt_input(input, context).await -> PromptEnforcementDecision`
- `screen(text).await -> PromptScan`, `screen_tool_definition(name, definition).await -> Result<DefinitionScan, String>`
- `PromptScanner`, `install`, `with_scanner`, `windows`, `Tier`, `PromptScan`, `DefinitionScan`, `DefinitionVerdict`
- the decision types above

## Configuration

None. The host's former `OPENHUMAN_PROMPT_INJECTION_CLASSIFIER` environment classifier is not part of the module.

## Persistence

None.
