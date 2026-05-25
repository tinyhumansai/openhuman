# MCP Setup Agent — design sketch

A sub-agent that walks the user through installing, configuring, and
connecting an MCP server from one of the upstream registries
(`mcp_registry::registries`: Smithery, modelcontextprotocol/registry).

This document is a **design sketch** for follow-up implementation. Nothing
here is wired up yet beyond the underlying primitives in
`src/openhuman/mcp_registry/`.

---

## Goal

A non-technical user says *"set up the Notion MCP server for me"*. The
agent:

1. Browses the enabled registries, finds the candidate, summarises it.
2. Asks for any secrets the server requires (API keys, OAuth tokens, …)
   **without ever pulling the values into the LLM context**.
3. Test-connects with the collected secrets, surfaces errors, lets the
   user retry / change values.
4. On success, persists the install and the secrets, runs boot-spawn for
   this one server, returns connection status + the tool list now
   available to the main agent.

The agent owns the conversation; the core owns the secrets, the
subprocess, and the persistence.

---

## Tool surface

Four tools registered behind a `mcp_setup_*` namespace. All tool inputs
and outputs are JSON; secret values **never** appear in either direction.

| Tool | Input | Output | Notes |
| --- | --- | --- | --- |
| `mcp_setup_search` | `{ query?, page?, page_size?, source? }` | `{ servers: [Summary], total_pages }` | Thin wrapper over `mcp_registry::registry::registry_search`. `source` optionally scopes to one upstream. |
| `mcp_setup_get` | `{ qualified_name }` | `{ detail, required_env_keys }` | Wraps `registry_get`; pre-computes `required_env_keys` from the `config_schema` (same logic as `ops::collect_required_env_keys`). |
| `mcp_setup_request_secret` | `{ key_name, prompt }` | `{ ref: "secret://<opaque>" }` | Triggers an out-of-band UI prompt. Returns an opaque ref; raw value is held in a process-local in-memory map keyed by ref. |
| `mcp_setup_test_connection` | `{ qualified_name, env_refs: { KEY: "secret://…" } }` | `{ ok, tools?: [McpTool], error?: string }` | Spawns the candidate subprocess in a **scratch** workspace, resolves refs to values just-in-time, runs `initialize` + `tools/list`, tears it down. No persistence. |
| `mcp_setup_install_and_connect` | `{ qualified_name, env_refs }` | `{ server_id, status, tools: [McpTool] }` | Resolves refs, persists the install + `mcp_client_env` rows, calls `connections::connect`. Refs are consumed (removed from the in-memory map) regardless of outcome. |

---

## Secret flow — opaque refs

The hard requirement: **raw secret values must not enter LLM context**.
Opaque refs solve this cleanly:

```
agent: mcp_setup_request_secret({ key_name: "NOTION_API_KEY", prompt: "Notion integration token" })
core:  → pushes prompt to UI; user types into a native input box
core:  ← receives value, stores in SETUP_SECRETS: HashMap<RefId, String>
core:  → returns { ref: "secret://7c9f2e" }   ← the agent sees only this
agent: mcp_setup_test_connection({
         qualified_name: "@notion/server",
         env_refs: { "NOTION_API_KEY": "secret://7c9f2e" }
       })
core:  → for each ref, look up the value in SETUP_SECRETS, build the env
         vector, spawn, init, list_tools, tear down
core:  ← returns { ok: true, tools: [...] }   ← still no raw value to agent
```

Lifecycle of `SETUP_SECRETS`:

- Process-local `OnceLock<RwLock<HashMap<RefId, SecretEntry>>>`.
- Entries TTL out after, say, 15 min (defends against stranded secrets if
  the conversation is abandoned mid-flow).
- `mcp_setup_install_and_connect` consumes refs on success: pulls each
  value, writes it to the `mcp_client_env` table (existing persistence,
  already keyed by `server_id`), removes the ref. On failure refs are
  left intact so the agent can retry without re-prompting the user.
- On core shutdown the map is dropped — refs do not survive restart.

`RefId` is a short random hex string. **No structure or hint of the
underlying value** so the agent has nothing useful to leak even if it
tries.

### Why not just take key names?

Considered (option 2 in the original AskUserQuestion). Rejected because:

- The agent can't decide between values it just collected — e.g. trying
  two different tokens to pick the one that works requires distinguishing
  them, which requires handles.
- Tying secrets to the `(server_id, key)` pair too early means a failed
  test-connect leaves stale rows in `mcp_client_env` for an
  uninstall-rolled-back server.

Opaque refs give the agent enough handle to iterate without exposing
values.

---

## Where the agent lives

Follow the existing sub-agent pattern (`src/openhuman/agent/harness/`):

- New archetype TOML at `app/src/lib/ai/agents/mcp_setup.toml` (loaded by
  `AgentDefinitionRegistry::init_global`).
- Prompt + tool allowlist scoped tight: only the four `mcp_setup_*` tools
  plus the standard `chat` / `ask_user` primitives. **No** general
  filesystem, network, or shell tools — the agent shouldn't be able to
  exfiltrate a leaked ref even if one shows up.
- Triggered by the main agent via `spawn_subagent("mcp_setup", { goal })`
  or by an explicit UI affordance ("Add MCP server…" button that opens a
  thread pinned to this archetype).

---

## Implementation outline

Following the project's `Specify → Rust → JSON-RPC → UI → tests` flow:

1. **Rust core** (in `src/openhuman/mcp_registry/`):
   - New module `setup.rs` owning `SETUP_SECRETS` (in-memory ref map with
     TTL) and helpers `mint_ref`, `resolve_refs(env_refs) -> Vec<(K,V)>`,
     `consume_refs(env_refs)`.
   - New module `setup_ops.rs` with the four handlers.
   - Wire schemas in `schemas.rs`, controllers in `core/all.rs`.
2. **Tool-side bridge** so the agent harness sees the four tools as
   regular tool defs. Reuse the controller-to-tool generator already
   used elsewhere.
3. **UI**: out-of-band secret prompt component (probably a `chat`-pinned
   modal listening on a new socket event `mcp_setup_request_secret`),
   submit POSTs the value to a Tauri command that calls into core to
   register the ref.
4. **Archetype** + system prompt at `app/src/lib/ai/agents/mcp_setup.toml`.
5. **Tests**:
   - Unit: ref lifecycle (mint → resolve → consume → TTL expiry).
   - Integration (`tests/mcp_registry_e2e.rs` style): full flow against
     the existing `test-mcp-stub` binary, asserting refs vanish after
     install + that test-connect failures leave refs intact.

---

## Open questions for the implementer

- TTL value — 15 min is a guess; calibrate against typical install flow.
- Should `test_connection` accept a partial env_refs (some refs, some
  literal-by-name) for iteration? Current design says refs only, which
  forces consistency.
- The official MCP registry returns servers with **multiple package
  ecosystems** (`packages: [{ registry_name: "npm" | "pypi" | … }]`). The
  setup agent needs to either pick one or ask the user. Add a
  `package_choice` step or default to npm?
- Telemetry: log `mcp_setup_*` calls (`tracing::info!` is fine) but
  never log ref values, never log env values, only key names.

---

## Anti-goals

- The setup agent is **not** a generic "ask user for any data" surface.
  Its prompt tool is scoped to MCP env values, full stop.
- It does **not** persist anything until `install_and_connect` succeeds.
  No half-installed rows in `mcp_servers` or `mcp_client_env`.
- It does **not** read back secrets. Once persisted into `mcp_client_env`
  they are write-only from the agent's perspective; only the subprocess
  spawn path in `connections::connect` reads them.
