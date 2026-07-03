# 08.5 — Worktrees behind `WorkspaceIsolation`

The crate has the isolation seam the sdk-gaps doc asked for:
`WorkspaceIsolation` trait, `WorkspaceDescriptor { root, trusted_roots,
policy_id, sandbox }` (+ `allows(path)`), `ToolExecutionContext.workspace`,
and `WorkspacePrepared/Violation/Cleanup` events.

Current status (2026-07-02): do not delete
`src/openhuman/agent/harness/worktree_context.rs` yet. `spawn_parallel_agents`
still threads `worktree_action_dir` into the sub-agent runner as a fallback,
async subagent session roots now prefer `ToolExecutionContext.workspace`, and
shell/git acting tools read the task-local `current_action_dir_override()` to
execute inside the isolated checkout when no descriptor is present. The module
is now hidden behind crate-internal harness re-exports. Its task-local carrier
and accessor helpers are no longer public beyond the crate.
`agent_orchestration::worktree::GitWorktreeIsolation` now implements the
TinyAgents `WorkspaceIsolation` trait over OpenHuman's existing git-worktree
create/remove policy and returns `WorkspaceDescriptor` roots for isolated
workers. Sub-agent run options now carry an optional `WorkspaceDescriptor`, and
the TinyAgents shared turn seam attaches it to `RunContext::with_workspace`.
For current `spawn_parallel_agents` worktree workers, dispatch now calls
`GitWorktreeIsolation::prepare` and carries the returned `WorkspaceDescriptor`
into `SubagentRunOptions`; the old `worktree_action_dir` task-local remains as
the live fallback until every acting tool reads the descriptor directly.
`spawn_parallel_agents` also reads the parent `ToolExecutionContext.workspace`
at the tool boundary, so shared workers inherit the caller workspace root and
isolated fanout workers prepare relative to that root before falling back to
`Config.action_dir`. `spawn_subagent` now preserves the same descriptor when it
routes to reusable async delegation and when a blocking sub-agent run is
requested, and `continue_subagent` passes the current caller descriptor into
resumed checkpoint runs. `spawn_worker_thread` does the same for persisted
worker-thread sub-agent runs. The archetype and integrations delegation tools
now also forward the descriptor through their shared sub-agent dispatcher, and
`agent_prepare_context` passes it into the read-only context scout when invoked
as a tool. `call_memory_agent` and `delegate_to_personality` likewise carry the
descriptor into their spawned sub-agents.
The TinyAgents tool adapter now forwards `ToolExecutionContext` into OpenHuman
tools, and shell/git plus core filesystem tools (`file_read`, `list`,
`file_write`, `edit`, `apply_patch`, `grep`, `glob`, `csv_export`) and
shell-family runtime tools (`node_exec`, `npm_exec`) use
`ToolExecutionContext.workspace.root` as their effective action directory when
present. Generated media tools (`media_generate_image`, `media_generate_video`)
also persist artifacts under that descriptor root for isolated fanout workers.
Codegraph tools (`codegraph_index`, `codegraph_search`) likewise resolve their
repo boundary and index store from the descriptor root during TinyAgents runs.
For shell and the runtime tools this also covers sandbox policy and
sandbox cwd. Delete the legacy task-local only after the remaining acting tools
also resolve roots from the carried crate `WorkspaceDescriptor`.

## Steps

1. Partially done: implement `WorkspaceIsolation` over
   `agent_orchestration/worktree.rs` (git-worktree create/cleanup, dirty-
   worktree safeguards stay OpenHuman policy). `spawn_parallel_agents` now uses
   the adapter's prepare path and passes the descriptor into the runner;
   cleanup remains a separate dirty-worktree policy slice.
2. In progress: thread `WorkspaceDescriptor` into tool execution. TinyAgents
   owns the descriptor carrier, while acting tools resolve their allowed root
   from `ToolExecutionContext.workspace` instead of task-local
   `worktree_context.rs`/action-dir globals. The carrier is now threaded
   through sub-agent run options, `RunContext::with_workspace`, the OpenHuman
   tool adapter, `spawn_subagent`, `continue_subagent`, `spawn_worker_thread`,
   archetype/integrations delegation dispatch, `spawn_async_subagent`
   reuse/session roots, `spawn_parallel_agents` shared-worker roots,
   `agent_prepare_context`, `call_memory_agent`, `delegate_to_personality`,
   `delegate_graph`, shell, git operations, `file_read`, `list`, `file_write`,
   `edit`, `apply_patch`, `grep`, `glob`, `csv_export`, `read_diff`,
   `run_linter`, `run_tests`, `update_memory_md`, `node_exec`, `npm_exec`,
   `media_generate_image`, `media_generate_video`, `codegraph_index`, and
   `codegraph_search`.
   Remaining acting tools still need to read it. OpenHuman
   `SecurityPolicy` remains the enforcement authority — the descriptor is the
   carrier, not the policy.
   `ParentExecutionContext` now also carries the descriptor as an ambient
   fallback for internal/background fanout: the sub-agent runner scopes it into
   child turns, and `AgentOrchestrationSession::spawn_agent` inherits it while
   still letting explicit per-spawn descriptors win. Parent-context snapshots now
   preserve an ambient descriptor when one is active, and post-turn session
   memory extraction reuses the turn's parent snapshot instead of rebuilding a
   default-only context after the scope has ended.
3. Emit `WorkspacePrepared/Violation/Cleanup` through the bridge; violations
   also feed the security audit trail. Use 1.3.0
   `WorkspaceDescriptor::enforce(path, events)` so the check and the
   violation event are one call.
4. Sandbox descriptor: map `sandbox_mode` onto the descriptor's sandbox
   field so sandboxed runs are inspectable.

## Deletions

- Later: `harness/worktree_context.rs` (74) task-local once shell/git and other
  acting tools read the descriptor; per-tool ad hoc root plumbing.

## Acceptance

- Parallel edit-capable workers run in isolated worktrees with descriptor-
  scoped tool roots; a path outside `allows()` is blocked AND evented;
  worktree tests (235) green.
