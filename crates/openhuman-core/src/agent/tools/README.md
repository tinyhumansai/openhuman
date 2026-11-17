# Agent tools

Tools that act on the agent loop itself, its todo list, or the user's stored
preferences, rather than on files, memory, or the network. `crate::tools`
re-exports everything here ([`tools/mod.rs`](../../tools/mod.rs) re-exports `crate::agent::tools`),
and `tools::ops` registers them into the tool catalog; nothing outside this
crate should construct them directly.

## Contents

| Tool | File | Wire name (if different) |
| --- | --- | --- |
| `AskClarificationTool` | `tinyagents_harness_tools` | `ask_user_clarification` |
| `plan_exit_tool` | [`plan_exit.rs`](./plan_exit.rs) | `plan_exit` |
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
- `plan_exit_tool` configures TinyAgents' plan-exit tool with OpenHuman's
  thread mode and pending-review stores. TinyAgents owns the tool behavior and
  `PLAN_EXIT_MARKER`.
- `RememberPreferenceTool` pins an explicit `(class, key, value)` preference
  into the `user_profile` memory namespace. `SavePreferenceTool` stores a
  free-form preference into either the `general` or `situational` lane.
  `RunWorkflowTool` and `AwaitWorkflowTool` spawn a `crate::skills::runtime`
  workflow run and wait on its outcome.
- `TodoTool` is the model-facing side of the session's todo list; TinyAgents
  owns its schema and store operations, while the host adapter selects the
  thread key and publishes the product event.

## Where next

- `crate::tools` for the shared `Tool` trait, `ToolSpec`, and how these tools
  join the rest of the catalog.
- `crate::agent::orchestration` for the sub-agent spawn tools, which sit next
  to these conceptually but live separately because they carry more
  lifecycle state.
- `tinyagents_harness_tools::todos` for the todo store and tool behavior.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Native tools](../../../../../docs/gitbooks/en/features/native-tools/README.md)
