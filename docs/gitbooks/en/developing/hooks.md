---
description: >-
  File-based hooks that observe and gate an agent turn, with a
  Cursor-compatible contract.
icon: link
---

# Hooks

A hook is a script you own. OpenHuman runs it at a set moment, such as before a tool runs, after a file is edited or when a turn finishes, and the agent obeys its answer. Hooks let you enforce rules that live in your repository instead of in our code. You can block `rm -rf`, run the formatter after every edit, write an audit line per tool call, or refuse to read `.env`.

The contract matches [Cursor's](https://cursor.com/docs/hooks): the same file name, event names, stdin envelope, stdout decision and exit codes. A hook script written for either host runs on the other unchanged.

> "Hook" has a second, unrelated meaning in this codebase: the in-process Rust traits in `crates/openhuman-core/src/agent/hooks.rs`. An embedding host installs those by compiling against the core. This page is about the file-based kind.

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

OpenHuman reads four locations and concatenates them. A more specific file cannot remove a broader file's rules.

| Layer | Path |
| ----- | ---- |
| System | `/etc/openhuman/hooks.json` · `/Library/Application Support/OpenHuman/hooks.json` · `%ProgramData%\OpenHuman\hooks.json` |
| User | `~/.openhuman/hooks.json` |
| Workspace | `<workspace_dir>/hooks.json` |
| Project | `<action_dir>/.openhuman/hooks.json` |

Concatenation is safe because the strictest verdict wins. Across every hook that ran, deny beats ask, and ask beats allow. Adding a hook never loosens a policy that another hook set. A repository can therefore ship its own `hooks.json` onto a machine an operator has already locked down.

### Fields

| Field | Meaning |
| ----- | ------- |
| `command` | Program to run, or the prompt text for `"type": "prompt"`. It runs with the directory of its `hooks.json` as the working directory. |
| `type` | `command` (default) or `prompt`. |
| `matcher` | Which occurrences reach this hook. See Matchers below. Absent means all. |
| `timeout` | Seconds. Falls back to `[hooks] default_timeout_secs` (30). |
| `failClosed` | Treat a crashed, missing or timed-out hook as a denial. Default `false`. |
| `loop_limit` | Follow-ups this hook may inject per session. Default 5; `0` means unlimited. |
| `model` | Model override for a `prompt` hook. |
| `enabled` | Set `false` to park a hook without deleting it. |

## The protocol

The event arrives on stdin as one JSON object. The decision goes to stdout. The exit code decides how stdout is read:

| Exit | Meaning |
| ---- | ------- |
| `0` | stdout is the decision. Empty stdout is a no-op. |
| `2` | Deny, whatever stdout said. stderr becomes the reason the agent is told. |
| anything else | Failure. It fails open (the action proceeds) unless `failClosed` is set. |

A timeout, a missing interpreter and unparseable stdout all take the same failure path. A hook that denies only when it manages to run is not a security control, so `failClosed` covers every way a script can fail to answer.

stdout is parsed leniently. The last standalone JSON object wins, so a script that logs progress before answering works as written.

### Decision object

Every field is optional. Each event honors the subset it defines:

```json
{
  "permission": "allow" | "deny" | "ask",
  "user_message": "shown to the human",
  "agent_message": "shown to the model",
  "updated_input": { "...": "replacement tool arguments" },
  "additional_context": "appended to the tool result",
  "continue": false,
  "followup_message": "sent as another user turn",
  "env": { "KEY": "value" }
}
```

`ask` goes to the approval gate where one exists. Inside the tool middleware, which has no approval channel, it denies instead of quietly allowing.

## Events

`hook_event_name` in the envelope tells a script which moment it is in. Names match loosely: `preToolUse`, `PreToolUse` and `pre_tool_use` are the same event, and Claude Code's `UserPromptSubmit` maps to `beforeSubmitPrompt`.

| Event | Fires | Honors |
| ----- | ----- | ------- |
| `preToolUse` | before any tool | `permission`, `updated_input`, `agent_message` |
| `postToolUse` | after a tool succeeded | `additional_context` |
| `postToolUseFailure` | after a tool failed | - |
| `beforeShellExecution` | before `shell` | `permission`, `agent_message` |
| `afterShellExecution` | after one completed | - |
| `beforeReadFile` | before `file_read` / `read_diff` | `permission` |
| `afterFileEdit` | after `file_write` / `edit` / `apply_patch` | - |
| `beforeMCPExecution` / `afterMCPExecution` | around an MCP tool | `permission` |
| `beforeSubmitPrompt` | on a chat message, before the model | `continue`, `permission`, `additional_context` |
| `subagentStart` | before a delegation | `permission` |
| `subagentStop` | after one (not fired yet, see below) | `followup_message` |
| `stop` | after a turn | `followup_message` |
| `afterAgentResponse` | on the assistant's message | - |

`sessionStart`, `sessionEnd`, `preCompact`, `afterAgentThought` and `subagentStop` are defined. They parse, match and execute, and you can exercise them with `hooks test`, but the core does not fire them yet. Configuring one produces a load warning, and `hooks list` reports `"wired": false` for it. A hook that silently never runs would be worse, so the warning is deliberate.

### Derived events

OpenHuman has no separate "shell execution" or "file read" call site. Those are the `shell`, `file_read` and `file_write` tools going through the ordinary tool path. The shell, file and MCP events are therefore derived from tool calls, and their payloads are reshaped the way a Cursor hook expects: a `command` string, a `file_path`, an `edits` array. The generic `preToolUse` fires first, then the specialized event.

## Matchers

A matcher is one string, matched against a subject the event chooses. That is the tool name for tool events, the command line for shell events, the path for file events and the agent id for subagent events.

* absent or `*`: everything
* `Shell`: a literal, case-insensitive name
* `Read|Write|Shell`: alternation
* `MCP:search_docs`: an MCP tool by name
* anything containing punctuation: a regular expression (`^rm\b`, `\.rs$`)

An invalid regex matches nothing and is logged.

## Latency

Gating events run their hooks one after another and the turn waits. A denial skips the rest. Observational events (`afterShellExecution`, `postToolUseFailure`, `afterAgentResponse` and others) run on a background task and the turn never waits, so a hung audit hook cannot hang the agent.

When nothing is configured, the harness bridge is not installed, so an unconfigured host pays nothing per tool call.

## Environment

Hook processes inherit the core's environment plus:

`OPENHUMAN_PROJECT_DIR` (also exported as `CLAUDE_PROJECT_DIR` and
`CURSOR_PROJECT_DIR`), `OPENHUMAN_VERSION`, `OPENHUMAN_HOOK_EVENT`,
`OPENHUMAN_SESSION_ID`, `OPENHUMAN_AGENT_ID`.

## Prompt hooks

With `"type": "prompt"` you write the policy in English instead of shell. The text goes to a model with the event JSON substituted for `$ARGUMENTS`, and the model answers `{"ok": true}` or `{"ok": false, "reason": "..."}`.

```json
{ "command": "Deny if $ARGUMENTS deletes anything outside /tmp.", "type": "prompt" }
```

Each event costs a model call, so use prompt hooks for rare, high-stakes moments, not every tool call.

## Inspecting and debugging

Three commands cover it:

```bash
openhuman-core hooks list      # what is configured, from which file, and whether it is wired
openhuman-core hooks reload    # re-read every layer
openhuman-core hooks test --event beforeShellExecution \
  --payload '{"command":"rm -rf /","sandbox":false}'
```

Over JSON-RPC they are `openhuman.hooks_list`, `openhuman.hooks_reload` and `openhuman.hooks_test`.

`hooks test` fires one synthetic event in the foreground and reports what each matching hook decided. That includes hooks on observational events, which a real dispatch would run in the background. Use it to debug a hook instead of asking the agent to do the dangerous thing to see whether the rule fires.

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
cat > /dev/null            # drain stdin, this hook ignores the event
cargo fmt >/dev/null 2>&1
echo '{}'
```

Both need `chmod +x`.

## Implementation

The engine and the whole `hooks.json` contract live upstream in `tinyagents_runtime::command_hooks`. That covers the types, the file and its layering, the matcher, running one hook (stdin, timeout, exit codes), selection, ordering and aggregation, the context envelope, and follow-ups.

`crates/openhuman-core/src/hooks/` holds the host half: `bridge` (mounts the engine on the harness's tool and turn paths), `ops` (moments with no existing path), `host`, `schemas` (the RPC surface) and `prompt_eval`. The `README.md` beside the code is the authoritative split.
