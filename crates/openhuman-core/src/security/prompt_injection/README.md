# Prompt injection

TinySecurity owns prompt rules and scores. This module maps its bus response to
OpenHuman's decision type, adds a content-free hash and trace fields, and fails
closed when the module cannot answer. The same adapter screens user input and
remote tool definitions.

## Public surface

- `enforce_prompt_input(input, context).await` asks `ScanPrompt2`. `Allow`
  continues; `Review` and `Block` stop the turn. Oversized input, timeouts,
  unavailable modules and missing dispatch configuration become `Review`.
- `scan_tool_definition(field, text).await` asks `ScanToolDefinition`; a
  missing answer is reported as a finding.
- `PromptEnforcementContext` carries source and optional request, user and
  session identifiers for content-free tracing.
- `PromptEnforcementDecision` carries verdict, score, rule codes, action,
  SHA-256 prompt hash and character count. Prompt contents are never logged.

The MCP registry sends the full remote tool name, description, schema and
metadata to TinySecurity before exposing a tool. Existing prompt enforcement
call sites remain in the agent bus, chat entry, inbound, session and host
security-gate paths.

## Ownership

Detection rules, normalization and thresholds live in TinySecurity. This host
adapter does not keep a local rule set or permit an unavailable scanner to
allow content. The oversized prompt guard preserves the module's 64 KiB
fail-closed contract.
