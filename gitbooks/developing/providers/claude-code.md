# Claude Code CLI provider

OpenHuman can route any chat workload through Anthropic's `claude` CLI instead of calling the Anthropic HTTP API directly. The CLI handles model selection, auth, and prompt-cache management; OpenHuman drives it as a long-lived, session-resuming child process, parses its stream-json output, and hands it an MCP endpoint so the model can reach native OpenHuman state (memory, threads, agents, search). This is one of several pluggable LLM backends; see [Engines](../engines.md) for the full list.

The provider itself, `ClaudeCodeProvider`, is owned by `tinyagents-harness` (vendor/tinyagents), not by OpenHuman: see [its README](../../../vendor/tinyagents/crates/tinyagents-harness/src/providers/claude_code/README.md) for the full file map and implementation notes. OpenHuman only wires it up in `crates/openhuman-core/src/inference/provider/factory/subprocess_providers.rs`, which supplies the MCP endpoint (`OpenHumanMcpEndpoint`, below) and reads the resulting model string back into its own routing.

## Requirements

- Claude Code CLI **≥ 2.0.0** (`MIN_CLI_VERSION`), either on `PATH`, at one of the supported
  well-known install locations, or selected with
  `OPENHUMAN_CLAUDE_CLI=/abs/path/to/claude`.
- An Anthropic API key in `ANTHROPIC_API_KEY`, **or** a pre-existing `~/.claude/.credentials.json` from `claude login`.
- The `http-server` Cargo feature (on by default): OpenHuman exposes its MCP tools to the CLI over a loopback HTTP endpoint (`crate::mcp::server::ensure_local_http`), not by spawning `openhuman-core mcp` as a subprocess. Without that feature, the provider still runs, but the CLI gets no OpenHuman tools.

## Routing a workload through the CLI

The factory grammar accepts a new prefix: `claude-code:<model>[@<temperature>]`. Apply it via the standard inference settings (per-role, locked decision #3):

```bash
# Through the JSON-RPC update endpoint:
openhuman-core rpc openhuman.inference_update_model_settings \
  --json '{"chat_provider":"claude-code:claude-sonnet-4-5"}'
```

| Role string          | Field updated                    |
| -------------------- | -------------------------------- |
| `chat_provider`      | foreground chat replies          |
| `reasoning_provider` | long-context reasoning workloads |
| `agentic_provider`   | multi-step agentic loops         |

A workload set to `claude-code:<model>` always spawns a fresh `claude` child per turn; concurrency is capped at `MAX_CONCURRENT_TURNS = 4` per `ClaudeCodeProvider` instance (see [Per-turn behavior](#per-turn-behavior)).

## Verifying the install

The status RPC is on the existing inference namespace:

```bash
openhuman-core rpc openhuman.inference_claude_code_status
```

Returns one of (`CliStatus` in [`tinyagents-harness`'s `claude_code/types.rs`](../../../vendor/tinyagents/crates/tinyagents-harness/src/providers/claude_code/types.rs)):

- `{"status":"ok","version":"2.0.4","path":"/usr/local/bin/claude"}`: ready
- `{"status":"not_installed"}`: no usable `claude` was found through the
  configured override, `PATH`, or the supported fallback locations
- `{"status":"outdated","version":"1.9.0","min_required":"2.0.0","path":"…"}`: bump CLI
- `{"status":"unusable","path":"…","reason":"…"}`: binary present but the version probe failed

Binary lookup uses this precedence: `OPENHUMAN_CLAUDE_CLI` first, then the
`PATH` search, then the ordered fallback locations. The fallback list covers
the native installer and common user-local, Bun, npm-global, and Homebrew
installations. When the CLI is found through a fallback, its directory and
user bin directories are prepended to the child process `PATH`, while the
inherited entries remain available.

The same status is rendered in the settings panel via `ClaudeCodeStatusCard` ([`app/src/components/settings/panels/ai/ClaudeCodeStatusCard.tsx`](../../../app/src/components/settings/panels/ai/ClaudeCodeStatusCard.tsx)).

## Per-turn behavior

`ClaudeCodeProvider::run_chat` acquires one of `MAX_CONCURRENT_TURNS` (4) semaphore permits, plus a per-thread mutex so two overlapping calls for the same conversation cannot race on the same session UUID. Each turn then:

1. Resolves a per-thread CC session UUID from `<workspace_dir>/claude-code-sessions.json`. New threads get a fresh RFC-4122 v4 UUID; the CLI requires v4 specifically for `--resume`.
2. Asks the host's `McpEndpointProvider` for an MCP endpoint. OpenHuman's implementation (`OpenHumanMcpEndpoint`) lazily starts one in-process HTTP MCP server per core (`crate::mcp::server::ensure_local_http`, loopback only) and returns its URL plus a bearer token; the config file passed via `--mcp-config` carries that token in its `Authorization` header. If starting the endpoint fails, the turn still proceeds, just without OpenHuman tools.
3. Spawns the CLI with:
   - `-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --add-dir <project_dir>`
   - `--mcp-config <scratch>/openhuman-mcp-config.json --strict-mcp-config` when the endpoint resolved, so only the configured MCP servers are visible
   - `--permission-mode acceptEdits` (default) or `bypassPermissions` (full access, see below)
   - by default, `--disallowedTools Bash,BashOutput,KillShell,WebFetch,WebSearch,Task`, so CC's own shell, network, and subagent-fanout builtins stay off and OpenHuman tools (`mcp__openhuman__*`) are the only way to reach those capabilities; omitted entirely when full access is on
   - `--session-id <uuid>` on first turn, `--resume <uuid>` thereafter
   - `--model <model>` (the suffix after `claude-code:`)
   - `--append-system-prompt-file <scratch>/append-system-prompt.txt` if the conversation carries a system message (a file, not an argv value, so a large harness prompt does not hit Windows' argv length limit)
4. Pipes stdin: full conversation history folded into a text preamble on a new session, just the pending user turn(s) on `--resume` (the CLI already holds its own prior-turn context server-side).
5. Streams stdout through the JSONL parser, then the event mapper, into `ProviderDelta`s on the request's `stream` sink.

On exit non-zero the driver bubbles stderr (capped at 16 KiB) up as the error message.

On macOS the spawn is wrapped in a Seatbelt jail (`sandbox-exec`) by default when `/usr/bin/sandbox-exec` exists; set `OPENHUMAN_CLAUDE_CODE_SANDBOX=0` to opt out. The profile blocks reads and writes under the first `.openhuman*`-named ancestor of `workspace_dir` (this provider's own session store and settings), not the CLI's own file tools. Linux and Windows have no OS-level wall yet.

### Permission posture and full access

The default posture, `acceptEdits`, restricts CC to file reads and edits under `project_dir` and withholds shell, network, and `Task` fan-out via `--disallowedTools`. A user can opt into full access (CC's entire toolset, including Bash) either through the settings toggle persisted in `<workspace_dir>/claude_code_settings.json`, or with `OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE=bypass|bypassPermissions|full`. This is an explicit user choice; enabling the Claude Code provider alone never grants shell or network access.

## Auth resolution order

1. `ANTHROPIC_API_KEY` env var (highest precedence, set on the spawned child).
2. A host-provided auth-profile store, or Claude Pro/Max OAuth: both still future work.
3. `~/.claude/.credentials.json`: the CLI's own login from `claude login` (Pro/Max subscription), used transparently by simply not setting `ANTHROPIC_API_KEY` on the child, so the CLI reads its own credentials (the Keychain, on macOS, under the `Claude Code-credentials` service).
4. None: the CLI will fail with an auth error.

The `openhuman.inference_claude_code_auth_status` RPC reports the richer state for the Settings → AI panel by spawning `claude auth status --json` (bounded by a 10-second timeout) rather than reading the credentials file directly, since a macOS login lives in the Keychain, not on disk.

## Tool surface exposed to the CLI

The CLI sees these tools as `mcp__openhuman__<name>`, served over the loopback HTTP MCP endpoint described above (the same tool set the stdio MCP server in [`crates/openhuman-core/src/mcp/server/`](../../../crates/openhuman-core/src/mcp/server/) exposes to other MCP clients):

- `core.list_tools`, `core.tool_instructions`
- `memory.recall`, `memory.fetch`, `memory.list`, `memory.learn`, `memory.forget`
- `agent.list_subagents`, `agent.run_subagent` (write, flagged `destructiveHint` per MCP spec)
- `searxng_search`

The MCP server enforces `SecurityPolicy::ToolOperation` checks; `agent.run_subagent`, `memory.learn` and `memory.forget` act (write); the rest are read-only. The CLI's own `tool_use` blocks (its internal Read/Bash/etc. calls) are never surfaced to OpenHuman as harness tool calls: the CLI has already executed them by the time they appear in the stream, so `event_mapper.rs` only strips their argument JSON out of the visible text.

## Limitations (v1)

- Vision input is forwarded as native image blocks when pasted images are available to the Claude Code provider. Images that cannot be read are sent as a short text notice.
- Every role routed to `claude-code:` shares the same `MAX_CONCURRENT_TURNS` semaphore; under load a CC turn waits in queue rather than failing fast.
- Cost accounting from the CLI's `result.total_cost_usd` is captured in the mapper but not yet wired into OpenHuman's billing layer ([`crates/openhuman-core/src/platform/cost/`](../../../crates/openhuman-core/src/platform/cost/)).
- Linux and Windows run the CLI unconfined; the Seatbelt jail is macOS-only.
