# cron

The cron domain runs scheduled jobs. It owns the background scheduler loop that
fires due jobs, the three ways a job can run (a shell command, an agent turn, or
a hand-off to another domain), the delivery layer that routes a run's output to
the right place, and the `cron.*` RPC controllers and agent tools that create
and manage jobs. The scheduler is spawned as a core background service; agents
reach it through the `cron` and `schedule` tools, and the desktop UI through
the `cron` RPC namespace.

Scheduling itself is not in core. The `tinyflows` loadable module owns the
scheduler runtime (poll pass, process-wide run claims, retry with backoff,
completion and rescheduling), the cron job and run store, and the schedule
maths (cron expressions, one-shot instants, intervals, active hours, the
agent-job cadence floor). Core reaches it only through the `tinyflows-bus`
contract (`tinyflows_bus::schedule`, v3), and keeps what is policy and what is
pluggable: the callbacks the module runs jobs through, the document port it
persists through, the job-kind handler registry, delivery, the operator limits
and the RPC and tool surface. Core links neither `tinyflows-schedule` nor
`tinyflows-drivers`' schedule store (`cargo tree -p openhuman -e normal -i
tinyflows-schedule` is empty).

## How it works

### The poll loop

`scheduler::run` is the host's service. It initializes the event bus,
registers the health subscriber, builds one `SecurityPolicy` from config,
publishes `SystemStartup { component: "scheduler" }`, and then ticks forever at
`reliability.scheduler_poll_secs` (never faster than 5 seconds).

```text
  interval tick
       |
       v
  tick_once --- JobDispatcher.tick ---> TickScheduler (tinyflows module)
       |                                     |  DueJobs/claim_slot over the
       |                                     |  document port (cron::documents_port)
       |  error -> HealthChanged{healthy:false} (only on transition)
       |  ok    -> HealthChanged{healthy:true}  (only on transition)
       v                                     v
  LiveAgentSchedulers.tick          per due job, in the module:
  (one JobDispatcher per live       claim the slot, retry loop, then call back
   embedded agent)                  into the host's ScheduleHost object:
                                    Execute / Failed / Deliver / ScheduleTrigger
```

Each scope (the operator, and every live embedded agent) has one
`JobDispatcher`: a manual-tick scheduler started in the module
(`StartScheduler { manual_ticks: true, storage: Documents }`) plus the
`ScheduleHost` callback object it calls back (`cron::bus_host`), bound to that
scope's config, security policy and `CoreContext`. The host drives the ticks
rather than letting the module run its own loop so the poll interval, the
transition-only `HealthChanged` signals and the live-agent reconcile stay in one
place, and so a test or embedder can run one deterministic pass
(`run_live_agent_pass`). Dropping a dispatcher stops its scheduler
(`StopScheduler`) and unserves its object.

Health is published on transitions only, so an idle queue does not put a
steady `healthy: true` event on the bus every poll. A successful tick after a
failed job re-emits `healthy: true`, which lets the Docker health check recover
on its own (#3312).

Every execution gets a fresh `run_id` (a UUID). The same id names the run
record, the origin delivery key (`cron:<job>:<run>`) and, for channel-origin
jobs, the turn's message id.

### Run claims

The module keeps process-wide run claims per `(scope, job id)`; the host uses
the scope `openhuman` for every scheduler so the poll loop of any agent and
Run Now refuse to start a job another of them is running. `cron.run` also
takes a host-side guard (`scheduler::try_acquire_run`) so a second concurrent
Run Now is rejected at once; a Run Now that loses to a scheduled run is
refused by the module's `ExecuteJob` ("job is already running"), and the host
records nothing for it. A claim releases on drop, so a scheduled run and a manual run
never overlap.

### Executing a job

The module's retry loop wraps all three job types in
`reliability.scheduler_retries` retries (a job's own `policy.retries` wins) with
exponential backoff starting at `reliability.provider_backoff_ms` (at least
200 ms). `shell` and `agent` attempts go to the `Execute` callback
([`scheduler/executor.rs`](./scheduler/executor.rs)), which returns each
failure classified `retryable` or permanent; `flow` rows go to
`ScheduleTrigger` ([`scheduler/run_delivery.rs`](./scheduler/run_delivery.rs)).

Which code runs a job is a table, not a `match`: the job-kind registry
([`handlers.rs`](./handlers.rs)) maps each `JobType` to a `JobKindHandler`
(`shell` and `agent` by default, `flow` for schedule ticks and `system:<name>`
jobs; the built-ins are in [`scheduler/builtin.rs`](./scheduler/builtin.rs)).
`handlers::register_kind` swaps one for the life of the returned guard and
`system_job_handlers::register` adds a `system:<name>` handler to the same
registry; an embedder does it through `RuntimeBuilder::cron_job_handler` and
`Runtime::register_job_handler`. Operator limits, per-run bookkeeping and the
permanent-failure classification stay in the executor and are not pluggable.

`shell` jobs run through `run_job_command_with_timeout`
([`scheduler/shell_job.rs`](./scheduler/shell_job.rs)). Before spawning anything,
it checks `can_act`, `is_rate_limited`, the host security policy against command
facts returned by TinyBox `AnalyzeCommand`, and `record_action` against the
action budget. Missing module analysis fails closed; no local parser replaces
it. Any refusal returns a string starting `blocked by security policy:`, which
the retry loop treats as final. Execution uses TinyBox `StartExec` on a
module-owned passthrough resource, preserving the action directory and the
shell's default pipeline exit status. The command has a hard 120 second timeout
(`SHELL_JOB_TIMEOUT_SECS`); timeout cancellation and output cleanup are
acknowledged by TinyBox before the resource is closed. With the autonomy policy
disabled (the default), most policy checks are inert; see the project
`CLAUDE.md`.

`agent` jobs run a full agent turn ([`scheduler/agent_run.rs`](./scheduler/agent_run.rs)):

1. `build_run_prompt` ([`scheduler/origin_context.rs`](./scheduler/origin_context.rs)) composes the prompt from
   the job's `session_target` (see below).
2. A per-job `model` override is applied to a cloned `Config`.
3. The job's `agent_id` (default `orchestrator`) is looked up in
   `AgentDefinitionRegistry`. A definition's `ModelSpec` is resolved onto the
   cloned config; a `Hint` goes through
   `inference::provider::create_chat_model_with_model_id` so the workload's
   configured model is used on any provider. The iteration cap is left to the
   session builder (#4868).
4. `build_agent_for_cron_job` builds an `OpenHumanSessionHost` with
   `from_config_for_agent`, falling back to the `orchestrator` definition if the
   requested one fails to build.
5. The turn is tagged with event context `cron:<job>` on channel `cron`, and
   `start_cron_turn_clean` sets `suppress_transcript_autoload`. A cron agent has
   no thread, so without this its first turn would resume whatever unthreaded
   transcript was newest for that agent name.
6. The turn runs inside `agent::turn_origin::with_origin` with the origin from
   `origin::turn_origin_for_job_run`. The morning briefing job also runs under a
   24 hour task-recency window so task-fetch tools only return recent tasks.

An agent failure is classified before anything is logged. The user sees a
canned string from `classify_agent_anyhow_for_user`
([`scheduler/failure_classification.rs`](./scheduler/failure_classification.rs)); the full error chain goes only to
observability. Some failures stop the retry loop on the first attempt and skip
the retries-exhausted error report: backend session expired, provider
insufficient credits (402), managed-backend budget exhausted (400), API key
unset, and a local LLM that is unreachable or has no model loaded.

`flow` jobs run nothing in cron. The module hands the row's flow id to the
host `ScheduleTrigger` callback. If it is `system:<name>` (see system jobs below) the sink
publishes `DomainEvent::CronSystemJobDue` and awaits the registered handler;
otherwise the command is a flow id and the sink calls
`flows::bus::FlowTriggerSubscriber::dispatch_schedule_tick` directly, with no
event in between.

### After the run

The module calls the host `Deliver` callback (`deliver_run`) first, records the
run with its `DeliveryStatus`, then decides the job's future:

- A `Schedule::At` job is one-shot. If it has `delete_after_run` and succeeded,
  it is removed. Otherwise it is disabled and keeps its run history.
- Every other schedule is rescheduled by the module, which computes the
  next run in the store.

A delivery failure marks the run failed unless `delivery.best_effort` is set.

### Delivery

`DeliveryConfig.mode` decides where output goes. `deliver_run`
([`scheduler/delivery.rs`](./scheduler/delivery.rs)) never posts a failed or empty run into a chat (empty
means blank or the placeholder `agent job executed`). Failures still reach the
alerts tab and run history.

| Mode | What happens |
| --- | --- |
| `none` | Silent. Output lives in the job's `last_output` only. This is `DeliveryConfig::default()`. |
| `proactive` | Publishes `ProactiveMessageRequested`. `channels::proactive::ProactiveMessageSubscriber` pushes to the in-app web stream and mirrors to `channels_config.active_channel` when set. Used for briefings and other desktop-first output. |
| `announce` | Requires `channel` and `to`. Publishes `CronDeliveryRequested`; `cron::bus::CronDeliverySubscriber` (`cron::delivery`) sends it through the named `tinychannels_bus::Channel`. The `cron_add` tool checks `to` against the channel's `allowed_users`. |
| `origin` | Sends the reply back into the conversation that created the job ([`scheduler/origin_delivery.rs`](./scheduler/origin_delivery.rs)). Requires `CronJob.origin`. |

Origin delivery treats blank output or exactly `NO_REPLY` as `Suppressed`:
nothing is sent and no alert is raised. For a web origin, the reply is stored in
the origin thread under `run_reply_message_id("cron:<job>:<run>")`, the
web-channel `proactive_message` event is emitted with that `thread_id` and
`request_id`, and `append_to_origin_transcript` appends it to the thread's agent
transcript in the session store, under the key the session host wrote it with
(`agent_key_for` of the job's tenant and agent), idempotent on the same key, and
it waits out a live turn on the session, up to 120 seconds. For a channel origin, the reply goes to the
origin's `reply_target` through `cron::channel_bridge` and is appended to that
chat's in-memory history.

Independently of the mode, `push_cron_alert` writes an entry into the
notifications store (the alerts tab) for any failed run, and for a non-empty
successful run in a delivering mode (`proactive`, `announce`, `origin`). A
`none`-mode success stays silent, and so does an origin run that chose
`NO_REPLY`.

### Origin and session targets

A job created by an agent inside a conversation records that conversation as a
`JobOrigin`. `origin::current_job_origin` reads the live `AgentTurnOrigin`: a
`WebChat` turn becomes `JobOrigin::Web { thread_id }`, a channel processor's
`ExternalChannel` turn becomes `JobOrigin::Channel { channel, reply_target,
history_key, sender }`, and any other origin records none.

`job_builder::create_agent_job` is the one path the `cron` and `schedule` tools
use to build agent jobs. With an origin, an agent job defaults to
`session_target: current` and `delivery: origin`; without one it defaults to
`isolated` and `proactive` (`default_delivery`). An explicit `delivery` or
`session_target` always wins. `validate_delivery` and
`check_origin_requirements` reject combinations that cannot work. An `announce`
whose `to` is the origin's own reply target on the same channel skips the
`allowed_users` check. The origin is stored on the job only when something
uses it.

The session target shapes the prompt:

| Target | Prompt |
| --- | --- |
| `isolated` | `[cron:<id> <name>] <prompt>` |
| `current` | An unattended-run preamble (the reply is delivered as-is; reply `NO_REPLY` to send nothing), then a tail of the origin conversation (last 10 messages, at most 1400 characters, newest kept), then the task. Web history comes from the thread store, channel history from `channel_bridge`. |
| `main` | Runs like `current`; it is not yet a distinct main session. |

Trust follows the origin. A job with a channel origin runs under
`AgentTurnOrigin::ExternalChannel { message_id: "cron:<job>:<run>" }`, so its
external-effect tools stay gated the way its creator's were. Web-origin and
origin-less jobs run as `TrustedAutomation { source: Cron }`. This grants the
saved agent prompt read-only tool use and delivery to the job's configured
destination. It does not grant shell, network, write, or cron mutation tools;
those calls are denied on every run, including when approval prompts are
globally disabled. A shell job runs its approved, stored command directly;
this limit applies to agent tool calls. Creating a job skips the approval gate only when
`origin::is_self_scoped_agent_job` holds: the turn is a channel conversation,
the job is an agent job with no shell `command`, and its delivery resolves to
that same conversation (no `delivery`, `origin`, or a matching `announce`).
`CronAddTool` and the `schedule` tool express this through
`external_effect_with_args`. Shell jobs, other recipients, `proactive`/`none`
delivery and non-channel turns are gated (GHSA-f46p-6vf9-64mm).

### Agent-job minimum interval

An agent job is a full inference turn per run, so agent jobs may not run more
often than `MIN_AGENT_JOB_INTERVAL` (5 minutes). `ValidateSchedule` (with `agent`) is
enforced in the module's store whenever an agent job is created or its schedule
updated, so the `cron` tool, the `schedule` tool and the `cron.add` /
`cron.update` RPCs all reject the same schedules, with a message naming the two
runs that would be too close. Shell and flow jobs are exempt.

The check, `RunsCloserThan`, walks a bounded number of consecutive
occurrences and reports the first pair closer than the floor. An irregular
expression such as `1,2,30 * * * *` is judged by its tightest gap, and
wrap-around counts (`*/7 * * * *` fires at :56 and then :00). Rows that predate
the floor keep running; `warn_if_high_frequency_agent_job` logs `Cron agent job
'<id>' is scheduled more frequently than every 5 minutes` on each run instead
(#6158).

### Seeded and system jobs

`seed::seed_proactive_agents` runs when onboarding flips to completed
([`config/ops/ui.rs`](../config/ops/ui.rs)). It dedupes named jobs, prunes a legacy one-shot `welcome`
job, and creates the `morning_briefing` agent job (daily at 7 AM, `proactive`
delivery) disabled until the user opts in. `seed::prune_retired_jobs` removes
rows for retired features (for example the TinyPlace autopilot, matched by its
`agent_id`) on boot and on user-scope activation.

System jobs let another domain own a recurring task while still showing up in
the routines list. A system job is a `flow` row whose command is
`system:<name>` ([`system_jobs.rs`](./system_jobs.rs)). When it fires, cron publishes
`CronSystemJobDue` and the owning domain does the work. Memory owns the two
current ones, `memory_background` and `memory_sources_sync` (every 15 minutes),
handled in `memory::bus`. `ensure_memory_jobs` is idempotent: it creates a
missing row, reschedules a drifted one, and removes the retired
`memory_context_refresh` row. It runs from [`core/runtime/services.rs`](../core/runtime/services.rs) and from
[`security/credentials/ops/user_scope.rs`](../security/credentials/ops/user_scope.rs).

### Storage

Every `store` function sends one typed `StoreOperation` through
`client::store_blocking` to the module's `ScheduleStore` member
([`client.rs`](./client.rs)); the module runs `CronDocuments` (jobs, runs and
per-job policy as documents). It persists through the acting scope's documents
on the storage ports via the host's document port
([`documents_port.rs`](./documents_port.rs)): one `ScheduleDocuments` object per
`(workspace, storage scope)`, which relays `get`/`put`/`query`/`atomic_batch`
and friends mechanically, preserves the `conflict` error kind and refuses any
collection other than the four schedule ones. The configured history cap and
due-job batch size ride on every request. tinyflows has no SQLite store of its
own, so there is one path whatever the storage URL says: the installed backend
when there is one, else the workspace default (`sqlite:<workspace>/storage`).
The `jobs.db` tinyflows used to write (with its policy side table) is decoded
by `tinyflows-import` into bus records and replayed through the module's
`import_job` / `import_run` operations, once, at boot for an installed backend
(`storage::import::tinyflows`), and is never written again.
`Reschedule` advances a job only if its stored schedule still matches the job
that fired, so two processes sharing one database don't double-advance it.

With `[components] tinyflows = false` (or a build without the `modules`
feature) nothing loads and every cron call answers `COMPONENT_DISABLED:tinyflows`
(the RPC handlers) or an unavailable error; the scheduler service does not start.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module wiring and re-exports. Re-exports the schedule records from `tinyflows_bus::schedule` so `cron::CronJob`, `cron::Schedule` and friends resolve here (none is redeclared). |
| [`store.rs`](./store.rs) | The synchronous job and run API the core calls. Applies the operator limits, then sends one `StoreOperation` to the module. |
| [`client.rs`](./client.rs) | Typed calls to the module's `Schedule` object: `store_in`/`store_blocking`, and the maths and validation members (`next_run`, `validate_schedule`, `runs_closer_than`, `normalize_expression`, `parse_human_delay`, `default_delivery`, `check_origin_requirements`, ...). |
| [`documents_port.rs`](./documents_port.rs) | The `ScheduleDocuments` callback object: the scope's document store, served to the module. |
| [`bus_host.rs`](./bus_host.rs) | `Binding` (config, policy, context of one scope) and the `ScheduleHost` callback object (`Execute`, `Failed`, `Deliver`, `ScheduleTrigger`), served per scheduler. |
| [`handlers.rs`](./handlers.rs) | The job-kind handler registry (`JobKindHandler`, `JobOutcome`, `register_kind`) and the system-job handlers. |
| `ops.rs` | Business operations: `add_once`, `add_once_at`, pause/resume, `update_cron_job`, `try_acquire_run`, and the async `cron_*` RPC operations. Also re-exported as `cron::rpc`. |
| [`schemas.rs`](./schemas.rs) | Controller schemas and handlers for the `cron` namespace. |
| [`scheduler.rs`](./scheduler.rs) | The host loop: `run`, `tick_once`, `JobDispatcher` (one manual-tick module scheduler per scope), the live-agent pass, `execute_job_now` (Run Now, through `ExecuteJob`), the manual-run guard and the health signals. |
| `scheduler/executor.rs` | `HostExecutor`: one shell or agent attempt through the registry, failure classification, the retries-exhausted report, the cadence warning. |
| `scheduler/builtin.rs` | The default `shell`, `agent` and `flow` handlers. |
| `scheduler/run_delivery.rs` | `deliver` (delivery plus completion events) and `trigger` (flow schedule trigger, system jobs). |
| `scheduler/failure_classification.rs` | Canned user messages and the permanent-failure classifiers that halt retries. |
| `scheduler/shell_job.rs` | Security-gated shell execution with a timeout. |
| `scheduler/agent_run.rs` | Agent-job turns, and `build_agent_for_cron_job`. |
| `scheduler/origin_context.rs` | `build_run_prompt`: the isolated prefix or the `current` preamble plus conversation tail. |
| `scheduler/delivery.rs` | `deliver_run` and `deliver_job`, the mode switch, and `push_cron_alert`. |
| `scheduler/origin_delivery.rs` | `deliver_to_origin`, `NO_REPLY` suppression, transcript append. |
| [`origin.rs`](./origin.rs) | `JobOrigin` capture from the live turn, the approval-gate exemption, and the turn origin a run executes under. |
| [`job_builder.rs`](./job_builder.rs) | `create_agent_job`: origin-aware defaults and delivery validation shared by the `cron` and `schedule` tools. |
| [`channel_bridge.rs`](./channel_bridge.rs) | Cron's handle on the channel runtime: per-chat history and live channels, registered by `start_channels`. |
| [`bus.rs`](./bus.rs) | `CronDeliverySubscriber`, which sends `announce` output to a channel. |
| [`seed.rs`](./seed.rs) | Built-in proactive jobs and pruning of retired rows. |
| `system_jobs.rs` | `system:<name>` flow rows owned by other domains. |
| [`tools.rs`](./tools.rs), `tools/` | Agent tools. See [tools/README.md](tools/README.md). |
| (moved) | The background-LLM power/CPU gate now lives in [`platform/llm_gate`](../platform/llm_gate/README.md). |

## Key types and entry points

- `CronJob`, `CronJobPatch`, `CronRun`, `Schedule` (`Cron` / `At` / `Every`),
  `JobType` (`Shell` / `Agent` / `Flow`), `SessionTarget` (`Isolated` /
  `Current` / `Main`), `JobOrigin` (`Web` / `Channel`), `DeliveryConfig`,
  `DeliveryStatus`, `ActiveHours`: the job and run model, defined in
  `tinyflows_bus::schedule::types` and re-exported from `mod.rs`.
  `delivery_mode` holds the mode string constants.
- `AgentJobSpec`: the store's agent-job creation spec (from `tinyflows_bus::schedule`),
  used by `add_agent_job_from_spec`.
- `scheduler::run(ctx, config)`: the service entry point.
- `scheduler::execute_job_now` and `scheduler::deliver_job`: what Run Now uses
  to execute and deliver outside the loop.
- `add_job`, `add_shell_job`, `add_agent_job`, `add_agent_job_with_definition`,
  `add_agent_job_from_spec`, `add_flow_schedule_job`, `find_flow_schedule_job`,
  `get_job`, `list_jobs`, `update_job`, `remove_job`, `due_jobs`, `list_runs`,
  `record_run`, `record_run_with_delivery`: store operations in `store.rs`.
- `client::next_run`, `normalize_expression`, `validate_schedule`,
  `runs_closer_than`, `schedule_cron_expression`: the schedule maths, which runs
  in the module; `TooFrequent` and `MIN_AGENT_JOB_INTERVAL` are contract types.
- `job_builder::create_agent_job`: the creation path for agent jobs from a
  conversation.
- `origin::current_job_origin`: the origin of the live turn, if any.
- `system_jobs::ensure_system_job` and `ensure_memory_jobs`: seeding rows for
  domain-owned recurring work.
- `tools::CronTool` / `CRON_TOOL_NAME`: the one cron tool the model sees, over
  the hidden `CronAddTool`, `CronListTool`, `CronUpdateTool`,
  `CronRemoveTool`, `CronRunTool` and `CronRunsTool`.

## RPC surface

Namespace `cron`, registered through `all_cron_registered_controllers` in
[`core/all.rs`](../core/all.rs):

| Method | What it does |
| --- | --- |
| `cron.add` | Create a job (shell or agent, schedule, prompt, session target, model, agent id, delivery, `delete_after_run`, origin). |
| `cron.list` | List all jobs. |
| `cron.update` | Patch a job. |
| `cron.remove` | Delete a job. |
| `cron.run` | Run Now. Records a `queued` placeholder run, returns at once, then executes in the background through `execute_job_now` and `deliver_job` and records the real result. Refused when `cron.enabled` is false or the job is already running. |
| `cron.runs` | Run history for a job. |

## Event bus

Cron publishes, through [`core/bus.rs`](../core/bus.rs), these `DomainEvent` variants from
[`core/events.rs`](../core/events.rs):

- `SystemStartup` and `HealthChanged` for component `scheduler`.
- `CronJobTriggered` and `CronJobCompleted` around each scheduled execution
  (output truncated to 512 characters).
- `ProactiveMessageRequested` (`proactive` delivery), consumed by
  `channels::proactive`.
- `CronDeliveryRequested` (`announce` delivery), consumed by
  `cron::bus::CronDeliverySubscriber`.
- `CronSystemJobDue`, consumed by `memory::bus`.

## Who calls in

- `core/runtime/services.rs` spawns `scheduler::run` as the `cron` background
  service when `cron.enabled` is set, after flows reconcile their schedule
  triggers. It also calls `ensure_memory_jobs` and `prune_retired_jobs`.
- `core/all.rs` registers the controllers.
- [`tools/impl/system/schedule.rs`](../tools/impl/system/schedule.rs) is the `schedule` tool, built on the same
  store functions and `create_agent_job`.
- [`channels/runtime/startup/start_channels.rs`](../channels/runtime/startup/start_channels.rs) registers
  `CronDeliverySubscriber` and the channel bridge.
- [`flows/ops/triggers.rs`](../flows/ops/triggers.rs) creates and removes flow schedule rows
  (`bind_schedule_trigger` / `unbind_schedule_trigger`), idempotent through
  `find_flow_schedule_job`. These rows are never created by the `cron_add`
  tool, whose `job_type` is `shell` or `agent` only.
- `config/ops/ui.rs` calls `seed_proactive_agents`;
  `security/credentials/ops/user_scope.rs` calls `prune_retired_jobs` and
  `ensure_memory_jobs` on user-scope activation.

## Boundaries

- The schedule model, next-run computation, cadence floor, scheduler runtime,
  claims, retry timing and the job and run store (CRUD, output truncation,
  pruning) belong to the `tinyflows` module (`vendor/tinyflows`, contract
  `tinyflows_bus::schedule`). Change them there, not here; a missing operation
  is a contract change upstream first.
- Shell sandboxing policy is `security::SecurityPolicy`. Cron only asks it.
- Flow execution is the `flows` domain. Cron only fires the schedule trigger from the `ScheduleTrigger` callback.
- Channel construction and sending belong to `channels`. Cron reaches channels
  through events and `channel_bridge`.
- Memory's background work is memory's. Cron only fires the system job.
- Agent construction, definitions and the approval gate belong to `agent`.

## Gotchas

- the LLM gate (`platform::llm_gate`) is not part of cron; it used to live here, but the poll loop does not
  consult it. Its consumers are `modules/memory_host.rs` and
  [`security/credentials/`](../security/credentials/).
- Dispatch is non-blocking: the module spawns each due job, bounded by
  `scheduler.max_concurrent`, so one long job does not hold up the poll loop. A
  job is claimed (module-side, shared with Run Now) from dispatch until its run
  is persisted, and the scheduler never dispatches a claimed job. A recurring
  job's `next_run` is advanced at dispatch (at-most-once per slot), then
  recomputed from the finish time as before.
- Callbacks outlive short runtimes only because they are served on the module
  runtime and their work is `offload`ed to the runtime that built the binding.
  The module gives `Execute`/`Deliver`/`ScheduleTrigger` a two-hour deadline
  (the bus default of 30 s would cut every agent turn short).
- Per-job `JobPolicy { retries, single_flight }` is a field of the job
  (`CronJob::policy`), read and written through the job store. The default
  policy is stored as none.
  `retries: Some(0)` means exactly one attempt; with `single_flight` a slot that
  comes due mid-run is recorded as a `skipped` run, and Run Now refuses while the
  job runs. Policies kept apart from the job before (the `cron_job_policies`
  table, then collection) are folded into the job by the importer and by
  the module's store when it opens.
- A `CronJobPatch` crosses the bus as JSON and `agent_id` is a double option
  whose `null` means "clear"; the contract (tinyflows v0.8.23) omits it when
  unset, so an unrelated patch leaves the agent definition alone
  (`a_patch_over_the_bus_keeps_the_agent_definition_it_does_not_mention`).
- The hosted-profile sweep runs an agent-only scheduler (`JobDispatcher::agent_only`):
  a due shell or flow row of a profile is refused by the callback (and recorded
  as a failed run), never executed. The sweep ticks on the module's clock; its
  `now` only decides which profiles are opened.
- System jobs: `system_job_handlers::register(name, handler)` installs an awaited in-process
  handler; the scheduler still publishes `CronSystemJobDue`, then records the handler's
  result as the run's status. With no handler, behaviour is unchanged.
- Host agents: `agent::host_agents` is a process-wide `HostAgentResolver`. Cron agent jobs
  (`scheduler/agent_run.rs`) and workflow `agent` nodes consult it before the registries and run
  as the embedder's agent (its prompt, host tools and own `CoreContext`).
- A cron agent turn must suppress transcript autoload, or it resumes an
  unrelated conversation. Any new cron path that builds a session host needs
  `start_cron_turn_clean`.
- `Schedule::At` jobs are always terminated after one run. Rescheduling would
  leave `next_run` in the past and refire the job every poll.
- Agent-job failure text shown to users is always a fixed string. Do not put
  the raw error into notifications or chat; it can carry provider URLs.

## Tests

Tests sit beside their modules as `<module>_tests.rs` (for example
[`scheduler_tests.rs`](./scheduler_tests.rs), which pulls in the `scheduler_*_tests.rs` files, [`bus_host_tests.rs`](./bus_host_tests.rs) (each `ScheduleHost` member driven over the bus), [`documents_port_tests.rs`](./documents_port_tests.rs), [`client_tests.rs`](./client_tests.rs), [`handlers_tests.rs`](./handlers_tests.rs), [`cutover_tests.rs`](./cutover_tests.rs) (system, seeded and flow rows still fire), plus
[`store_tests.rs`](./store_tests.rs), [`ops_tests.rs`](./ops_tests.rs), [`origin_tests.rs`](./origin_tests.rs), [`job_builder_tests.rs`](./job_builder_tests.rs),
[`system_jobs_tests.rs`](./system_jobs_tests.rs), `scheduler/origin_*_tests.rs` and the per-tool files in
`tools/`). Timing, claims, retry and history are tested in the module (`vendor/tinyflows`); here the tests run against the real module linked into the test binary (`modules::tinyflows::fixture`, the `static-link` build of `tinyflows-module`). The import of
`jobs.db` is covered in `storage/import/tinyflows_tests.rs` and `store_tests.rs`.

```bash
cargo test -p openhuman cron::
pnpm debug rust cron
```

## Agent-only limits (`limits.rs`)

`[cron] agent_jobs_only = true` is an operator-owned mode for hosted
profiles (`SaasConfig::cron` forces it, with `min_interval_secs` = 300 and
`max_active_jobs` = 20, onto every profile; a profile's own config cannot
loosen it). It refuses shell and flow jobs, restricts delivery to `origin` and
`none`, requires a job to be created from a conversation (the origin is read
from the live turn, never from tool arguments), keeps recurring jobs
`min_interval_secs` apart and caps enabled jobs at `max_active_jobs`, counted
in the store. Checks run in `store.rs` when a job is added or updated, and
again where it runs (the `Execute` callback, Run Now, the run tool), so rows stored
earlier are refused too.

## Further reading

- [Parent module README](../../README.md)
- [Cron and scheduling](../../../../docs/gitbooks/en/features/native-tools/cron.md)
- [Workflows](../../../../docs/gitbooks/en/features/workflows.md)

## Relation to `[components].flows`

`flow` schedule rows are fired through the `flow` handler. When `[components]
flows = false`, such a tick is answered as skipped (`COMPONENT_DISABLED:flows`,
no retry, the recurrence advances) while agent, shell and system jobs keep
running; `cron.enabled = false` still stops the whole scheduler.
