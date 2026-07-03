# 02.3 — Native reasoning + tool-arg streaming

tinyagents 1.3.0 `MessageDelta` has a `reasoning` field and
`StreamAccumulator::reasoning()`; `ModelStreamItem::ToolCallDelta` streams
tool-arg fragments. The out-of-band `ThinkingForwarder` is not one-shot
deletable: streaming reasoning can ride the crate stream, but OpenHuman still
uses the forwarder for non-streaming post-hoc reasoning and tool-argument
progress that preserves the UI's `tool_name`/start-event contract.

Current status (2026-07-02): streaming provider `ThinkingDelta` values are
mapped to `MessageDelta::reasoning(...)`, and `OpenhumanEventBridge` projects
`delta.reasoning` into the same parent/sub-agent thinking progress events as the
old forwarder path. Tool-call **argument** fragments now ride the crate stream
too: `forward_delta` maps each `ProviderDelta::ToolCallArgsDelta` onto a
`ModelStreamItem::ToolCallDelta(ToolDelta { call_id, content })`, the crate
agent-loop surfaces it as `MessageDelta.tool_call`, and `OpenhumanEventBridge`
projects it into `AgentProgress::ToolCallArgsDelta`. Because the crate
`ToolDelta` has only `call_id`/`content` (no `tool_name`), the tool-call
**start** event — the empty-delta `ToolCallArgsDelta` that carries the tool name
and opens the UI timeline row — still rides `ThinkingForwarder.note_tool_call`,
which records the name into a `call_id → tool_name` map (`ToolNameMap`) *shared*
with the bridge so the streamed fragments stay labelled. The forwarder remains
live for that start marker and for non-streaming post-hoc reasoning; its
`emit_tool_args` argument-fragment path has been removed. The `StreamAccumulator`
treats the terminal `Completed` response as authoritative, so streaming the
fragments never disturbs the final native tool calls.

## Steps

1. Done for streaming providers: `src/openhuman/tinyagents/model.rs` maps
   provider thinking deltas to `MessageDelta::reasoning(...)` on the crate
   stream, and `OpenhumanEventBridge` (`observability.rs`) projects
   `delta.reasoning` into OpenHuman progress events.
2. Done: provider tool-call argument fragments map onto
   `ModelStreamItem::ToolCallDelta` (crate `ToolDelta { call_id, content }`) and
   are projected by `OpenhumanEventBridge`. `ToolDelta` carries no `tool_name`,
   so the `tool_name`/start-event half of the UI timeline contract stays on
   `ThinkingForwarder.note_tool_call`, which shares a `call_id → tool_name` map
   (`ToolNameMap`) with the bridge so the streamed fragments keep their label.
   Only `ThinkingForwarder.emit_tool_args` was removed.
3. Verify sub-agent child thinking deltas still reach the scope-aware bridge
   (parity with the current forwarder behavior).

## Deletions

- Reasoning side of `ThinkingForwarder`: moved for streaming providers; keep the
  non-streaming fallback until the non-streaming path has an equivalent event.
- Argument-fragment side of `ThinkingForwarder` (`emit_tool_args`, in
  `tinyagents/model.rs`): removed — fragments ride `ToolCallDelta` and are
  projected by the bridge. The tool-call **start** marker
  (`note_tool_call`, empty-delta `ToolCallArgsDelta` with `tool_name`) stays,
  since the crate `ToolDelta` cannot carry a tool name; it will only move if a
  future crate revision adds `tool_name`/start semantics to `ToolDelta`.

## Acceptance

- UI receives visible-text, reasoning, and tool-arg deltas from crate stream
  items alone, including `tool_name`/start-event semantics; streaming e2e (chat
  - sub-agent) green.
- Non-streaming providers emit post-hoc reasoning once.
