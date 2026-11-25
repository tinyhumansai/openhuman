# Cron tools

Agent-facing tools over the `cron` domain. Six per-operation tools
([`add.rs`](./add.rs), [`list.rs`](./list.rs), [`update.rs`](./update.rs), [`remove.rs`](./remove.rs), [`run.rs`](./run.rs), [`runs.rs`](./runs.rs)) are the
implementation; [`collapsed.rs`](./collapsed.rs) wraps them into a single action-dispatched
`CronTool`. The cron tool types are re-exported from `crate::cron::tools`; `tools/ops.rs`
assembles the registered tool set.

The cron-domain tools first check `config.cron.enabled` and return an error
result when cron is disabled. The separate `schedule` tool exposes its own
one-shot and recurring schedule actions.

## Registration state

`crate::tools::ops::all_tools` registers the six per-operation tools as the
advertised scheduler surface: each keeps the default `ToolExposure::Direct`
and is a member of the `scheduling` tool pack (`CRON_OPERATION_TOOLS`), so the
pack's `GroupMode` decides whether they are on the wire (`Advertised`) or
reached through `use_skill` (`Withheld`, the default). Each carries only its
own fields and a one-line description; the rules (grounding the time,
confirming, delivery defaults, `NO_REPLY`) live once in the pack's guide,
`tools/toolpacks/guides/scheduling.md`.

## Collapsed dispatch (`collapsed.rs`)

`CronTool` (`CRON_TOOL_NAME = "cron"`) dispatches the same six on one `action`
field (`list` / `add` / `update` / `remove` / `run` / `runs`) through
`tinytools::collapse` (`merge_action_schemas`, `resolve`,
`args_without_action`), so schedule parsing, the `SecurityPolicy` check and
delivery validation live in exactly one place. It is
`ToolExposure::Deferred`: never listed up front, found by `tool_search`, and
run when called by name, so saved skills and scripts that call `cron` keep
working. `widen_named_scope` widens a named tool scope that lists `cron`
to the six, which `openhuman_embed` applies to agent and profile scopes, so an
embedder that named `cron` before the split keeps its scheduler.
`permission_level_with_args` / `external_effect_with_args` resolve the real
per-action answer once `action` is known; with a missing or unknown `action`
they fall back to the strictest member (`Execute`, shared by `add` / `update`
/ `run`) and `true`, so an unparseable call over-restricts rather than under-.

## Per-operation tools

| Tool | Name | Permission | `external_effect` | Delegates to |
| --- | --- | --- | --- | --- |
| `CronAddTool` | `cron_add` | `Execute` | `true` | `cron::add_shell_job` / `cron::add_agent_job` |
| `CronListTool` | `cron_list` | trait default (`ReadOnly`) | trait default (`false`) | `cron::list_jobs` |
| `CronUpdateTool` | `cron_update` | `Execute` | `true` | `cron::update_job` (after `SecurityPolicy::is_command_allowed` on `patch.command`) |
| `CronRemoveTool` | `cron_remove` | `Write` | `true` | `cron::remove_job` |
| `CronRunTool` | `cron_run` | `Execute` | `true` | `cron::get_job` + Schedule v6 manual worker lifecycle; the module owns run history and completion |
| `CronRunsTool` | `cron_runs` | trait default (`ReadOnly`) | trait default (`false`) | `cron::list_runs` (output truncated to `MAX_RUN_OUTPUT_CHARS = 500`) |

The mutating tools (`add`, `update`, `remove`, `run`) set
`external_effect = true` so `ApprovalGate::intercept` runs even for turns that
originate from an inbound channel message (GHSA-f46p-6vf9-64mm): they persist
or immediately execute a stored command or agent prompt on the host.

### `CronAddTool` details

- Args: `name` (derived as a slug of `prompt` when omitted), `schedule`
  (`kind: cron | at | every`), `job_type` (`shell` | `agent`, inferred from
  the presence of `prompt` when omitted), `command` or `prompt`,
  `session_target` (`SessionTarget`, default `Isolated`), `model`,
  `delivery`, `delete_after_run` (defaults to `true` for `at` schedules).
- Shell commands are checked with `SecurityPolicy::is_command_allowed` before
  being scheduled.
- Agent jobs default to `DeliveryConfig { mode: "proactive", best_effort:
  true, .. }`. `validate_delivery` only inspects `mode: "announce"`: it
  requires `channel` and `to`, exempts `web`, rejects an unconfigured
  channel, accepts any `to` when the channel's `allowed_users` is empty, and
  otherwise requires `to` to be in that list: this blocks scheduling a cron
  whose output is delivered to an arbitrary chat id (#928).
- `JobType::Flow` is unreachable through this tool (flow-schedule rows are
  created internally by `flows::ops::flows_set_enabled` via
  `cron::add_flow_schedule_job`); the arm returns an explicit error instead of
  `unreachable!()` in case the `job_type` heuristic above ever changes.

## Related

- `cron` domain: [`../README.md`](../README.md): job/run model, scheduler,
  delivery modes, agent-job minimum interval.
- [`schedule.rs`](./schedule.rs): the separate `schedule` tool for recurring and
  one-shot jobs; it is not part of the `scheduling` pack.
- `crates/openhuman-core/src/tools/impl/meta/collapse.rs` (module path
  `tinytools::collapse`): the generic
  action-collapsing helper `collapsed.rs` builds on.
- [Cron and scheduling](../../../../../docs/gitbooks/en/features/native-tools/cron.md)

## Tests

[`add_tests.rs`](./add_tests.rs), [`list_tests.rs`](./list_tests.rs), [`update_tests.rs`](./update_tests.rs), [`remove_tests.rs`](./remove_tests.rs),
[`run_tests.rs`](./run_tests.rs), [`runs_tests.rs`](./runs_tests.rs) cover each per-operation tool;
[`collapsed_tests.rs`](./collapsed_tests.rs) covers the merged schema, per-action versus fallback
permission levels, the unknown-action error, and that every member is
`Hidden`.
