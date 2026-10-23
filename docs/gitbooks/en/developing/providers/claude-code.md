---
description: >-
  Route chat workloads through the Claude Code CLI.
icon: terminal
---

# Claude Code CLI provider

OpenHuman can send any chat workload through Anthropic's `claude` CLI instead of calling the Anthropic HTTP API directly. The CLI handles model selection, auth and prompt caching. OpenHuman runs it as a long-lived child process that resumes sessions, parses its stream-json output, and gives it an MCP endpoint so the model can reach OpenHuman state (memory, threads, agents, search). This is one of several pluggable LLM backends. See [Engines](../engines.md) for the full list.

`tinyagents-harness` (vendor/tinyagents) owns the provider, `ClaudeCodeProvider`. See [its README](https://github.com/tinyhumansai/tinyagents/blob/main/crates/tinyagents-harness/src/providers/claude_code/README.md) for the file map and implementation notes. OpenHuman only wires it up in `crates/openhuman-core/src/inference/provider/factory/subprocess_providers.rs`, which supplies the MCP endpoint (`OpenHumanMcpEndpoint`, below) and reads the resulting model string back into its own routing.

## Requirements

- Claude Code CLI 2.0.0 or newer (`MIN_CLI_VERSION`). It can be on `PATH`, in a supported well-known install location, or selected with `OPENHUMAN_CLAUDE_CLI=/abs/path/to/claude`.
- An Anthropic API key in `ANTHROPIC_API_KEY`, or an existing `~/.claude/.credentials.json` from `claude login`.
- The `http-server` Cargo feature (on by default). OpenHuman exposes its MCP tools to the CLI over a loopback HTTP endpoint (`crate::mcp::server::ensure_local_http`), not by running `openhuman-core mcp` as a subprocess. Without that feature the provider still runs, but the CLI gets no OpenHuman tools.

## Routing a workload through the CLI

Use the provider prefix `claude-code:<model>[@<temperature>]`. Set it per role through the standard inference settings:

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

A workload set to `claude-code:<model>` starts a fresh `claude` child for every turn. Concurrency is capped at `MAX_CONCURRENT_TURNS = 4` per `ClaudeCodeProvider` instance (see [Per-turn behavior](#per-turn-behavior)).

## Verifying the install

The status RPC is in the inference namespace:

```bash
openhuman-core rpc openhuman.inference_claude_code_status
```

It returns one of these (`CliStatus` in [`tinyagents-harness`'s `claude_code/types.rs`](https://github.com/tinyhumansai/tinyagents/blob/main/crates/tinyagents-harness/src/providers/claude_code/types.rs)):

- `{"status":"ok","version":"2.0.4","path":"/usr/local/bin/claude"}`: ready
- `{"status":"not_installed"}`: no usable `claude` was found through the
  configured override, `PATH`, or the supported fallback locations
- `{"status":"outdated","version":"1.9.0","min_required":"2.0.0","path":"..."}`: upgrade the CLI
- `{"status":"unusable","path":"...","reason":"..."}`: binary present but the version probe failed

Binary lookup checks `OPENHUMAN_CLAUDE_CLI` first, then `PATH`, then the ordered fallback locations. The fallbacks cover the native installer and common user-local, Bun, npm-global and Homebrew installs. When the CLI is found through a fallback, its directory and the user bin directories are prepended to the child process `PATH`. The inherited entries stay available.

The settings panel shows the same status through `ClaudeCodeStatusCard` ([`app/src/components/settings/panels/ai/ClaudeCodeStatusCard.tsx`](https://github.com/tinyhumansai/openhuman/blob/main/app/src/components/settings/panels/ai/ClaudeCodeStatusCard.tsx)).

## Per-turn behavior

`ClaudeCodeProvider::run_chat` takes one of `MAX_CONCURRENT_TURNS` (4) semaphore permits, plus a per-thread mutex so two overlapping calls for the same conversation cannot race on one session UUID. Each turn then:

1. Resolves a per-thread session UUID from `<workspace_dir>/claude-code-sessions.json`. New threads get a fresh RFC 4122 v4 UUID, because the CLI requires v4 for `--resume`.
2. Asks the host's `McpEndpointProvider` for an MCP endpoint. OpenHuman's implementation (`OpenHumanMcpEndpoint`) lazily starts one in-process HTTP MCP server per core (`crate::mcp::server::ensure_local_http`, loopback only) and returns its URL plus a bearer token. The config file passed with `--mcp-config` carries that token in its `Authorization` header. If the endpoint fails to start, the turn still runs, without OpenHuman tools.
3. Spawns the CLI with:
   - `-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --add-dir <project_dir>`
   - `--mcp-config <scratch>/openhuman-mcp-config.json --strict-mcp-config` when the endpoint resolved, so only the configured MCP servers are visible
   - `--permission-mode acceptEdits` (default) or `bypassPermissions` (full access, described below)
   - by default, `--disallowedTools Bash,BashOutput,KillShell,WebFetch,WebSearch,Task`, so Claude Code's own shell, network and sub-agent builtins stay off and OpenHuman tools (`mcp__openhuman__*`) are the only way to reach those capabilities. This flag is omitted when full access is on
   - `--session-id <uuid>` on the first turn, `--resume <uuid>` after that
   - `--model <model>` (the suffix after `claude-code:`)
   - `--append-system-prompt-file <scratch>/append-system-prompt.txt` if the conversation carries a system message. It is a file, not an argument, so a large harness prompt does not hit Windows' argument length limit
4. Pipes stdin. A new session gets the full conversation history folded into a text preamble. A `--resume` gets only the pending user turns, because the CLI already holds the earlier context.
5. Streams stdout through the JSONL parser, then the event mapper, into `ProviderDelta`s on the request's `stream` sink.

If the CLI exits non-zero, the driver returns its stderr (capped at 16 KiB) as the error message.

On macOS the spawn runs inside a Seatbelt jail (`sandbox-exec`) by default when `/usr/bin/sandbox-exec` exists. Set `OPENHUMAN_CLAUDE_CODE_SANDBOX=0` to opt out. The profile blocks reads and writes under the first `.openhuman*`-named ancestor of `workspace_dir`, which holds this provider's own session store and settings. It does not restrict the CLI's own file tools. Linux and Windows have no OS-level wall yet.

### Permission posture and full access

The default, `acceptEdits`, limits Claude Code to file reads and edits under `project_dir` and withholds shell, network and `Task` access through `--disallowedTools`. You can opt into full access (the whole Claude Code toolset, including Bash) with the settings toggle saved in `<workspace_dir>/claude_code_settings.json`, or with `OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE=bypass|bypassPermissions|full`. This is an explicit choice. Enabling the provider alone never grants shell or network access.

## Auth resolution order

1. `ANTHROPIC_API_KEY` env var (highest precedence, set on the spawned child).
2. `~/.claude/.credentials.json`: the CLI's own login from `claude login` (Pro or Max subscription). OpenHuman leaves `ANTHROPIC_API_KEY` unset on the child, so the CLI reads its own credentials. On macOS they live in the Keychain under the `Claude Code-credentials` service.
3. Neither: the CLI fails with an auth error.

The `openhuman.inference_claude_code_auth_status` RPC reports the richer state for the Settings > AI panel. It runs `claude auth status --json` with a 10-second timeout instead of reading the credentials file, because a macOS login lives in the Keychain, not on disk.

## Tool surface exposed to the CLI

The CLI sees these tools as `mcp__openhuman__<name>`, served over the loopback HTTP MCP endpoint described above. It is the same tool set that the stdio MCP server in [`crates/openhuman-core/src/mcp/server/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-core/src/mcp/server) exposes to other MCP clients:

- `core.list_tools`, `core.tool_instructions`
- `memory.recall`, `memory.fetch`, `memory.list`, `memory.learn`, `memory.forget`
- `agent.list_subagents`, `agent.run_subagent` (write, flagged `destructiveHint` as the MCP spec requires)
- `searxng_search`

The MCP server enforces the security policy per call: `agent.run_subagent`, `memory.learn` and `memory.forget` write, so each reserves one action of the hourly budget and is refused under a read-only autonomy tier (`SecurityPolicy::spend_action`, answered by the security module). The rest are read-only. The CLI's own `tool_use` blocks (its internal Read, Bash and similar calls) never reach OpenHuman as harness tool calls, because the CLI has already run them by the time they appear in the stream. `event_mapper.rs` only strips their argument JSON from the visible text.

## Limitations

- Vision input is forwarded as native image blocks when pasted images are available to the Claude Code provider. Images that cannot be read are sent as a short text notice.
- Every role routed to `claude-code:` shares the same `MAX_CONCURRENT_TURNS` semaphore; under load a turn waits in the queue instead of failing fast.
- Cost accounting from the CLI's `result.total_cost_usd` is captured in the mapper but not yet wired into OpenHuman's cost tracking ([`crates/openhuman-core/src/platform/cost/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-core/src/platform/cost)).
- Linux and Windows run the CLI unconfined; the Seatbelt jail is macOS-only.
