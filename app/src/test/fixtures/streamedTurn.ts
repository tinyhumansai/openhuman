/**
 * One scripted assistant turn, told twice: as the socket events the live
 * thread receives, and as the core's derived transcript a reopened thread
 * reads (`threads_transcript_get`).
 *
 * The two tellings describe the SAME turn, and the parity tests hold the live
 * projection to the history projection. That pairing is the regression guard:
 * a change that makes live streaming and a reopened thread render differently —
 * a part in another order, narration shown on one side only, a delegation in a
 * different slot — breaks a test instead of shipping as "the live thread
 * glitches".
 *
 * The turn deliberately covers every shape that used to diverge:
 *
 * - reasoning, then narration, then tools, in round 1;
 * - a tool whose args stream (`tool_args_delta`) BEFORE its `tool_call`;
 * - a tool call with no provider id;
 * - a `chat_interim` closing round 1;
 * - a delegation (`spawn_subagent` → `subagent_spawned` → `subagent_done`);
 * - a final round whose text is the answer.
 */
import type { ChatEventListeners } from '../../services/chatService';
import type { DerivedDisplayItem } from '../../types/derivedTranscript';

export const TURN_THREAD = 't-parity';
export const TURN_REQUEST = 'req-parity';

export const ROUND1_THINKING = 'I need the calendar first.';
export const ROUND1_NARRATION = 'Let me check your calendar.';
export const ROUND2_THINKING = 'The research should be delegated.';
export const FINAL_ANSWER = 'You have two meetings today, and the research is done.';

type ListenerName = keyof ChatEventListeners;
export type SocketStep = {
  [K in ListenerName]: { listener: K; event: Parameters<NonNullable<ChatEventListeners[K]>>[0] };
}[ListenerName];

const base = { thread_id: TURN_THREAD, request_id: TURN_REQUEST };

/** Split text into small deltas, the way tokens arrive. */
function deltas(
  listener: 'onTextDelta' | 'onThinkingDelta',
  round: number,
  text: string
): SocketStep[] {
  const out: SocketStep[] = [];
  for (let at = 0; at < text.length; at += 6) {
    out.push({ listener, event: { ...base, round, delta: text.slice(at, at + 6) } });
  }
  return out;
}

/** Everything up to (not including) `chat_done`. */
export const LIVE_TURN_STEPS: SocketStep[] = [
  { listener: 'onInferenceStart', event: { ...base } },
  ...deltas('onThinkingDelta', 1, ROUND1_THINKING),
  ...deltas('onTextDelta', 1, ROUND1_NARRATION),
  // Args stream before the call is announced — the row is minted by the delta.
  {
    listener: 'onToolArgsDelta',
    event: {
      ...base,
      round: 1,
      tool_call_id: 'call-calendar',
      tool_name: 'calendar_list',
      delta: '{"day":',
    },
  },
  {
    listener: 'onToolArgsDelta',
    event: {
      ...base,
      round: 1,
      tool_call_id: 'call-calendar',
      tool_name: 'calendar_list',
      delta: '"today"}',
    },
  },
  {
    listener: 'onToolCall',
    event: {
      ...base,
      round: 1,
      tool_name: 'calendar_list',
      skill_id: 'calendar',
      args: { day: 'today' },
      tool_call_id: 'call-calendar',
    },
  },
  // No provider id at all.
  {
    listener: 'onToolCall',
    event: { ...base, round: 1, tool_name: 'web_search', skill_id: 'web', args: { q: 'agenda' } },
  },
  {
    listener: 'onToolResult',
    event: {
      ...base,
      round: 1,
      tool_name: 'calendar_list',
      skill_id: 'calendar',
      output: '2 meetings',
      success: true,
      tool_call_id: 'call-calendar',
    },
  },
  {
    listener: 'onToolResult',
    event: {
      ...base,
      round: 1,
      tool_name: 'web_search',
      skill_id: 'web',
      output: 'results',
      success: true,
    },
  },
  { listener: 'onInterim', event: { ...base, round: 1, full_response: ROUND1_NARRATION } },
  ...deltas('onThinkingDelta', 2, ROUND2_THINKING),
  {
    listener: 'onToolCall',
    event: {
      ...base,
      round: 2,
      tool_name: 'spawn_subagent',
      skill_id: 'agents',
      args: { agent_id: 'researcher', prompt: 'Research the attendees' },
      tool_call_id: 'call-spawn',
    },
  },
  {
    listener: 'onSubagentSpawned',
    event: { ...base, round: 2, tool_name: 'researcher', skill_id: 'sub-1', message: '', seq: 1 },
  },
  {
    listener: 'onSubagentDone',
    event: {
      ...base,
      round: 2,
      tool_name: 'researcher',
      skill_id: 'sub-1',
      message: '',
      success: true,
      seq: 2,
    },
  },
  ...deltas('onTextDelta', 3, FINAL_ANSWER),
];

export const DONE_EVENT = {
  ...base,
  full_response: FINAL_ANSWER,
  total_input_tokens: 10,
  total_output_tokens: 10,
};

/** The same turn as the core projects it on reload, NEWEST-first (wire order). */
export const HISTORY_ITEMS: DerivedDisplayItem[] = (
  [
    { kind: 'turnBoundary', requestId: TURN_REQUEST },
    { kind: 'userMessage', content: 'What is on today?', requestId: TURN_REQUEST },
    { kind: 'reasoning', text: ROUND1_THINKING },
    {
      kind: 'assistantMessage',
      content: ROUND1_NARRATION,
      interim: true,
      requestId: TURN_REQUEST,
      iteration: 1,
    },
    {
      kind: 'toolCall',
      callId: 'call-calendar',
      name: 'calendar_list',
      args: { day: 'today' },
      result: '2 meetings',
      status: 'success',
    },
    {
      kind: 'toolCall',
      callId: '',
      name: 'web_search',
      args: { q: 'agenda' },
      result: 'results',
      status: 'success',
    },
    { kind: 'reasoning', text: ROUND2_THINKING },
    {
      kind: 'toolCall',
      callId: 'call-spawn',
      name: 'spawn_subagent',
      args: { agent_id: 'researcher', prompt: 'Research the attendees' },
      result: 'done',
      status: 'success',
    },
    { kind: 'assistantMessage', content: FINAL_ANSWER, requestId: TURN_REQUEST, iteration: 3 },
    // Sub-agents are projected after every root item, with no link back to the
    // call that spawned them.
    {
      kind: 'subagent',
      id: 'researcher',
      requestId: TURN_REQUEST,
      items: [{ kind: 'assistantMessage', content: 'Attendees researched.' }],
    },
  ] satisfies DerivedDisplayItem[]
).reverse();
