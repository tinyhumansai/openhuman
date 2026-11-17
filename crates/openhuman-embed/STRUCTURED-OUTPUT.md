# Structured output for embedding hosts

`Completer` and runtime-owned `Agent::turn` validate requested JSON locally.
Parseable JSON is not sufficient: `JsonSchema` applies the full schema,
including numeric bounds, combinators and local references. External schema
retrieval is disabled even when another dependency enables retrieval features.
Invalid schemas fail before provider dispatch. `JsonObject` requires an object.
A `length` or `max_tokens` finish is refused even if the text happens to parse.

Repair is explicit: `.structured_retries(2)` allows two additional terminal
answer attempts. The default is zero and values above three are rejected before
dispatch. Agent tools can run between attempts; their calls count towards the
ordinary harness limits. The existing TinyAgents output-validator loop owns
agent repair; Embed does not implement a second agent loop. Every model call
still passes through the configured budget and route.

Failures are `CoreError::StructuredOutput { method, failure }`. The failure
contains classification, terminal attempt count, last answering model and finish
reason, and reported usage. It contains no original answer or validation error
that might quote a secret. Repair feedback describes the failure without
feeding the invalid answer back. Completion usage sums all repair attempts;
unknown cost stays unknown rather than treating an unreported call as free.

`Turn::require_tool_call(true)` additionally requires a successful tool execution
before accepting the answer. A proposed, rejected or failed tool call is not a
successful read. Before execution the provider gets `tool_choice: required` and
no final response format; afterwards it receives the requested answer schema.
The host also enforces this requirement when a provider ignores the wire hint.

Use a `HostOnly`, read-only agent with host-owned application tools
for untrusted application input. Validation does not grant tool or write permissions.

Tests: `structured_validation`, `structured_turns`, `tool_required_routing`, and
`completion_routing` use loopback provider fixtures and require no model key.
