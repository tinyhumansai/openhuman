# tests

End-to-end suites for `openhuman-embed`. Each file builds a real `Runtime`
(or `Harness`) in-process and drives turns against `wiremock` servers that
stand in for the model provider and, where needed, the backend. No suite
calls a live LLM or a real service. Cargo discovers these files on its own;
this crate declares no `[[test]]` entries.

## How the suites are shaped

A runtime claims a process-wide slot (the core's keyring, event bus and
domain subscribers are process-scoped), so each file is its own test binary
and usually holds a single `#[test]` that builds one runtime and checks
everything against it. Splitting the assertions into separate tests would
either race for the slot or need a mutex. [`attached_tools.rs`](attached_tools.rs) holds a static
`RUNTIME_LOCK` for the same reason.

Each `#[test]` builds its tokio runtime with `common::runtime()`, which sets
the large worker stack a turn needs, instead of using `#[tokio::test]`.

Routing assertions come from the mocks: each agent gets its own provider
mock, so a turn that leaked to another agent's endpoint shows up as a request
recorded on the wrong server.

`session_codecs.rs` uses no runtime: it verifies that the public session-store
record types preserve populated persisted JSON, including metadata and lineage.

## Layout

| File | What it proves |
| --- | --- |
| [`common/mod.rs`](common/mod.rs) | Shared helpers: `runtime()`, `offline_config()`, `provider(reply)`, `stub_backend()`, `chat_completion`, `tool_call_completion`, `echo_inference()` with `PointedInference` (managed inference to a mock), and request inspectors (`chat_requests`, `tool_names`, `tool_results`, `last_user_message`). |
| [`facades.rs`](facades.rs) | The runtime-free curated facades (`artifacts`, `chat_surface`, `identity`, `config` helpers, `modules`), the compile-status constants and `schema_for_rpc_method`. |
| [`seams_runtime.rs`](seams_runtime.rs) | A controller extension is invokable after a real `build()`, a `live_policy` is applied after boot, and hooks are removed on drop while the extension outlives the runtime. |
| [`harness_embed.rs`](harness_embed.rs) | A `Harness` runs a real turn with no transport and no background services, routes it to the mock provider, and binds nothing. |
| [`runtime_agents.rs`](runtime_agents.rs) | Several agents on one runtime keep their own provider, access tier, skills, MCP servers and working directory, and the runtime's API key reaches a mocked managed backend as a bearer. |
| [`session_store.rs`](session_store.rs) | On a `Stateless` workspace with `InMemorySessionStores`, transcripts, journal and run status land in the store per agent, a reopened agent resumes from the store, and nothing durable is written to the scratch directory. |
| [`attached_tools.rs`](attached_tools.rs) | `Agent::attach_tools` sources survive clones and session resume, and name collisions are refused. |
| [`composio_agents.rs`](composio_agents.rs) | Two agents with their own `ComposioHostCredential` reach Composio with their own key only. |
| [`memory_facade.rs`](memory_facade.rs) | `Runtime::memory` over TinyMemory's in-memory reference engine keeps two tenant roots apart. |
| [`saas_profiles.rs`](saas_profiles.rs) | A `ProfileRuntime` (SaaS mode, in-process): two users on thread `t1` see only their own messages and ride their own credential, a held `ProfileHandle` keeps its profile from release, a relayed Telegram message lands on the user's `channel:` thread with its `channel_outbound` reply on that user's events only, and the process refuses any other core afterwards. Inference is `common::echo_inference` behind `common::PointedInference`. |
| [`host_only_tools.rs`](host_only_tools.rs) | Host tools declare read-only requirements through Embed's `ToolPolicy`; the exact host catalog is advertised and built-in shell/write calls are refused. |
| [`structured_turns.rs`](structured_turns.rs) | Host-owned document tools preserve structured response shape, usage, budgets and untrusted-input restrictions across the tool loop. |
| [`completion_routing.rs`](completion_routing.rs) | Ordered endpoint fallback, bounded 2x/4x truncation retries, final unpinned gateway routing, images and accounting across all attempts. |
| [`tool_required_routing.rs`](tool_required_routing.rs) | Native host tool metadata for GPT, Kimi and MiniMax model IDs; premature JSON refusal; required successful execution before final schema; gateway options survive. |
| [`completion_cancellation.rs`](completion_cancellation.rs) | Cancellation acknowledged after the provider future stops, pre-cancelled calls make no request, and deadlines are typed. |
| [`structured_validation.rs`](structured_validation.rs) | Full schema constraints, invalid/external schema refusal before dispatch, typed failures and bounded repair usage. |
| [`turn_observers.rs`](turn_observers.rs) | Terminal error privacy, explicit input capture, and exactly one terminal callback on dispatch failure. |
| [`observed_turns.rs`](observed_turns.rs) | Actual model and host-tool observations survive the core runtime task hop; payloads require consent; actual model, finish reason and reasoning usage. |
| [`public_api.rs`](public_api.rs) | Compile-time check that the host-facing types and signatures stay exported. |
| [`turn_steering.rs`](turn_steering.rs) | A steer sent during a plain chat turn's only model call is applied after the draft answer and earns one more call in the same turn; a steer queued before send reaches the first call; a steer after the turn ended is refused with its text; a cancelled turn hands an undelivered steer back. |
| [`turn_cancellation.rs`](turn_cancellation.rs) | Cancellation before send, during inference and during a builtin shell command; repeated requests and agent reuse. |
| [`process_cancellation.rs`](process_cancellation.rs) | On Linux, dropping a command future kills its shell descendants. |
| `inline_permissions.rs` | An inline UI decision precedes execution; concurrent agents and one-turn denials stay isolated. |
| `usage_hooks.rs` | Per-agent stop policy and per-turn cumulative usage observation, with provider charges and no extra calls after a stop. |
| [`turn_tools.rs`](turn_tools.rs) | One-turn belt replacement/revocation, resumed-session schemas and independent concurrent workers. |
| [`tool_environment.rs`](tool_environment.rs) | Overlapping child environments exclude inherited variables and stay within their own scopes. |
| [`scoped_hooks.rs`](scoped_hooks.rs) | Same-named runtime, agent and turn callbacks remain additive and isolated during concurrent turns, resumed sessions and reuse of a removed agent id. |
| [`route_headers.rs`](route_headers.rs) | Attribution headers and bearer follow only the per-turn route, without reaching other agents, later turns or backend calls. |
| [`tool_hook_context.rs`](tool_hook_context.rs) | Hook agent/session identities and cwd agree with builtin shell execution on an overridden and a default working root. |

## Running

[`runtime_agents.rs`](runtime_agents.rs) gates its skills and MCP assertions on the `skills` and
`mcp` features, so run with them on:

```bash
cargo test -p openhuman-embed --features inference,mcp,skills
cargo test -p openhuman-embed --features inference,mcp,skills --test session_store
pnpm debug rust session_store
```

Add a new scenario as a new file rather than a second `#[test]` in an
existing one, and reuse [`common/`](common/) for mocks.

## Further reading

- [`docs/gitbooks/en/developing/embedding.md`](../../../docs/gitbooks/en/developing/embedding.md): embedding the core in another product.
- [`docs/gitbooks/en/developing/testing-strategy.md`](../../../docs/gitbooks/en/developing/testing-strategy.md): testing strategy.
- [`crates/openhuman-embed/README.md`](../README.md): the openhuman-embed crate README.
