# Agent tools

Tools that act on the agent loop itself, its todo list, or the user's stored
preferences, rather than on files, memory, or the network. `crate::tools`
re-exports everything here ([`tools/mod.rs`](../../tools/mod.rs) re-exports `crate::agent::tools`),
and `tools::ops` registers them into the tool catalog; nothing outside this
crate should construct them directly.

## Contents

| Tool | File | Wire name (if different) |
| --- | --- | --- |
| `AskClarificationTool` | `tinyagents_harness::tools` | `ask_user_clarification` |
| `DelegateTool` | [`delegate.rs`](./delegate.rs) | `delegate` |
| `PlanExitTool` | [`plan_exit.rs`](./plan_exit.rs) | `plan_exit` |
| `RememberPreferenceTool` | `remember_preference.rs` | `remember_preference` |
| `SavePreferenceTool` | `save_preference.rs` | `save_preference` |
| `RunWorkflowTool`, `AwaitWorkflowTool` | [`run_workflow.rs`](./run_workflow.rs) | `run_workflow`, `await_workflow` |
| `TodoTool` | [`todo.rs`](./todo.rs) | `todo` |

`RunWorkflowTool` and `AwaitWorkflowTool` are compiled in only with the
`skills` feature; a build without it omits both tools from the catalog
instead of registering them as disabled.

## Key types

- `AskClarificationTool` returns the question text as its own tool output; it
  does not pause anything by itself. The pause happens because callers list
  `ask_user_clarification` in the harness seam's `early_exit_tools`
  (`tinyagents::TurnSpec::run`), and a successful call there
  steers the loop to `Pause` instead of feeding the result back to the model.
  Any caller that exposes this tool without that listing gets a tool that
  answers its own question, since the model reads a normal success and keeps
  going.
- `DelegateTool` hands a subtask to a named agent configured with its own
  provider and model, using `DelegateAgentConfig`; `DelegateToolDispatch` is
  the typed harness-side wrapper that supplies live thread and workspace
  context and makes cancellation explicit.
- `PlanExitTool` ends a plan-mode pass by returning the plan text plus
  `PLAN_EXIT_MARKER`. The mode switch itself lives outside this tool; nothing
  in this crate currently consumes the marker.
- `RememberPreferenceTool` pins an explicit `(class, key, value)` preference
  into the `user_profile` memory namespace. `SavePreferenceTool` stores a
  free-form preference into either the `general` or `situational` lane.
  `RunWorkflowTool` and `AwaitWorkflowTool` spawn a `crate::skills::runtime`
  workflow run and wait on its outcome.
- `TodoTool` is the model-facing side of the session's todo list; storage and
  status transitions belong to `crate::agent::todos`, which wraps the
  TinyAgents todo store.

## Where next

- `crate::tools` for the shared `Tool` trait, `ToolSpec`, and how these tools
  join the rest of the catalog.
- `crate::agent::orchestration` for the sub-agent spawn tools, which sit next
  to these conceptually but live separately because they carry more
  lifecycle state.
- `crate::agent::todos` for the store `TodoTool` reads and writes.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Native tools](../../../../../docs/gitbooks/en/features/native-tools/README.md)
