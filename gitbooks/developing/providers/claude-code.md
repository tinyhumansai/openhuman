# Claude Code CLI provider

OpenHuman can route any chat workload through **Anthropic's `claude` CLI** instead of calling the Anthropic HTTP API directly. The CLI handles model selection, auth, and prompt-cache management; OpenHuman drives it as a child process per turn, parses its stream-json output, and re-exposes its own read-only tools back into the CLI over MCP so the model can reach native OpenHuman state (memory, threads, channels, people).

> Locked decisions live in [`.planning/claude-code-provider/PLAN.md`](../../../.planning/claude-code-provider/PLAN.md) §13.

## Requirements

- Claude Code CLI **≥ 2.0.0** on `PATH` (or `OPENHUMAN_CLAUDE_CLI=/abs/path/to/claude`).
- An Anthropic API key in `ANTHROPIC_API_KEY`, **or** a pre-existing `~/.claude/.credentials.json` from `claude login`.
- The `openhuman-core` binary on disk — OpenHuman spawns `openhuman-core mcp` as a stdio MCP server so the CLI can call OpenHuman tools. The path is discovered via `std::env::current_exe()`.

## Routing a workload through the CLI

The factory grammar accepts a new prefix: `claude-code:<model>[@<temperature>]`. Apply it via the standard inference settings (per-role, locked decision #3):

```bash
# Through the JSON-RPC update endpoint:
openhuman-core rpc openhuman.inference_update_model_settings \
  --json '{"chat_provider":"claude-code:claude-sonnet-4-5"}'
```

| Role string | Field updated |
| --- | --- |
| `chat_provider`       | foreground chat replies                  |
| `reasoning_provider`  | long-context reasoning workloads         |
| `agentic_provider`    | multi-step agentic loops                 |

A workload set to `claude-code:<model>` always spawns a fresh `claude` child per turn; concurrency is capped at `MAX_CONCURRENT_TURNS = 4` per `ClaudeCodeProvider` instance.

## Verifying the install

The status RPC is on the existing inference namespace:

```bash
openhuman-core rpc openhuman.inference_claude_code_status
```

Returns one of (`CliStatus` in [`src/openhuman/inference/provider/claude_code/types.rs`](../../../src/openhuman/inference/provider/claude_code/types.rs)):

- `{"status":"ok","version":"2.0.4","path":"/usr/local/bin/claude"}` — ready
- `{"status":"not_installed"}` — `claude` not on `PATH`
- `{"status":"outdated","version":"1.9.0","min_required":"2.0.0","path":"…"}` — bump CLI
- `{"status":"unusable","path":"…","reason":"…"}` — binary present but the version probe failed

The same status is rendered in the settings panel via `ClaudeCodeStatusCard` ([`app/src/components/settings/panels/ai/ClaudeCodeStatusCard.tsx`](../../../app/src/components/settings/panels/ai/ClaudeCodeStatusCard.tsx)).

## Per-turn behavior

Each chat turn:

1. Resolve a per-thread CC session UUID from `<workspace>/claude-code-sessions.json`. New threads get a fresh RFC-4122 v4 UUID; the CLI requires v4 specifically for `--resume`.
2. Write `mcp-config.json` to a tempdir pointing at `openhuman-core mcp` (stdio MCP server, no extra credentials).
3. Spawn the CLI with:
   - `-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages`
   - `--mcp-config <tmp> --strict-mcp-config` so only the configured MCP servers are visible
   - `--disallowedTools Bash,Read,Write,Edit,Glob,Grep,WebFetch,WebSearch,TodoWrite,Task,BashOutput,KillShell` — CC's own builtins stay off so OpenHuman tools (`mcp__openhuman__*`) are authoritative
   - `--session-id <uuid>` on first turn, `--resume <uuid>` thereafter
   - `--model <model>` (the suffix after `claude-code:`)
   - `--append-system-prompt <…>` if the conversation carries a system message
4. Pipe stdin: full conversation history on a new session, just the last user turn on `--resume` (the CLI already holds its own prior-turn context server-side).
5. Stream stdout through the JSONL parser → event mapper → `ProviderDelta`s on the request's `stream` sink.

On exit non-zero the driver bubbles stderr (capped at 16 KiB) up as the error message.

## Auth resolution order

1. `ANTHROPIC_API_KEY` env var (highest precedence — set on the spawned child).
2. Per-thread / per-agent key from `ChatRequest` config (future, not yet wired).
3. `~/.claude/.credentials.json` — the CLI's own OAuth tokens from `claude login` (Pro / Max subscription). We never read or round-trip the access token; auth detection probes this file for non-secret metadata only.
4. None — the CLI will fail with an auth error.

The `openhuman.inference_claude_code_auth_status` RPC probes sources 1 and 3 without spawning the CLI and surfaces the result in the Settings → AI panel.

## Tool surface exposed to the CLI

The CLI sees these tools as `mcp__openhuman__<name>` (delivered by the existing stdio MCP server in [`src/openhuman/mcp_server/`](../../../src/openhuman/mcp_server/)):

- `core.list_tools`, `core.tool_instructions`
- `memory.search`, `memory.recall`
- `tree.read_chunk`, `tree.browse`, `tree.top_entities`, `tree.list_sources`
- `agent.list_subagents`, `agent.run_subagent` (write — flagged `destructiveHint` per MCP spec)
- `searxng_search`

The MCP server enforces `SecurityPolicy::ToolOperation` checks; all tools except `agent.run_subagent` are read-only.

## Limitations (v1)

- Vision input is not forwarded — set the `vision_provider` to a different provider when you need images.
- `agentic` runs share the same `Semaphore(4)`; under load a CC turn waits in queue rather than failing fast.
- Cost accounting from the CLI's `result.total_cost_usd` is captured in the mapper but not yet wired into OpenHuman's billing layer ([`src/openhuman/cost/`](../../../src/openhuman/cost/)).
