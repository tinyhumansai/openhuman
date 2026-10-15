# Cron tools

Agent-facing tools over the `cron` domain. Six per-operation tools
([`add.rs`](./add.rs), [`list.rs`](./list.rs), [`update.rs`](./update.rs), [`remove.rs`](./remove.rs), [`run.rs`](./run.rs), [`runs.rs`](./runs.rs)) are the
implementation; [`collapsed.rs`](./collapsed.rs) wraps them into a single action-dispatched
`CronTool`. All seven are re-exported wholesale by `crate::tools` (`pub use
crate::cron::tools::*` in `tools/mod.rs`).

Every tool first checks `config.cron.enabled` and returns an error result
when cron is disabled.

## Registration state

`crate::tools::ops::all_tools` registers `CronTool` as the compact, advertised
scheduler surface, plus the six per-operation tools as hidden aliases behind
it. Each per-operation tool reports `ToolExposure::Hidden` (`fn exposure`),
which `tools/impl/meta/tool_search.rs` strips from the advertised catalogue,
so they stay dispatchable by name (replayed transcripts, saved skills,
`user_filter.rs` family `cron`, toolpack `scheduling`) without cluttering the
model's tool list with six near-duplicate schemas.

## Collapsed dispatch (`collapsed.rs`)

`CronTool` (`CRON_TOOL_NAME = "cron"`) advertises one schema with an `action`
field (`list` / `add` / `update` / `remove` / `run` / `runs`) instead of six
near-duplicate schemas: four of the six take only `job_id`. Each action
forwards to the matching per-operation tool through
`tinytools::collapse` (`merge_action_schemas`,
`resolve`, `args_without_action`), so schedule parsing, the `SecurityPolicy`
check and delivery validation live in exactly one place.
`permission_level_with_args` / `external_effect_with_args` resolve the real
per-action answer once `action` is known; with a missing or unknown `action`
they fall back to the strictest member (`Execute`, shared by `add` / `update`
/ `run`) and `true`, so an unparseable call over-restricts rather than under-.
The argument-free `permission_level` / `external_effect` report the same
strictest values.

## Per-operation tools

| Tool | Name | Permission | `external_effect` | Delegates to |
| --- | --- | --- | --- | --- |
| `CronAddTool` | `cron_add` | `Execute` | `true` | `cron::add_shell_job` / `cron::add_agent_job` |
| `CronListTool` | `cron_list` | trait default (`ReadOnly`) | trait default (`false`) | `cron::list_jobs` |
| `CronUpdateTool` | `cron_update` | `Execute` | `true` | `cron::update_job` (after `shell_job::authorize_shell_command` on `patch.command`) |
| `CronRemoveTool` | `cron_remove` | `Write` | `true` | `cron::remove_job` |
| `CronRunTool` | `cron_run` | `Execute` | `true` | `cron::get_job` + `cron::scheduler::execute_job_now`, then `cron::record_run` / `cron::record_last_run` |
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
- Shell commands are checked by the security module (`shell_job::authorize_shell_command`) before
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
- [`crates/openhuman-core/src/tools/impl/system/schedule.rs`](../../tools/impl/system/schedule.rs): the separate
  one-shot `schedule` tool built on `cron::add_once` / `cron::add_once_at`;
  not part of the collapse above.
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
