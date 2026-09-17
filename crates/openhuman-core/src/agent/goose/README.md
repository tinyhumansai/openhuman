# Goose primary-turn adapter

This directory is OpenHuman's host boundary around the pinned Apache-2.0
`goose_agent::machine::StateMachine`. It does not select primary-turn modes or
capability routes; those are later Phase 7 gates. The existing TinyAgents path
remains the live default.

- `runner.rs` assembles Goose operations and calls `StateMachine::run`.
- `convert.rs` mechanically translates OpenHuman transcript records, Goose
  conversation messages, RMCP tool declarations/results, and TinyInference
  provider messages.
- `inference.rs` invokes OpenHuman's existing `ChatModel` and projects per-call
  and cumulative usage into `AgentProgress`.
- `tools.rs` dispatches the already-authorized tool list through OpenHuman's
  security/approval gate and existing tool executor.
- `store.rs` is the strict checkpoint facade. Each machine step uses one
  optimistic commit. A tool action enters the durable action index with the
  assistant request before execution; one matching observation completes it.
  Duplicate or orphaned observations fail the commit. A separate durable
  execution claim closes the crash window: a resume may record an uncertain
  failure, but it never repeats an already-claimed side effect.

Cancellation is another Goose operation. When a persisted action is pending,
it appends one matching interrupted observation and yields without executing
the tool. Approval continues to use OpenHuman's existing asynchronous approval
gate: while the gate waits for the UI decision, the accepted action is already
durable and tool execution has not begun.
