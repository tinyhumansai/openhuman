# 07.3 — Steering surface + one recursion authority

## Steering

Current status (2026-07-02): `harness/run_queue/` is still live. OpenHuman
drains its own `Steer`/`Collect` lanes and forwards them as TinyAgents
`SteeringCommand::InjectMessage`; TinyAgents drains those `SteeringHandle`
commands at loop checkpoints. The queue is also the product adapter for
web-channel `followup` and `parallel` semantics and for detached sub-agent
steer/collect lookup through `running_subagents`. The local
`openhuman::tinyagents::orchestration` seam now re-exports `SteeringRegistry`,
`TaskId`, `SteeringCommand`, `SteeringPolicy`, and `SteeringHandle`, and
sub-agent TinyAgents runs register their live `SteeringHandle` in a
process-local `SteeringRegistry` while the run is active. Detached
`steer`/`collect` controls now prefer that registry handle and fall back to
`RunQueue` when no handle is available. The shared OpenHuman turn seam installs
a local steering policy that accepts only `InjectMessage` and `Pause` until a
product control surface owns additional crate command kinds. Do not delete the
directory until the remaining detached control modes and the web-channel
followup/parallel lanes either have TinyAgents-owned equivalents or move into
local owners. Hard cancel/prune paths also deregister task handles so aborted
runs do not leave stale registry entries.
The unused `DomainEvent::RunQueueMessageDelivered` projection has been removed;
queued/interrupt/followup events remain live.

1. Map product tools onto crate commands: `steer_subagent` →
   `SteeringCommand::InjectMessage` via the registered `SteeringRegistry` is
   live for active TinyAgents sub-agent runs, with `RunQueue` fallback. Next:
   map redirect/pause/resume/cancel modes to corresponding variants while
   preserving delivery-at-safe-boundaries semantics (crate drains before each
   model call).
2. Tighten policy by run class: the shared turn seam now installs an
   `InjectMessage`/`Pause` allowlist; future background-only runs can accept
   Cancel without also accepting transcript injection.
3. Accepted/rejected steering emits `AgentEvent::Steered`; the bridge now logs
   accepted/rejected command kinds, and UI projection remains pending. Delete
   bespoke acknowledgment plumbing in `run_queue/` after parity.
4. `harness/run_queue/` (317 total; 174 non-test): first split it by ownership.
   Detached sub-agent `Steer`/`Collect` should collapse to a `SteeringRegistry`
   lookup/registration path; web-channel `Followup`/`Parallel` remain product
   turn orchestration unless a crate-owned replacement is introduced.

## Recursion

5. One cap: crate `RunLimits.max_depth`/`RecursionPolicy` becomes
   authoritative; crate-internal `spawn_depth_context.rs` (66) becomes a
   reader/projector of crate depth (`ToolExecutionContext.depth`) for product
   error wording, or is deleted if the wording can wrap
   `TinyAgentsError::SubAgentDepth`. Current code now threads
   `MAX_SPAWN_DEPTH = 3` into TinyAgents `RunPolicy.limits.max_depth`,
   `RunConfig.max_depth`, and MCP `agent.run_subagent` via
   `mcp_server/subagent_depth.rs`, but OpenHuman's own
   `spawn_depth_context` still rejects beyond the same cap before the
   TinyAgents run.
6. One error shape: `SubAgentDepth`/`RecursionLimit` now preserve the existing
   `SpawnDepthExceeded` error surface before the subagent graph wraps provider
   failures.

## Deletions

- `harness/run_queue/` queue mechanics, only after detached steer/collect no
  longer use `Arc<RunQueue>` and web-channel followup/parallel behavior has a
  replacement or local owner.
- crate-internal `harness/spawn_depth_context.rs` (if step 5 wording-wrap
  suffices).

## Acceptance

- Mid-flight steer lands only at loop boundaries (existing steering tests);
  depth-exceeded produces the same user-facing error as today.
