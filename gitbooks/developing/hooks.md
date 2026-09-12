# Hooks

A hook is a script you own that OpenHuman runs at a specific moment — before a
tool executes, after a file is edited, when a turn finishes — and whose answer
the agent obeys. It is how you make the agent follow a rule that lives in your
repository rather than in our code: block `rm -rf`, run the formatter after every
edit, write an audit line per tool call, refuse to read `.env`.

The contract is deliberately the same as
[Cursor's](https://cursor.com/docs/hooks): same file name, same event names,
same stdin envelope, same stdout decision, same exit codes. A hook script written
for either host runs on the other unchanged.

> There is a second, unrelated meaning of "hook" in this codebase: the in-process
> Rust traits in `src/openhuman/agent/hooks.rs` that an *embedding host* installs
> by compiling against the core. Those are for building a product on top of
> OpenHuman. This page is about the file-based kind, for using OpenHuman.

## The file

`hooks.json`, schema version 1:

```json
{
  "version": 1,
  "hooks": {
    "beforeShellExecution": [
      {
        "command": "./.openhuman/deny-destructive.sh",
        "matcher": "^\\s*(rm|dd|mkfs)\\b",
        "timeout": 5,
        "failClosed": true
      }
    ],
    "afterFileEdit": [
      { "command": "./.openhuman/format.sh", "matcher": "\\.rs$" }
    ]
  }
}
```

Four locations are read, and **they concatenate — a more specific file cannot
remove a broader one's rules**:

| Layer | Path |
| ----- | ---- |
| System | `/etc/openhuman/hooks.json` · `/Library/Application Support/OpenHuman/hooks.json` · `%ProgramData%\OpenHuman\hooks.json` |
| User | `~/.openhuman/hooks.json` |
| Workspace | `<workspace_dir>/hooks.json` |
| Project | `<action_dir>/.openhuman/hooks.json` |

Concatenation is safe because the **strictest verdict wins**: across every hook
that ran, deny beats ask beats allow. Adding a hook can never loosen a policy
another one set, which is what lets a repository ship its own `hooks.json` onto
a machine an operator has already locked down.

### Fields

| Field | Meaning |
| ----- | ------- |
| `command` | Program to run (or the prompt text, for `"type": "prompt"`). Runs with its own `hooks.json` directory as cwd. |
| `type` | `command` (default) or `prompt`. |
| `matcher` | Which occurrences reach this hook — see below. Absent means all. |
| `timeout` | Seconds. Falls back to `[hooks] default_timeout_secs` (30). |
| `failClosed` | Treat a crashed, missing, or timed-out hook as a denial. Default `false`. |
| `loop_limit` | Follow-ups this hook may inject per session. Default 5; `0` means unlimited. |
| `model` | Model override for a `prompt` hook. |
| `enabled` | Set `false` to park a hook without deleting it. |

## The protocol

The event arrives on **stdin** as one JSON object. The decision goes to
**stdout**. The exit code decides how stdout is read:

| Exit | Meaning |
| ---- | ------- |
| `0` | stdout is the decision. Empty stdout is a no-op. |
| `2` | Deny, whatever stdout said. stderr becomes the reason the agent is told. |
| anything else | Failure. **Fails open** — the action proceeds — unless `failClosed`. |

A timeout, a missing interpreter, and unparseable stdout all take that same
failure path. That symmetry is the point: a hook that denies only when it
manages to run is not a security control, so `failClosed` covers every way a
script can fail to answer.

stdout is parsed leniently — the last standalone JSON object wins — so a script
that logs progress before answering works as written.

### Decision object

Every field is optional, and each event honours the subset it defines:

```json
{
  "permission": "allow" | "deny" | "ask",
  "user_message": "shown to the human",
  "agent_message": "shown to the model",
  "updated_input": { "…": "replacement tool arguments" },
  "additional_context": "appended to the tool result",
  "continue": false,
  "followup_message": "sent as another user turn",
  "env": { "KEY": "value" }
}
```

`ask` escalates to the approval gate where one is available; inside the tool
middleware, which has no approval channel, it denies rather than quietly
allowing.

## Events

`hook_event_name` in the envelope tells a script which moment it is in. Names are
matched loosely — `preToolUse`, `PreToolUse` and `pre_tool_use` are the same
event, and Claude Code's `UserPromptSubmit` aliases onto `beforeSubmitPrompt`.

| Event | Fires | Honours |
| ----- | ----- | ------- |
| `preToolUse` | before any tool | `permission`, `updated_input`, `agent_message` |
| `postToolUse` | after a tool succeeded | `additional_context` |
| `postToolUseFailure` | after a tool failed | — |
| `beforeShellExecution` | before `shell` / `node_exec` / … | `permission`, `agent_message` |
| `afterShellExecution` | after one completed | — |
| `beforeReadFile` | before `file_read` / `read_diff` | `permission` |
| `afterFileEdit` | after `file_write` / `edit` / `apply_patch` | — |
| `beforeMCPExecution` / `afterMCPExecution` | around an MCP tool | `permission` |
| `beforeSubmitPrompt` | on a chat message, before the model | `continue`, `permission`, `additional_context` |
| `subagentStart` | before a delegation | `permission` |
| `subagentStop` | after one — not fired yet, see below | `followup_message` |
| `stop` | after a turn | `followup_message` |
| `afterAgentResponse` | on the assistant's message | — |

`sessionStart`, `sessionEnd`, `preCompact`, `afterAgentThought` and `subagentStop`
are defined — they parse, match, execute, and can be exercised with `hooks test` —
but the core does not fire them yet. (`subagentStop` is the near miss: the handler
`hooks::ops::subagent_stopped` is complete, but `subagent_runner` fires only
`subagentStart` and returns without ever calling the stop side.) Configuring one
produces a load warning saying so, and `hooks list` reports `"wired": false` for
it. That is deliberate: a hook that silently never runs is the worst thing this
system can do to you.

### Derived events

OpenHuman has no separate "shell execution" or "file read" call site — those are
the `shell`, `file_read` and `file_write` tools going through the ordinary tool
seam. So the shell, file and MCP events are *derived* from tool calls, and their
payloads are reshaped the way a Cursor hook expects: a `command` string, a
`file_path`, an `edits` array. Both the generic `preToolUse` and the specialised
event fire, generic first.

## Matchers

One string, matched against a subject the event chooses: the tool name for tool
events, the command line for shell events, the path for file events, the agent id
for subagent events.

* absent or `*` — everything
* `Shell` — a literal, case-insensitive name
* `Read|Write|Shell` — alternation
* `MCP:search_docs` — an MCP tool by name
* anything containing punctuation — a regular expression (`^rm\b`, `\.rs$`)

An invalid regex matches **nothing** and logs.

## Latency

Gating events run their hooks sequentially and the turn waits; a denial
short-circuits the rest. Observational events (`afterShellExecution`,
`postToolUseFailure`, `afterAgentResponse`, …) are dispatched onto a background
task and the turn never waits — an audit hook that hangs must not hang the agent.

When nothing is configured, the harness bridge is not installed at all, so an
unconfigured host pays nothing per tool call.

## Environment

Hook processes inherit the core's environment plus:

`OPENHUMAN_PROJECT_DIR` (also exported as `CLAUDE_PROJECT_DIR` and
`CURSOR_PROJECT_DIR`), `OPENHUMAN_VERSION`, `OPENHUMAN_HOOK_EVENT`,
`OPENHUMAN_SESSION_ID`, `OPENHUMAN_AGENT_ID`.

## Prompt hooks

`"type": "prompt"` writes the policy in English instead of shell. The text is
sent to a model with the event JSON substituted for `$ARGUMENTS`, and the model
answers `{"ok": true}` or `{"ok": false, "reason": "…"}`.

```json
{ "command": "Deny if $ARGUMENTS deletes anything outside /tmp.", "type": "prompt" }
```

It costs a model call per event, so put it on rare, high-stakes moments — not on
every tool call.

## Inspecting and debugging

Three RPC methods, on the `hooks` namespace:

```bash
openhuman hooks list      # what is configured, from which file, and whether it is wired
openhuman hooks reload    # re-read every layer
openhuman hooks test --event beforeShellExecution \
  --payload '{"command":"rm -rf /","sandbox":false}'
```

Over JSON-RPC the same three are `openhuman.hooks_list`, `openhuman.hooks_reload`
and `openhuman.hooks_test`.

`hooks test` fires one synthetic event in the foreground and reports what each
matching hook decided, including hooks for observational events that a real
dispatch would run detached. Debug a hook with it rather than by asking the agent
to do the dangerous thing to see whether the rule fires.

## Host switches

`config.toml`:

```toml
[hooks]
enabled = true            # off means no hooks.json is read and no bridge installed
default_timeout_secs = 30 # for hooks that name no timeout of their own
```

## Example

`.openhuman/hooks.json`:

```json
{
  "version": 1,
  "hooks": {
    "beforeReadFile": [{ "command": "./.openhuman/no-secrets.sh", "matcher": "\\.env" }],
    "afterFileEdit": [{ "command": "./.openhuman/fmt.sh", "matcher": "\\.rs$" }]
  }
}
```

`.openhuman/no-secrets.sh`:

```sh
#!/bin/sh
echo '{"permission":"deny","agent_message":"Secrets files are off limits. Ask the user for the value you need."}'
```

`.openhuman/fmt.sh`:

```sh
#!/bin/sh
cat > /dev/null            # drain stdin; this hook does not read the event
cargo fmt >/dev/null 2>&1
echo '{}'
```

Both need `chmod +x`.

## Implementation

`src/openhuman/hooks/` — `types` (the wire contract), `config` (the file and its
layering), `matcher`, `exec` (one hook: stdin, timeout, exit codes),
`engine` (selection, ordering, aggregation), `context` (the envelope),
`bridge` (mounting on the harness's existing tool and turn seams), `ops` (the
moments with no existing seam), `followup`.
