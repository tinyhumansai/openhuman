# hooks

User-authored hooks: small scripts (or English-language prompts judged by a
model) that observe or gate the agent at specific moments. They are discovered
from `hooks.json` files at runtime, so nobody has to compile against the core
to add one. The file format is Cursor's `hooks.json`
(<https://cursor.com/docs/hooks>): same event names, same stdin envelope, same
stdout decision object, same exit-code semantics. A script written for Cursor
runs here unchanged, and the reverse holds too.

This folder is the OpenHuman host side only. The engine, the `hooks.json`
contract, config loading, matching and the process runner live upstream in
`tinyagents_runtime::command_hooks` ([`vendor/tinyagents`](../../../../vendor/tinyagents/), crate
`tinyagents-runtime`). What stays here is the glue: one process-global engine
built with OpenHuman's product name, shell and model, a bridge that mounts it
on the harness's tool and turn seams, direct entry points for the moments that
have no seam, and the `hooks` RPC namespace.

"Hook" means two other things elsewhere in the codebase. The in-process Rust
traits an embedding host installs (`ToolHook` and `PostTurnHook` in
[`agent/hooks.rs`](../agent/hooks.rs), plus [`agent/stop_hooks.rs`](../agent/stop_hooks.rs)) are the seams this module rides
on. Inbound webhook ingestion ([`skills/webhooks/`](../skills/webhooks/), RPC namespace `webhooks`)
is unrelated.

## How it works

### Boot

[`core/runtime/bootstrap.rs`](../core/runtime/bootstrap.rs) calls `crate::hooks::init(&cfg)` during core boot.
`ops::init` does the following, in order:

1. Calls `set_host_context` with the workspace roots (the configured
   `action_dir`, plus the current turn workspace if one is set) and the crate
   version. These end up in every hook's stdin envelope.
2. If `[hooks] enabled = false`, uninstalls the bridge, installs an empty
   `HookConfig` and returns.
3. Otherwise sets the engine's default timeout from
   `[hooks] default_timeout_secs` (30 seconds by default) and calls
   `host::reload`, which reads every `hooks.json` layer on a blocking task and
   installs the result.
4. If nothing is configured, uninstalls the bridge so an unconfigured host
   pays nothing per tool call. Otherwise installs it.

`init` is safe to call again. `hooks.reload` does exactly that.

### Where the files come from

The upstream loader (`command_hooks::config::layer_paths`) reads four layers
and concatenates them, broadest first:

| Layer | Path |
| --- | --- |
| `System` | `/etc/openhuman/hooks.json` (Linux), `/Library/Application Support/OpenHuman/hooks.json` (macOS), `%ProgramData%\OpenHuman\hooks.json` (Windows) |
| `User` | `~/.openhuman/hooks.json` |
| `Workspace` | `<workspace_dir>/hooks.json` |
| `Project` | `<action_dir>/.openhuman/hooks.json` |

All of these paths derive from `host::PRODUCT_NAME` (`"OpenHuman"`), which is
also the prefix of the `OPENHUMAN_*` environment variables a hook process
receives. Concatenation is the opposite of how `config.toml` merges, on
purpose: combined with the strictest-verdict-wins rule below, it means a more
specific layer can add policy but never remove it. Each definition's `layer`
and `source_dir` are stamped from the file's location (they are
`skip_deserializing` upstream), so a repository's `hooks.json` cannot claim to
be a system layer. Only `version: 1` is accepted. A missing file is silent; an
unreadable or malformed one becomes a warning that `host::reload` logs and
`hooks.list` returns.

`host::reload` exists because the upstream `HookEngine::reload` reuses the
environment it was built with, which froze the home directory at first use.
The host version builds a fresh `HookEnvironment` each time, so a reload after
`HOME` changes reads the right user file.

### Firing a hook on a tool call

The harness already calls every registered `ToolHook` around every tool and
every `PostTurnHook` after every turn. `bridge::ConfiguredHookBridge`
implements both and registers itself through
`agent::hooks::replace_embedder_tool_hook` and
`replace_embedder_post_turn_hook` under the name `BRIDGE_HOOK_NAME`
(`"configured_hooks"`). Registering by name means a rebuilt core replaces the
bridge instead of firing every hook twice.

```text
 tool call
    |
    v
 ConfiguredHookBridge::before_tool_decision
    |
    +--> preToolUse            (generic, raw tool_input)
    |       deny? -> stop here
    +--> derived before* event (Cursor-shaped payload), if the tool has one
    |
    v
 merged HookOutput --> ToolHookDecision
                       Deny | Ask | ProceedWith(new args) | Proceed
    |
    v
 autonomy policy + approval gate (still apply after "allow")
    |
    v
 tool runs
    |
    v
 after_tool_context
    +--> postToolUse or postToolUseFailure
    +--> derived after* event, if any
    |
    v
 additional_context returned to the harness
```

Cursor has first-class events for shell commands, file reads and file edits.
In OpenHuman those are ordinary tools, so `derived_event` maps tool names onto
them:

| Tool family | Names | Pre event | Post event |
| --- | --- | --- | --- |
| `SHELL_TOOLS` | `shell`, `run_command`, `bash` | `beforeShellExecution` | `afterShellExecution` |
| `READ_TOOLS` | `file_read`, `read_diff` | `beforeReadFile` | none |
| `WRITE_TOOLS` | `file_write`, `edit`, `apply_patch`, `update_memory_md` | none | `afterFileEdit` |
| MCP | names starting `mcp_`, `mcp:` or `mcp__` | `beforeMCPExecution` | `afterMCPExecution` |

`derived_payload` builds the shape a Cursor script expects (a command string
and sandbox flag, a file path, a list of `old_string`/`new_string` edits)
instead of raw tool arguments. `file_path_argument` accepts `path`,
`file_path`, `filename` or `file`, because the file tools disagree on the name.
A write has no derived pre-event: denying a write belongs to `preToolUse`,
which already fired with the same arguments. Failures are classified into
Cursor's `timeout`, `permission_denied` or `error` by `classify_failure`.

After the turn, the bridge's `PostTurnHook::on_turn_complete` fires
`afterAgentResponse` with the assistant text, then `stop`. A `stop` hook's
`followup_message` cannot re-enter the turn that already returned, so it is
handed to `command_hooks::followup::publish`, and the entry point that owns
the conversation decides whether to start another turn.

### Moments without a tool seam

[`ops.rs`](./ops.rs) has one function per lifecycle moment that is not a tool call. Each
checks `engine.has_hooks(event)` first and returns immediately when nothing is
registered, so call sites can invoke them unconditionally.

| Function | Event | Caller |
| --- | --- | --- |
| `prompt_submitted` | `beforeSubmitPrompt` | [`web_chat/ops/start_chat.rs`](../web_chat/ops/start_chat.rs), before a prompt reaches the agent |
| `subagent_starting` | `subagentStart` | [`agent/subagent_host/ops/runner.rs`](../agent/subagent_host/ops/runner.rs), before a sub-agent spawns |
| `session_started` | `sessionStart` | not called yet |
| `session_ended` | `sessionEnd` | not called yet |
| `pre_compact` | `preCompact` | not called yet |
| `subagent_stopped` | `subagentStop` | not called yet |
| `agent_thought` | `afterAgentThought` | not called yet |

`prompt_submitted` returns a `PromptVerdict`: `Submit` with optional extra
context, or `Block` with a message for the user. It treats both
`continue: false` and `permission: "deny"` as a block, since Cursor documents
both. `subagent_starting` returns `Err(reason)` when the child must not run.
`session_ended` also releases per-session engine state and queued follow-ups.

### Prompt hooks

Most hooks are `command`: spawn a program, write the event JSON to its stdin,
read a decision from its stdout. A `prompt` hook is a policy written in
English. The engine calls back into the host through `PromptEvaluator`, which
[`host.rs`](./host.rs) implements with `prompt_eval::evaluate`. That function loads the
config with `load_config_with_timeout`, applies the hook's optional model
override to that copy's `default_model`, and makes a one-shot
`inference::ops::inference_prompt` call capped at 200 output tokens. The
override never persists and does not affect a concurrent turn. Each prompt
hook costs a model call per event, so they belong on rare, high-stakes moments.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations; re-exports `init`, `PromptVerdict` and the controller aggregators. |
| `host.rs` | The process-global `HookEngine` (`engine()`), the `HookEnvironment` built from `PRODUCT_NAME`, the home dir, `agent::platform_shell::build_tokio_command` and the prompt evaluator, and `reload`. |
| [`bridge.rs`](./bridge.rs) | `ConfiguredHookBridge`: the `ToolHook` and `PostTurnHook` impls, derived-event mapping and payload shaping, and the translation from `HookOutput` to `ToolHookDecision`. |
| `ops.rs` | `init` and the direct entry points for non-tool lifecycle moments. |
| [`prompt_eval.rs`](./prompt_eval.rs) | Model evaluation for `prompt`-kind hooks. |
| [`schemas.rs`](./schemas.rs) | The `hooks` RPC namespace. |

## Key types and entry points

- `hooks::init(&Config)` (`ops.rs`): bring the system up or tear it down from config. Called at boot and by `hooks.reload`.
- `host::engine()` (`host.rs`): the single `HookEngine` for the process. Everything dispatches through it.
- `host::reload(project_dir, workspace_dir)` (`host.rs`): re-read every layer with a fresh environment and install the result.
- `ConfiguredHookBridge::install` / `uninstall` (`bridge.rs`): add or remove the bridge from the harness seams.
- `ops::prompt_submitted`, `ops::subagent_starting` (`ops.rs`): the two wired direct entry points.
- `PromptVerdict` (`ops.rs`): the answer `prompt_submitted` gives `start_chat`.
- `config::schema::hooks::HooksConfig`: the `[hooks]` table in `config.toml`. It holds only host switches (`enabled`, default `true`; `default_timeout_secs`, default 30). The hooks themselves live in `hooks.json`.

## RPC surface

Namespace `hooks`, registered through `all_hooks_registered_controllers` in
[`core/all.rs`](../core/all.rs):

| Method | What it does |
| --- | --- |
| `hooks.list` | Configured hooks grouped by event, each with its command, type, matcher, timeout, `fail_closed`, layer and source dir; whether each event is wired; source files; load warnings. |
| `hooks.reload` | Reload `config.toml`, call `init` again, and return the fresh configuration in the same shape as `list`. |
| `hooks.test` | Fire one synthetic event (`event`, optional `payload`) and return the merged decision plus every hook run with its duration, error and output. |

`hooks.test` exists so an author can see what a hook decides without
provoking the real moment (asking the agent to run `rm -rf` to see whether a
deny rule fires). It uses `dispatch_for_test`, which runs in the foreground
even for observing events, since a detached dispatch would report nothing.

## Boundaries

- The `hooks.json` contract, event and payload types, `HookOutput` and its
  merge rule, layer loading, matchers, the command runner, the engine and
  follow-up queueing belong to `tinyagents_runtime::command_hooks` in the
  `tinyagents` repo. Fix or extend those there, then move the gitlink. See its
  own `README.md` under
  [`vendor/tinyagents/crates/tinyagents-runtime/src/command_hooks/`](../../../../vendor/tinyagents/crates/tinyagents-runtime/src/command_hooks/).
- The `ToolHook` / `PostTurnHook` seams and how the harness runs them belong
  to `agent/hooks.rs` and [`agent/tinyagents/middleware/embedder_hooks.rs`](../agent/tinyagents/middleware/embedder_hooks.rs).
- Approval and the autonomy policy belong to `security/`. A hook's `allow`
  only lets a call continue to them.
- Product-facing docs for hook authors live in [`gitbooks/developing/hooks.md`](../../../../gitbooks/developing/hooks.md).

## Gotchas

- The strictest verdict wins. `HookOutput::merge` folds deny over ask over
  allow, and layers concatenate. Adding a hook can never loosen a policy
  another hook set, so an operator's system-wide deny cannot be overridden by
  a repository's own `hooks.json`.
- Gating costs latency; observing does not. `HookEvent::is_gating` upstream
  decides whether the engine runs an event's hooks in the turn's path or on a
  background task. An audit hook that hangs must not hang the agent, and a
  gating hook must not become fire-and-forget.
- Exit codes: `0` parses stdout as a `HookOutput` (the last complete JSON
  object, so progress lines are fine), `2` denies with stderr as the reason,
  and anything else, including a timeout, a missing interpreter or unparseable
  stdout, fails open unless the definition sets `fail_closed`. Read the
  configuration and security section of `AGENTS.md` before changing that
  default.
- `Ask` currently denies. The bridge returns `ToolHookDecision::Ask`, but the
  `embedder_hooks` middleware has no approval channel, so it refuses the call
  instead of quietly allowing it.
- Not every event fires. `HookEvent::is_wired` lists `sessionStart`,
  `sessionEnd`, `preCompact`, `afterAgentThought` and `subagentStop` as
  having no call site, and the loader warns when a `hooks.json` registers one.
  Remove an event from that list only when its call site lands.

## Tests

[`bridge_tests.rs`](./bridge_tests.rs) sits beside `bridge.rs` (derived events, payload shaping,
decision translation). The engine, config, matcher and exec tests live with
the engine in `tinyagents-runtime`. Run the host tests with
`cargo test -p openhuman hooks::` or `pnpm debug rust hooks::`.

## Related docs

- [gitbooks/developing/hooks.md](../../../../gitbooks/developing/hooks.md): the `hooks.json` guide for hook authors.
- [gitbooks/developing/architecture/security.md](../../../../gitbooks/developing/architecture/security.md): the approval gate and autonomy policy that still apply after a hook allows.
- [Parent module README](../../README.md)
- [Agent harness architecture](../../../../gitbooks/developing/architecture/agent-harness.md)
- [tinyagents](../../../../vendor/tinyagents/README.md)
