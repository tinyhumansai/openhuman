# Triage

Classification pipeline for external triggers. It turns a source-specific
event (Composio webhook, incoming HTTP webhook, task-source card, desktop
notification, ad hoc RPC) into a `drop` / `acknowledge` / `react` / `escalate`
decision, then dispatches the sub-agent that decision implies. It does not
own the trigger sources themselves (Composio sync, webhook tunnels,
notification ingest); those build a `TriggerEnvelope` and call in.

## Pipeline

1. Envelope ([`envelope.rs`](./envelope.rs)): the caller builds a `TriggerEnvelope` via
   `from_composio`, `from_webhook`, `from_cron` or `from_external`, or by
   filling the struct directly ([`desktop/notifications/rpc.rs`](../../desktop/notifications/rpc.rs) does this for
   `TriggerSource::WebviewIntegration`). It carries the `TriggerSource` and
   the raw payload, truncated to 8 KB when rendered into the prompt.
2. Evaluator (`evaluator*.rs`): `run_triage` sends the `trigger_triage`
   agent turn over the native bus (`agent.run_turn`, zero tools, origin
   `AgentTurnOrigin::ExternalChannel`) through a tiered chain: cloud, one
   retry on 429/transient/malformed-reply, then a local-model arm. Budget
   exhaustion and prompt-guard rejections skip the cloud retry and go
   straight to the local arm. If the local arm is unavailable or also fails,
   the result is `TriageOutcome::Deferred { defer_until_ms, reason }` (30 s
   tick) rather than `Err`. Only non-retryable failures (auth, missing
   registry or definition) return `Err`, and those also publish
   `TriggerEscalationFailed`. `decision.rs::parse_triage_decision` parses
   the reply tolerantly (fenced JSON, prose around the object, trailing
   commas, wrong-case action).
3. Escalation ([`escalation.rs`](./escalation.rs)): `apply_decision` publishes
   `TriggerEvaluated` for every action. `drop`/`acknowledge` do no further
   work. `react`/`escalate` first pass `ApprovalGate::intercept_audited` with
   tool key `triage.react` / `triage.escalate`, then build a root
   `ParentExecutionContext` (`parent_context::build_root_parent`)
   and run `trigger_reactor` or `orchestrator` as a direct one-shot agent turn
   under the triage context.
4. Origin ([`origin.rs`](./origin.rs)): every caller scopes an `AgentTurnOrigin` around
   `apply_decision` because `AGENT_TURN_ORIGIN` is a task-local and none of
   the callers inherit one; an unscoped call reads `Unknown` and the gate
   fails closed. `local_trigger_origin` (`Cli`, trust root, no audit row) is
   for triggers the machine generated itself; `remote_trigger_origin`
   (`TrustedAutomation::Workflow { require_approval: true }`) is for anything
   whose payload came from outside.
5. Events ([`events.rs`](./events.rs)): thin wrappers around `DomainEvent::Trigger*`
   (`TriggerEvaluated`, `TriggerEscalated`, `TriggerEscalationFailed`) so the
   field list lives in one place.

[`routing.rs`](./routing.rs) resolves the arms. `resolve_provider_with_config` resolves the
`chat` workload role and forces the managed backend whenever that
role points at a local runtime, a local CLI delegate, or an incomplete BYOK
route, so the initial attempt never depends on a local model being up.
`build_local_provider_with_config` returns the local arm only when
`local_ai.runtime_enabled` is set and a `chat_model_id` is configured;
otherwise the chain skips straight to `Deferred`.

Only the local arm takes the `platform::llm_gate::wait_for_capacity`
permit (held for the duration of that turn); cloud attempts do not wait on
the gate. Tests use a `_for_test` variant that skips the permit.

## Not routed through triage

`TriggerEnvelope::from_cron` still exists, but only the manual RPCs
`agent.triage_evaluate` ([`agent_runs/schemas.rs`](../../agent_runs/schemas.rs)) and `webhooks.trigger_agent`
([`skills/webhooks/ops.rs`](../../skills/webhooks/ops.rs)) build one. The cron scheduler does not call
`run_triage`: `cron/scheduler/agent_run.rs::run_agent_job` runs the job's
agent directly, and `scheduler/delivery.rs` hard-codes
`triage_action: "react"` / `triage_reason: "Scheduled delivery"` on the
delivered notification for display.

## Public surface

Re-exported from [`mod.rs`](./mod.rs):

- `TriggerEnvelope`, `TriggerSource`: `envelope.rs`.
- `TriageAction`, `TriageDecision`, `parse_triage_decision`, `ParseError`:
  [`decision.rs`](./decision.rs).
- `run_triage(&envelope) -> anyhow::Result<TriageOutcome>`, `TriageOutcome`,
  `TriageRun`, `TriageResolutionPath`: `evaluator*.rs`. Also `pub` on the
  module: `TRIGGER_TRIAGE_AGENT_ID`.
- `apply_decision(run, &envelope) -> anyhow::Result<()>`: `escalation.rs`.
- `local_trigger_origin()`, `remote_trigger_origin(&envelope)`: `origin.rs`.
- `build_local_provider_with_config(&config)`,
  `ResolvedProvider`: `routing.rs`. Also `pub` on the module:
  `resolve_provider_with_config`.

## Called by

- [`crates/openhuman-core/src/skills/webhooks/bus.rs`](../../skills/webhooks/bus.rs): incoming webhook
  requests on a tunnel registered to an agent.
- `crates/openhuman-core/src/skills/webhooks/ops.rs`: `webhooks.trigger_agent`
  manual RPC (`webhook` / `cron` / `external` sources).
- `crates/openhuman-core/src/memory/sync/composio/bus*.rs`: Composio
  trigger events; `OPENHUMAN_TRIGGER_TRIAGE_DISABLED`,
  `composio.triage_disabled`, and `composio.triage_disabled_toolkits` skip
  the pipeline.
- [`crates/openhuman-core/src/integrations/task_sources/route.rs`](../../integrations/task_sources/route.rs): proactive
  task-source cards targeting `SourceTarget::AgentTodoProactive`.
- `crates/openhuman-core/src/desktop/notifications/rpc.rs`: background
  triage spawned after `notification_ingest`; score persisted via
  `store::update_triage`, origin `local_trigger_origin`.
- `crates/openhuman-core/src/agent_runs/schemas.rs`: `agent.triage_evaluate` RPC
  (synthetic trigger from any source, optional dry-run without
  `apply_decision`).

## Related

- [`crates/openhuman-core/src/agent_registry/agents/trigger_triage/`](../../agent_registry/agents/trigger_triage/): the
  classifier agent definition; `prompt.md` describes the JSON contract
  `decision.rs` parses.
- [`crates/openhuman-core/src/agent_registry/agents/trigger_reactor/`](../../agent_registry/agents/trigger_reactor/): the
  single-step sub-agent `react` decisions dispatch to.
- [`crates/openhuman-core/src/platform/llm_gate/README.md`](../../platform/llm_gate/README.md): the LLM-permit
  gate the local arm waits on.
- [Parent module README](../README.md)
- [Triggers](../../../../../docs/gitbooks/en/features/integrations/triggers.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)

## Tests

[`envelope_tests.rs`](./envelope_tests.rs), [`decision_tests.rs`](./decision_tests.rs), [`escalation_tests.rs`](./escalation_tests.rs),
[`evaluator_tests.rs`](./evaluator_tests.rs), [`evaluator_deferral_tests.rs`](./evaluator_deferral_tests.rs), [`evaluator_fallback_chain_tests.rs`](./evaluator_fallback_chain_tests.rs), [`events_tests.rs`](./events_tests.rs),
[`origin_tests.rs`](./origin_tests.rs), [`routing_tests.rs`](./routing_tests.rs).
