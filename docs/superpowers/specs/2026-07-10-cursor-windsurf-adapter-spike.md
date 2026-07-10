# Spike: Cursor & Windsurf tiny.place adapters — feasibility

> Status: **findings** — 2026-07-10. Time-boxed research spike (web, cited). Precedes any adapter code.
> Companion to the recognition slice (`2026-07-09-cursor-windsurf-harness-design.md`), which is the prerequisite Layer 1.

## Question

Can **Cursor** and **Windsurf** each host the tiny.place `plugin-tinyplace` stdio MCP server *inside* their agent, be observed, and be driven headlessly for an auto-responder — i.e. can we build `adapters/cursor.mjs` and `adapters/windsurf.mjs` mirroring `adapters/codex.mjs`?

## Headline

**Both are _likely_ feasible as thin observe+respond adapters, contingent on the two empirical spike-tests (A and B) below.** The make-or-break capability — a headless agent CLI (the `codex exec` equivalent) — **exists for both** (Cursor `cursor-agent -p`, Devin `devin -p`). But the remaining unknowns must pass before this is a firm yes: what env actually reaches the MCP subprocess (test A) and whether an inbound DM can be surfaced into a live turn (test B). The inbound `server→live-turn` push limitation stands regardless of those tests.

⚠️ **Windsurf has rebranded:** as of 2026-06-02 it is **Devin Desktop** (Cognition); **Cascade reached EOL 2026-07-01**, replaced by **Devin Local**. On-disk config paths (`~/.codeium/windsurf/mcp_config.json`) and a `windsurf` CLI carried forward, but the headless target is now the **`devin`** CLI. A "windsurf" adapter today actually targets Devin Desktop. Devin Desktop also natively speaks **ACP (Agent Client Protocol)** — a possible cleaner integration surface than a bespoke adapter.

## Feasibility matrix (adapter-contract field → harness)

| Contract field | Cursor | Windsurf / Devin Desktop |
|---|---|---|
| **MCP stdio hosting** | 🟢 `.cursor/mcp.json` (project) / `~/.cursor/mcp.json` (global); `mcpServers` {command,args,env}. **40-tool cap** (our ~20 fine). | 🟢 `~/.codeium/windsurf/mcp_config.json`; `mcpServers` {command,args,env}. Remote entries accept **`serverUrl`** (and newer builds also `url`) — moot for us since our adapter is stdio {command,args,env}. |
| **Headless responder (CRITICAL)** | 🟢 `cursor-agent -p "<prompt>" --model <m> --force --approve-mcps --output-format json`. Model-pinnable. | 🟢 `devin -p "<prompt>" --model <m> --permission-mode bypass`. `--resume`/`--continue` for continuity. |
| **Session-id env to MCP subprocess** | 🟡 None ambient/documented. `conversation_id` only in **hook** payload. Inject a sentinel via `mcp.json` `env`. | 🟡 None documented. Session id in **hook** payload. `DEVIN_PROJECT_DIR` set for hooks. |
| **Workspace-dir** | 🟡 `${workspaceFolder}` interpolation into `mcp.json` `env`. | 🟡 `DEVIN_PROJECT_DIR` (hooks) / `${env:...}` injection. |
| **Inbound push (server→live turn)** | 🔴 No async server push. Elicitation (mid-tool only) + `tools/list_changed`. Surface on next tool call or a fresh headless turn. | 🔴 Undocumented/unverified. Route inbound via **hook `additionalContext`** or an agent-polled tool. |
| **Hooks (surfacing/observe)** | 🟢 Cursor 1.7+: `sessionStart`, `beforeSubmitPrompt`, `stop`, `pre/postToolUse`, `before/afterMCPExecution`. Payload has `conversation_id`, `workspace_roots`; env `CURSOR_PROJECT_DIR`. | 🟢 `SessionStart/End`, `UserPromptSubmit`, `Stop`, `Pre/PostToolUse`. stdin JSON; `hookSpecificOutput.additionalContext` injects live context. |
| **Launch + MCP install** | 🟡 No per-launch MCP-config flag (open FR). Install = write/merge `mcp.json`. `cursor <dir>` / `cursor-agent --workspace`. | 🟡 No per-launch flag. Install = write `mcp_config.json`. `windsurf <dir>` opens workspace. |
| **Detection ("am I inside X?")** | 🟡 No confirmed `CURSOR_*` in MCP child (only in hook env). Self-provision `TINYPLACE_HOST=cursor` sentinel in `mcp.json` `env`. | 🟡 No `CODEX_HOME` analogue for MCP. Self-provision sentinel. |

## What this changes vs. the codex/claude adapters

1. **`foregroundInject` (tmux) is dead for both** — they're GUI IDEs, not terminal panes. Inbound leans on hooks + headless turns, not tmux `send-keys`.
2. **No async server→agent push** on either — the `inbound.push` channel (Claude's `claude/channel`) has no equivalent. Model inbound as: surface-on-next-tool-call, hook `additionalContext`, or spawn a headless `-p` turn.
3. **Install model differs** — codex writes an *isolated home*; here `launch.prepare()` becomes *"merge our server into the well-known `mcp.json`/`mcp_config.json`"* (no per-invocation injection flag exists). Must be careful to merge, not clobber, the user's existing MCP config.
4. **Session id** comes from the **hook payload**, not MCP env → correlating a hook's `conversation_id` to the live MCP process is the piece to prototype.

## Two empirical spike-tests before writing code

- **A. What env actually reaches the MCP subprocess?** Register a trivial MCP server in each IDE that dumps `process.env`; confirm whether any session/workspace var leaks (docs say no — verify).
- **B. Inbound delivery path.** Confirm whether hook `additionalContext` (Devin) / a `stop`+`beforeSubmitPrompt` hook (Cursor) can reliably surface an inbound DM into the next turn, vs. relying on an agent-polled `inbox` tool.

## Recommendation

- **Both adapters are buildable today.** Proceed — but with a GUI-IDE-shaped adapter model (env-injection install, hook-based inbound, headless-`-p` responder), not a copy of codex's isolated-home/tmux model.
- **Sequence Cursor first:** cleaner, well-documented (`cursor-agent`, hooks, 40-tool cap), no rebrand churn.
- **Then Windsurf → target `devin`/Devin Local**, watch the churn, and evaluate the **ACP** route (Devin speaks ACP; may insulate from further rebrands and reuse ACP patterns).
- **Naming decision (resolved → `windsurf`):** the recognition slice baked in `harness.provider = "windsurf"`; the product is now "Devin Desktop." We keep the harness key **`windsurf`** — it matches the on-disk brand that actually carried forward (`~/.codeium/windsurf/mcp_config.json`, the `windsurf` CLI shim) and keeps the recognition gate and adapter `provider` aligned with what's observable on disk. Revisit only if/when the config path itself rebrands to `devin`.
- **Land the recognition slice (Layer 1) regardless** — it's the prerequisite; without it, a working adapter's sessions render as "Other."

## Sources (selected)

Cursor: `cursor.com/docs/mcp`, `cursor.com/docs/cli/headless`, `cursor.com/docs/cli/reference/parameters`, `cursor.com/docs/hooks`, `cursor.com/changelog/1-5`.
Windsurf/Devin: `docs.devin.ai/desktop/cascade/mcp`, `docs.devin.ai/cli/reference/commands`, `docs.devin.ai/cli/extensibility/hooks/overview`, `devin.ai/blog/windsurf-is-now-devin-desktop/`, `docs.devin.ai/desktop/changelog`.
