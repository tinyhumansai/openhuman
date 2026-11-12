import { describe, expect, it, vi } from 'vitest';

import type { ToolTimelineEntry } from '../../store/chatRuntimeSlice';
import { CHAT_ERROR_METADATA_KEY } from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import {
  buildRuntimeMessages,
  STREAMING_TAIL_ID,
  streamingMessageId,
  streamingTailMessage,
  toThreadMessageLike,
} from '../assistantUiMessages';

function msg(over: Partial<ThreadMessage> = {}): ThreadMessage {
  return {
    id: 'm1',
    content: 'hello',
    type: 'text',
    extraMetadata: {},
    sender: 'user',
    createdAt: '2026-01-01T00:00:00.000Z',
    ...over,
  };
}

// Every tool row renders on the main surface now, so this default is no longer
// load-bearing the way it was while a read-only allowlist could drop a row and
// quietly make the identity / status / naming assertions below vacuous. Left as
// `file_write` so those assertions keep naming a concrete tool rather than a
// category boundary.
function tool(over: Partial<ToolTimelineEntry> = {}): ToolTimelineEntry {
  return { id: 'call-1', name: 'file_write', round: 1, seq: 0, status: 'running', ...over };
}

describe('toThreadMessageLike', () => {
  it.each([
    '',
    '[IMAGE:data:image/png;base64,old]',
    `[ATTACHMENT:${encodeURIComponent(
      JSON.stringify({
        path: 'uploads/code.py',
        name: 'code.py',
        mime: 'text/plain',
        size_bytes: 1,
      })
    )}]`,
  ])('preserves user code and Markdown whitespace with attachment %s', attachment => {
    const text = '```python\nif ready:\n    print("two  spaces")\n```\nfirst line  \nsecond line';
    const content = attachment ? `${text} ${attachment}` : text;

    expect(toThreadMessageLike(msg({ content })).content[0]).toEqual({ type: 'text', text });
  });

  it('renders durable uploads as file chips without raw reference JSON or inline bytes', () => {
    const references = [
      { path: 'uploads/t/id/scan.png', name: 'scan.png', mime: 'image/png', size_bytes: 3 },
      {
        path: 'uploads/t/id/archive.zip',
        name: 'archive.zip',
        mime: 'application/zip',
        size_bytes: 8,
      },
    ]
      .map(file => `[ATTACHMENT:${encodeURIComponent(JSON.stringify(file))}]`)
      .join(' ');
    expect(toThreadMessageLike(msg({ content: `inspect ${references}` })).content).toEqual([
      { type: 'text', text: 'inspect' },
      { type: 'file', filename: 'scan.png', data: '', mimeType: 'image/png' },
      { type: 'file', filename: 'archive.zip', data: '', mimeType: 'application/zip' },
    ]);
  });

  it('keeps malformed or unsafe durable markers as ordinary text', () => {
    for (const path of ['../escape', '/etc/passwd', 'C:\\secret', 'uploads/../secret']) {
      const content = `[ATTACHMENT:${encodeURIComponent(JSON.stringify({ path, name: 'file', mime: 'image/png', size_bytes: 1 }))}]`;
      expect(toThreadMessageLike(msg({ content })).content).toEqual([
        { type: 'text', text: content },
      ]);
    }
    const content = '[ATTACHMENT:%invalid]';
    expect(toThreadMessageLike(msg({ content })).content).toEqual([
      { type: 'text', text: content },
    ]);
  });

  it('continues to render legacy inline image previews', () => {
    expect(
      toThreadMessageLike(msg({ content: 'photo [IMAGE:data:image/png;base64,old]' })).content
    ).toEqual([
      { type: 'text', text: 'photo' },
      { type: 'image', image: 'data:image/png;base64,old', filename: undefined },
    ]);
  });

  it('maps sender to role', () => {
    expect(toThreadMessageLike(msg({ id: 'u' })).role).toBe('user');
    expect(toThreadMessageLike(msg({ id: 'a', sender: 'agent' })).role).toBe('assistant');
  });

  it('unwraps a tool-call envelope so raw JSON never reaches the runtime', () => {
    const envelope = JSON.stringify({
      content: 'Pulling that up now.',
      tool_calls: [{ id: 'c1', name: 'memory_search', arguments: '{}' }],
    });
    const converted = toThreadMessageLike(msg({ id: 'e', sender: 'agent', content: envelope }));
    expect(converted.content).toEqual([{ type: 'text', text: 'Pulling that up now.' }]);
  });

  it('leaves ordinary prose untouched', () => {
    const m = msg({ id: 'p', sender: 'agent', content: 'just prose { not json' });
    expect(toThreadMessageLike(m).content).toEqual([
      { type: 'text', text: 'just prose { not json' },
    ]);
  });

  it('yields an empty content array for an empty message', () => {
    expect(toThreadMessageLike(msg({ id: 'blank', content: '' })).content).toEqual([]);
  });

  it('carries extraMetadata through as custom metadata', () => {
    const m = msg({ id: 'meta', extraMetadata: { requestId: 'r1' } });
    expect(toThreadMessageLike(m).metadata?.custom).toMatchObject({
      extraMetadata: { requestId: 'r1' },
    });
  });

  it('shows a failed turn through assistant-ui error status without raw link markup', () => {
    const content =
      'Something went wrong. Please try again.\n<openhuman-link path="community/discord-report">Report on Discord</openhuman-link>\n\n> Provider detail';
    const converted = toThreadMessageLike(
      msg({
        id: 'failed-turn',
        sender: 'agent',
        content,
        extraMetadata: { [CHAT_ERROR_METADATA_KEY]: { errorType: 'inference' } },
      })
    );

    expect(converted.content).toEqual([]);
    expect(converted.status).toEqual({
      type: 'incomplete',
      reason: 'error',
      error: 'Something went wrong. Please try again.\n\n> Provider detail',
    });
  });

  it('returns the identical object for the same source message', () => {
    const m = msg({ id: 'cached' });
    expect(toThreadMessageLike(m)).toBe(toThreadMessageLike(m));
  });
});

describe('streamingTailMessage', () => {
  it('is null with no stream and with an empty stream', () => {
    expect(streamingTailMessage(null)).toBeNull();
    expect(streamingTailMessage({ requestId: 'r', content: '', thinking: '' })).toBeNull();
  });

  it('is a running assistant message when tokens have landed', () => {
    const tail = streamingTailMessage({ requestId: 'r', content: 'partial', thinking: '' });
    expect(tail).toMatchObject({
      id: streamingMessageId('r'),
      role: 'assistant',
      status: { type: 'running' },
      content: [{ type: 'text', text: 'partial' }],
    });
  });

  it('renders streamed thinking as a reasoning part above the answer', () => {
    // The disclosure is collapsed once settled and held open while it streams,
    // so this is one quiet line rather than the prose firehose that got the
    // block removed in the first place. It goes FIRST: it is what the agent
    // thought before it answered.
    const tail = streamingTailMessage({ requestId: 'r', content: 'answer', thinking: 'reasoning' });
    expect(tail?.content).toEqual([
      { type: 'reasoning', text: 'reasoning' },
      { type: 'text', text: 'answer' },
    ]);
  });

  it('does not double the reasoning once the transcript has recorded it', () => {
    // `assistantParts` already emits the transcript's `thinking` item in its
    // proper place; the live unshift must stand down or the block renders twice.
    const tail = streamingTailMessage(
      { requestId: 'r', content: 'answer', thinking: 'recorded' },
      [],
      [{ kind: 'thinking', round: 1, seq: 0, text: 'recorded' }]
    );
    expect(tail?.content).toEqual([
      { type: 'reasoning', text: 'recorded' },
      { type: 'text', text: 'answer' },
    ]);
  });

  it('mints a reasoning-only tail for a turn that has so far only thought', () => {
    // There IS something to paint now: the live reasoning block, expanded while
    // it streams. Before the block came back this minted nothing at all and the
    // status line carried the whole burden of showing the turn was alive.
    const tail = streamingTailMessage({ requestId: 'r', content: '', thinking: 'still working' });
    expect(tail).toMatchObject({
      id: streamingMessageId('r'),
      status: { type: 'running' },
      content: [{ type: 'reasoning', text: 'still working' }],
    });
  });

  it('keeps a running delegation on args and adds result only when complete', () => {
    const subagent = {
      taskId: 'sub-1',
      agentId: 'researcher',
      toolCalls: [],
      transcript: [{ kind: 'thinking' as const, text: 'checking sources' }],
    };
    const running = streamingTailMessage(null, [
      tool({ id: 'sub-1', name: 'subagent:researcher', subagent }),
    ]);
    const runningPart = running?.content[0];
    expect(runningPart).toMatchObject({
      type: 'tool-call',
      toolName: 'task',
      args: { progress: subagent },
    });
    expect(runningPart).not.toHaveProperty('result');

    const complete = streamingTailMessage(null, [
      tool({ id: 'sub-1', name: 'subagent:researcher', status: 'success', subagent }),
    ]);
    // `result` is `{status, activity}`, not the bare activity: the outer row's
    // OWN `entry.status` is what settles reliably (`subagentDone` never
    // touches `activity.status` itself), so `SubagentTaskCard` reads that
    // rather than the activity's possibly-stale `status` field.
    expect(complete?.content[0]).toMatchObject({
      type: 'tool-call',
      toolName: 'task',
      result: { status: 'success', activity: subagent },
    });
  });
});

describe('buildRuntimeMessages', () => {
  it('omits hidden messages', () => {
    const visible = msg({ id: 'v' });
    const hidden = msg({ id: 'h', extraMetadata: { hidden: true } });
    expect(buildRuntimeMessages([visible, hidden], null).map(m => m.id)).toEqual(['v']);
  });

  it('appends the live tail after the settled transcript', () => {
    const ids = buildRuntimeMessages([msg({ id: 'a' })], {
      requestId: 'r',
      content: 'tok',
      thinking: '',
    }).map(m => m.id);
    expect(ids).toEqual(['a', streamingMessageId('r')]);
  });

  it('does not keep a synthetic thinking/tool tail running after lifecycle completion', () => {
    const projected = buildRuntimeMessages([msg({ id: 'answer', sender: 'agent' })], null, {
      isRunning: false,
      liveTimeline: [tool({ id: 'stale-tool', status: 'success' })],
      liveTranscript: [{ kind: 'thinking', round: 1, seq: 0, text: 'already finished thinking' }],
    });
    const ids = projected.map(message => message.id);

    expect(ids).toEqual(['answer']);
    expect(ids).not.toContain(STREAMING_TAIL_ID);
    expect(projected[0]?.content).toEqual([
      { type: 'reasoning', text: 'already finished thinking' },
      expect.objectContaining({ type: 'tool-call', toolCallId: 'stale-tool' }),
      { type: 'text', text: 'hello' },
    ]);
  });

  it("hands the live rows to the settled reply they belong to, never to another request's", () => {
    const reply = (requestId: string) =>
      msg({ id: `agent:${requestId}`, sender: 'agent', extraMetadata: { requestId } });
    const liveTimeline = [tool({ id: 'call-1', status: 'success' })];
    const toolIds = (liveTimelineRequestId: string | undefined, requestId: string) => {
      const [message] = buildRuntimeMessages([reply(requestId)], null, {
        isRunning: false,
        liveTimeline,
        liveTimelineRequestId,
      });
      return typeof message.content === 'string'
        ? []
        : message.content.flatMap(part => (part.type === 'tool-call' ? [part.toolCallId] : []));
    };
    // The just-settled turn's own rows, and rows of unknown owner: unchanged.
    expect(toolIds('req-1', 'req-1')).toEqual(['call-1']);
    expect(toolIds(undefined, 'req-1')).toEqual(['call-1']);
    // A reply that settled without an `inference_start` (a background
    // delivery) does not show the previous turn's cards again.
    expect(toolIds('req-1', 'bgdeliver-1')).toEqual([]);
  });

  it('replays a settled turn with its reasoning, narration and tools in the order they happened', () => {
    const answer = msg({
      id: 'answer',
      sender: 'agent',
      content: 'finished',
      extraMetadata: { requestId: 'req-1' },
    });
    const timeline = [tool({ id: 'call-1', status: 'success', result: 'found it' })];
    const transcript = [
      { kind: 'thinking' as const, round: 1, seq: 0, text: 'need to search' },
      { kind: 'narration' as const, round: 1, seq: 1, text: 'I will check the sources.' },
      { kind: 'toolCall' as const, round: 1, seq: 2, callId: 'call-1' },
    ];

    expect(
      buildRuntimeMessages([answer], null, {
        turnTimelines: { 'req-1': timeline },
        turnTranscripts: { 'req-1': transcript },
      })[0]?.content
    ).toEqual([
      // Everything comes back inline, in the transcript's own order. Narration
      // before a tool call is what the live turn showed while it streamed, so
      // a reload shows it too (the answer is not narration: it closes the turn).
      { type: 'reasoning', text: 'need to search' },
      { type: 'text', text: 'I will check the sources.' },
      expect.objectContaining({
        type: 'tool-call',
        toolCallId: 'call-1',
        toolName: 'file_write',
        result: 'found it',
      }),
      { type: 'text', text: 'finished' },
    ]);
  });

  it('settles a frozen trail’s running row from the core projection, keeping its id', () => {
    const answer = msg({
      id: 'answer',
      sender: 'agent',
      content: 'Done.',
      extraMetadata: { requestId: 'req-f' },
    });
    const frozenTimeline = [tool({ id: 'call-1', status: 'running' })];
    const frozenTranscript = [{ kind: 'toolCall' as const, round: 1, seq: 0, callId: 'call-1' }];
    const build = (settledRows?: ReturnType<typeof tool>[]) =>
      buildRuntimeMessages([answer], null, {
        isRunning: false,
        settledTurns: { 'req-f': { timeline: frozenTimeline, transcript: frozenTranscript } },
        ...(settledRows ? { turnTimelines: { 'req-f': settledRows } } : {}),
      })[0]?.content;

    const before = build();
    const toolBefore = Array.isArray(before) ? before[0] : undefined;
    expect(toolBefore).toMatchObject({ toolCallId: 'call-1' });
    expect(toolBefore && 'result' in toolBefore ? toolBefore.result : undefined).toBeUndefined();

    const after = build([tool({ id: 'call-1', status: 'cancelled' })]);
    const toolAfter = Array.isArray(after) ? after[0] : undefined;
    expect(toolAfter).toMatchObject({
      toolCallId: 'call-1',
      result: expect.objectContaining({ status: 'cancelled' }),
    });
  });

  it('never renders the answer twice when a transcript records it as narration', () => {
    // An older core projects a prompt-guided turn's answer as an interim step
    // with every call after it. The answer must still render once, last.
    const answer = msg({
      id: 'answer',
      sender: 'agent',
      content: 'The setting is on.',
      extraMetadata: { requestId: 'req-p' },
    });
    const content = buildRuntimeMessages([answer], null, {
      turnTimelines: { 'req-p': [tool({ id: 'call-1', status: 'success', result: 'ok' })] },
      turnTranscripts: {
        'req-p': [
          { kind: 'narration', round: 3, seq: 0, text: 'The setting is on.' },
          { kind: 'toolCall', round: 3, seq: 1, callId: 'call-1' },
        ],
      },
    })[0]?.content;
    expect(Array.isArray(content) ? content.map(part => part.type) : content).toEqual([
      'tool-call',
      'text',
    ]);
  });

  it('chronologically anchors persisted trails to async agent messages without request ids', () => {
    const acknowledgement = msg({
      id: 'ack',
      sender: 'agent',
      content: 'Accepted background work',
      extraMetadata: {},
    });
    const content = buildRuntimeMessages([acknowledgement], null, {
      isRunning: false,
      turnTimelines: { 'request-async': [tool({ id: 'async-tool', status: 'success' })] },
      turnTranscripts: {
        'request-async': [
          { kind: 'thinking', round: 1, seq: 0, text: 'delegate this research' },
          { kind: 'toolCall', round: 1, seq: 1, callId: 'async-tool' },
        ],
      },
    })[0]?.content;

    expect(content).toEqual([
      { type: 'reasoning', text: 'delegate this research' },
      expect.objectContaining({ type: 'tool-call', toolCallId: 'async-tool' }),
      { type: 'text', text: 'Accepted background work' },
    ]);
  });

  it('renders final streamed narration only once', () => {
    const finalText = 'hey! what is up?';
    const answer = msg({ id: 'answer', sender: 'agent', content: finalText });
    const content = buildRuntimeMessages([answer], null, {
      turnTranscripts: { request: [{ kind: 'narration', round: 1, seq: 0, text: finalText }] },
      turnTimelines: { request: [] },
    })[0]?.content;

    expect(content).toEqual([{ type: 'text', text: finalText }]);
  });

  it('coalesces legacy assistant segments into one bubble with one tool trail', () => {
    const requestId = 'legacy-segmented-request';
    const intro = "Here's the crypto picture today:";
    const finalText = `${intro}\n\nBitcoin is trading around $77,000.`;
    const messages = [
      msg({ id: 'user', content: 'What is happening with Bitcoin?' }),
      msg({
        id: 'tool-envelope',
        sender: 'agent',
        content: JSON.stringify({
          content: null,
          tool_calls: [
            { id: 'call-search', name: 'web_search_tool', arguments: '{"query":"bitcoin"}' },
          ],
        }),
        extraMetadata: { requestId },
      }),
      msg({ id: 'intro', sender: 'agent', content: intro, extraMetadata: { requestId } }),
      msg({ id: 'final', sender: 'agent', content: finalText, extraMetadata: { requestId } }),
    ];

    const projected = buildRuntimeMessages(messages, null, {
      turnTimelines: {
        [requestId]: [
          tool({ id: 'call-search', name: 'tool', status: 'success', result: 'market results' }),
        ],
      },
      turnTranscripts: {
        [requestId]: [{ kind: 'toolCall', round: 1, seq: 0, callId: 'call-search' }],
      },
    });

    expect(projected).toHaveLength(2);
    expect(projected[1]).toMatchObject({ id: 'final', role: 'assistant' });
    // One bubble, carrying the final text once — the intro segment is a prefix
    // of it and must not render twice.
    // The row is minted as `tool` and recovered to `web_search_tool` from the
    // envelope. That rename used to be asserted only INDIRECTLY, through a
    // read-only visibility filter that dropped the row once it was renamed —
    // so with the filter gone the old assertion would have quietly stopped
    // testing the rename at all. It is named directly here instead.
    expect(projected[1]?.content).toEqual([
      expect.objectContaining({
        type: 'tool-call',
        toolCallId: 'call-search',
        toolName: 'web_search_tool',
      }),
      { type: 'text', text: finalText },
    ]);
  });

  it('does not coalesce adjacent assistant turns with different request ids', () => {
    const projected = buildRuntimeMessages(
      [
        msg({ id: 'first', sender: 'agent', content: 'first', extraMetadata: { requestId: 'r1' } }),
        msg({
          id: 'second',
          sender: 'agent',
          content: 'second',
          extraMetadata: { requestId: 'r2' },
        }),
      ],
      null
    );

    expect(projected.map(message => message.id)).toEqual(['first', 'second']);
  });

  it('keeps a scoped standalone delivery out of the adjacent legacy runs', () => {
    // Legacy segments carry no request id; a background delivery persisted by
    // the core carries no request id either but is stamped with a `scope`.
    // Without the marker the three rows read as one segmented answer and the
    // delivery's text and metadata would be folded into its neighbours.
    const projected = buildRuntimeMessages(
      [
        msg({ id: 'seg-a', sender: 'agent', content: 'first paragraph' }),
        msg({
          id: 'delivery',
          sender: 'agent',
          content: 'Background result: inbox digest',
          extraMetadata: { scope: 'autonomous_task_result', success: true },
        }),
        msg({ id: 'seg-b', sender: 'agent', content: 'a later paragraph' }),
      ],
      null
    );

    expect(projected.map(message => message.id)).toEqual(['seg-a', 'delivery', 'seg-b']);
    expect(projected[1]?.content).toEqual([
      { type: 'text', text: 'Background result: inbox digest' },
    ]);
  });

  it('does not let a scoped delivery absorb the identified segment before it', () => {
    const projected = buildRuntimeMessages(
      [
        msg({
          id: 'answer',
          sender: 'agent',
          content: 'answer',
          extraMetadata: { requestId: 'r1' },
        }),
        msg({
          id: 'delivery',
          sender: 'agent',
          content: 'worker output',
          extraMetadata: { scope: 'worker_thread', requestId: 'r-worker' },
        }),
      ],
      null
    );

    expect(projected.map(message => message.id)).toEqual(['answer', 'delivery']);
  });

  it('does not hand a later turn trail to an earlier trail-less answer', () => {
    // Two unanchored answers, one unclaimed trail. Positional pairing would
    // give the trail to `earlier` (which produced nothing) and leave `later`
    // (which actually used the tool) bare — a wrong attribution, not a loss.
    const projected = buildRuntimeMessages(
      [
        msg({ id: 'ask-1', content: 'first question' }),
        msg({ id: 'earlier', sender: 'agent', content: 'plain answer', extraMetadata: {} }),
        msg({ id: 'ask-2', content: 'second question' }),
        msg({ id: 'later', sender: 'agent', content: 'tool answer', extraMetadata: {} }),
      ],
      null,
      {
        isRunning: false,
        turnTimelines: { 'request-later': [tool({ id: 'later-tool', status: 'success' })] },
        turnTranscripts: {
          'request-later': [{ kind: 'toolCall', round: 1, seq: 0, callId: 'later-tool' }],
        },
      }
    );

    const toolBearing = projected
      .filter(
        message =>
          Array.isArray(message.content) && message.content.some(part => part.type === 'tool-call')
      )
      .map(message => message.id);
    expect(toolBearing).not.toContain('earlier');
    expect(projected[1]?.content).toEqual([{ type: 'text', text: 'plain answer' }]);
  });

  /**
   * The crash this guards: assistant-ui keys tool parts as `toolCallId-${id}`
   * and throws "Duplicate key … in useResources" on a repeat, taking the whole
   * thread render down on load. A provider that emits tool calls without ids
   * writes `''` for every one, so a settled turn can hold two transcript
   * pointers naming the same row.
   */
  it('emits one tool part per row when a turn has two pointers to the same call id', () => {
    const answer = msg({
      id: 'answer',
      sender: 'agent',
      content: 'done',
      extraMetadata: { requestId: 'req-1' },
    });
    const timeline = [tool({ id: '', status: 'success', result: 'once' })];
    const transcript = [
      { kind: 'toolCall' as const, round: 1, seq: 0, callId: '' },
      { kind: 'toolCall' as const, round: 1, seq: 1, callId: '' },
    ];

    const content = buildRuntimeMessages([answer], null, {
      turnTimelines: { 'req-1': timeline },
      turnTranscripts: { 'req-1': transcript },
    })[0]?.content as unknown as { type: string; toolCallId?: string }[];

    const toolIds = content.filter(part => part.type === 'tool-call').map(part => part.toolCallId);
    expect(toolIds).toHaveLength(1);
  });

  it('never repeats a toolCallId across the transcript and timeline passes', () => {
    const answer = msg({
      id: 'answer',
      sender: 'agent',
      content: 'done',
      extraMetadata: { requestId: 'req-1' },
    });
    const timeline = [
      tool({ id: 'c1', status: 'success', result: 'a' }),
      tool({ id: 'c2', status: 'success', result: 'b' }),
    ];
    const transcript = [{ kind: 'toolCall' as const, round: 1, seq: 0, callId: 'c1' }];

    const content = buildRuntimeMessages([answer], null, {
      turnTimelines: { 'req-1': timeline },
      turnTranscripts: { 'req-1': transcript },
    })[0]?.content as unknown as { type: string; toolCallId?: string }[];

    const toolIds = content.filter(part => part.type === 'tool-call').map(part => part.toolCallId);
    expect(toolIds).toEqual(['c1', 'c2']);
    expect(new Set(toolIds).size).toBe(toolIds.length);
  });

  it('re-converts only the tail as tokens land, never the settled transcript', () => {
    // Streaming must not sweep the transcript: only the live tail re-converts.
    const settled = Array.from({ length: 40 }, (_, i) =>
      msg({ id: `m-${i}`, sender: i % 2 ? 'agent' : 'user', content: `prose ${i}` })
    );
    const parse = vi.spyOn(JSON, 'parse');

    buildRuntimeMessages(settled, null); // warm the identity cache
    parse.mockClear();

    let text = '';
    for (let i = 0; i < 5; i += 1) {
      text += ` tok${i}`;
      buildRuntimeMessages(settled, { requestId: 'r', content: text, thinking: '' });
    }

    // Zero: settled messages are cached by identity and the tail is plain text.
    expect(parse).not.toHaveBeenCalled();
    parse.mockRestore();
  });
});

describe('recovered tool names', () => {
  it('does not consume a recovered name on an entry it does not rename', () => {
    // `recoveredNames` comes from tool-call envelopes, so it only ever holds
    // names for the *generic* rows. Advancing the cursor on every entry made a
    // named row eat the first recovered name: the first generic row then took
    // the second name and the last one kept the placeholder.
    const converted = toThreadMessageLike(
      msg({
        id: 'a',
        sender: 'agent',
        content: 'done',
        extraMetadata: { assistantUiToolNames: ['shell', 'apply_patch'] },
      }),
      [
        tool({ id: 'c1', name: 'file_write', seq: 0, status: 'success' }),
        tool({ id: 'c2', name: 'tool', seq: 1, status: 'success' }),
        tool({ id: 'c3', name: 'tool', seq: 2, status: 'success' }),
      ]
    );
    const names = (converted.content as unknown as { type: string; toolName?: string }[])
      .filter(part => part.type === 'tool-call')
      .map(part => part.toolName);
    expect(names).toEqual(['file_write', 'shell', 'apply_patch']);
  });
});

describe('terminal tool status', () => {
  it('carries a failed tool status through to the rendered part', () => {
    // assistant-ui's tool-call part has no status field, so a failed tool that
    // produced output used to arrive as a bare result and read as success.
    // `web_search` is read-only, so this doubles as proof that a FAILED row
    // stays on the main surface whatever its category.
    const converted = toThreadMessageLike(msg({ id: 'a', sender: 'agent', content: 'done' }), [
      tool({ id: 'c1', name: 'web_search', seq: 0, status: 'error', result: 'boom' }),
    ]);
    const part = (converted.content as unknown as { type: string; result?: unknown }[]).find(
      candidate => candidate.type === 'tool-call'
    );
    expect(part?.result).toMatchObject({ status: 'error', value: 'boom' });
  });

  it('leaves a successful tool result untouched', () => {
    const converted = toThreadMessageLike(msg({ id: 'a', sender: 'agent', content: 'done' }), [
      tool({ id: 'c1', name: 'file_write', seq: 0, status: 'success', result: 'the answer' }),
    ]);
    const part = (converted.content as unknown as { type: string; result?: unknown }[]).find(
      candidate => candidate.type === 'tool-call'
    );
    expect(part?.result).toBe('the answer');
  });
});

describe('main-surface tool rows', () => {
  const partsOf = (entries: ToolTimelineEntry[]) =>
    toThreadMessageLike(msg({ id: 'a', sender: 'agent', content: 'done' }), entries)
      .content as unknown as { type: string; toolCallId?: string }[];
  const visibleIds = (entries: ToolTimelineEntry[]) =>
    partsOf(entries)
      .filter(part => part.type === 'tool-call')
      .map(part => part.toolCallId);

  it('renders a read-only step instead of dropping it to the rail', () => {
    // These four used to be filtered off the surface as "process". They are
    // back: assistant-ui coalesces consecutive tool rows into ONE collapsed
    // group, so the clutter argument that justified hiding them no longer
    // holds — the cost is a count in a group header, and the benefit is that
    // the transcript shows what the agent actually did.
    //
    // `web_search_tool` is the name a real search row carries; `web_search` is
    // the settings-family id the core expands from. Both are asserted so
    // neither can regress.
    expect(
      visibleIds([
        tool({ id: 'r1', name: 'file_read', seq: 0, status: 'success' }),
        tool({ id: 'r2', name: 'grep', seq: 1, status: 'success' }),
        tool({ id: 'r3', name: 'web_search_tool', seq: 2, status: 'success' }),
        tool({ id: 'r4', name: 'web_search', seq: 3, status: 'success' }),
      ])
    ).toEqual(['r1', 'r2', 'r3', 'r4']);
  });

  it('renders a side-effecting step', () => {
    expect(
      visibleIds([
        tool({ id: 'w', name: 'file_write', seq: 0, status: 'success' }),
        tool({ id: 's', name: 'shell', seq: 1, status: 'success' }),
        tool({ id: 'x', name: 'send_email_to_customer', seq: 2, status: 'success' }),
        tool({ id: 'f', name: 'curl', seq: 3, status: 'success' }),
      ])
    ).toEqual(['w', 's', 'x', 'f']);
  });

  it('renders a step that failed or is waiting on the user', () => {
    expect(
      visibleIds([
        tool({ id: 'e', name: 'file_read', seq: 0, status: 'error' }),
        tool({ id: 'a', name: 'web_search', seq: 1, status: 'awaiting_user' }),
      ])
    ).toEqual(['e', 'a']);
  });

  it('renders a delegation row — it is the only door to the sub-agent drawer', () => {
    expect(visibleIds([tool({ id: 'd', name: 'subagent:researcher', status: 'success' })])).toEqual(
      ['d']
    );
  });
});

describe('part ordering', () => {
  const contentOf = (
    entries: ToolTimelineEntry[],
    transcript: Parameters<typeof toThreadMessageLike>[2]
  ) =>
    toThreadMessageLike(msg({ id: 'a', sender: 'agent', content: 'done' }), entries, transcript)
      .content as unknown as { type: string; toolCallId?: string; text?: string }[];

  it('follows the transcript, reasoning first, answer last', () => {
    expect(
      contentOf(
        [
          tool({ id: 'c1', seq: 0, status: 'success' }),
          tool({ id: 'c2', seq: 1, status: 'success' }),
        ],
        [
          { kind: 'thinking', round: 1, seq: 0, text: 'plan it' },
          { kind: 'toolCall', round: 1, seq: 1, callId: 'c1' },
          { kind: 'toolCall', round: 1, seq: 2, callId: 'c2' },
        ]
      ).map(part => part.toolCallId ?? part.type)
    ).toEqual(['reasoning', 'c1', 'c2', 'text']);
  });

  it('places a row the transcript never named by its own seq, not at the end', () => {
    // THE ordering defect. `c-early` was issued FIRST (seq 0) but no transcript
    // pointer ever landed for it — a live turn mints the row before the pointer
    // arrives, and a legacy snapshot has no transcript at all. It used to be
    // appended after every row the transcript DID name, so the first call the
    // agent made rendered last.
    expect(
      contentOf(
        [
          tool({ id: 'c-early', seq: 0, status: 'success' }),
          tool({ id: 'c-named', seq: 1, status: 'success' }),
        ],
        [{ kind: 'toolCall', round: 1, seq: 0, callId: 'c-named' }]
      ).map(part => part.toolCallId ?? part.type)
    ).toEqual(['c-early', 'c-named', 'text']);
  });

  it('keeps an unnamed later row after the named row it followed', () => {
    // The mirror of the case above, so the fix cannot be "always put unnamed
    // rows first", which would pass the previous test while still being wrong.
    expect(
      contentOf(
        [
          tool({ id: 'c-named', seq: 0, status: 'success' }),
          tool({ id: 'c-late', seq: 1, status: 'success' }),
        ],
        [{ kind: 'toolCall', round: 1, seq: 0, callId: 'c-named' }]
      ).map(part => part.toolCallId ?? part.type)
    ).toEqual(['c-named', 'c-late', 'text']);
  });

  it('orders a legacy snapshot with no transcript by seq', () => {
    expect(
      contentOf(
        [
          tool({ id: 'second', seq: 5, status: 'success' }),
          tool({ id: 'first', seq: 1, status: 'success' }),
        ],
        []
      ).map(part => part.toolCallId ?? part.type)
    ).toEqual(['first', 'second', 'text']);
  });

  it('emits one part for a row two transcript pointers both name', () => {
    // assistant-ui THROWS on a duplicate `toolCallId` and takes the whole
    // thread render down, so this is an invariant rather than tidiness.
    expect(
      contentOf(
        [tool({ id: 'dup', seq: 0, status: 'success' })],
        [
          { kind: 'toolCall', round: 1, seq: 0, callId: 'dup' },
          { kind: 'toolCall', round: 1, seq: 1, callId: 'dup' },
        ]
      ).filter(part => part.type === 'tool-call')
    ).toHaveLength(1);
  });
});

describe('one copy of the turn', () => {
  const custom = (message: { metadata?: unknown }) =>
    (message.metadata as { custom?: Record<string, unknown> } | undefined)?.custom ?? {};

  it('carries reasoning and tools only as parts, with no second trail in metadata', () => {
    // A settled answer used to carry its reasoning and tools twice: inline as
    // parts, and again as `metadata.custom.processTrail`, which a footer under
    // the answer summarised as "N steps · M tools".
    const converted = toThreadMessageLike(
      msg({ id: 'a', sender: 'agent', content: 'done' }),
      [tool({ id: 'c1', name: 'file_read', status: 'success' })],
      [
        { kind: 'thinking', round: 1, seq: 0, text: 'think' },
        { kind: 'toolCall', round: 1, seq: 1, callId: 'c1' },
      ]
    );
    expect(converted.content).toEqual([
      { type: 'reasoning', text: 'think' },
      expect.objectContaining({ type: 'tool-call', toolCallId: 'c1' }),
      { type: 'text', text: 'done' },
    ]);
    expect(custom(converted)).not.toHaveProperty('processTrail');
  });

  it('emits the pages the turn fetched as url source parts after the answer', () => {
    const converted = toThreadMessageLike(msg({ id: 'a', sender: 'agent', content: 'done' }), [
      tool({
        id: 'f1',
        name: 'web_fetch',
        status: 'success',
        argsBuffer: '{"url":"https://example.com/a"}',
      }),
      tool({
        id: 'f2',
        seq: 1,
        name: 'web_fetch',
        status: 'success',
        argsBuffer: '{"url":"javascript:alert(1)"}',
      }),
    ]);
    const parts = converted.content as unknown as { type: string }[];
    expect(parts.at(-1)).toEqual({
      type: 'source',
      sourceType: 'url',
      id: 'f1',
      url: 'https://example.com/a',
      title: 'example.com',
    });
    expect(parts.filter(part => part.type === 'source')).toHaveLength(1);
  });
});

describe('tool label on the part', () => {
  const artifactOf = (converted: { content: unknown }) =>
    (converted.content as { type: string; artifact?: unknown }[]).find(
      part => part.type === 'tool-call'
    )?.artifact;

  it('carries the row label and detail the store resolved', () => {
    const converted = toThreadMessageLike(msg({ id: 'a', sender: 'agent', content: 'done' }), [
      tool({
        id: 'c1',
        name: 'GMAIL_SEND_EMAIL',
        status: 'success',
        displayName: 'Gmail send email',
        detail: 'me@example.com',
      }),
    ]);
    expect(artifactOf(converted)).toEqual({
      kind: 'openhuman-tool',
      displayName: 'Gmail send email',
      detail: 'me@example.com',
    });
  });

  it('carries no artifact when the row has no server label — the renderer derives the label from tool identity', () => {
    // `tool_search` is a client-known tool (an exact `toolSpecs.ts` entry), so
    // `AssistantUiToolCall` resolves its own label through `describeToolCall`
    // and never needs the artifact. Emitting one here would be pure noise.
    const converted = toThreadMessageLike(msg({ id: 'a', sender: 'agent', content: 'done' }), [
      tool({ id: 'c1', name: 'tool_search', status: 'success', argsBuffer: '{"query":"gmail"}' }),
    ]);
    expect(artifactOf(converted)).toBeUndefined();
  });
});

describe('narration merged into the final answer', () => {
  it('does not render narration that the final text already contains', () => {
    // `mergedAssistantText` prefers the longest text when it *contains* every
    // segment, so the duplicate is a substring rather than an exact match and
    // the old equality guard let it through twice.
    const finalText = 'I will check the sources. Here is what I found.';
    const converted = toThreadMessageLike(
      msg({ id: 'a', sender: 'agent', content: finalText }),
      [],
      [{ kind: 'narration', round: 1, seq: 0, text: 'I will check the sources.' }]
    );
    const texts = (converted.content as unknown as { type: string; text?: string }[])
      .filter(part => part.type === 'text')
      .map(part => part.text);
    expect(texts).toEqual([finalText]);
  });
});

describe('feedback round-trip (Defect A)', () => {
  // These assert `metadata.submittedFeedback`, which is what the runtime's
  // pressed state reads (`s.message.metadata.submittedFeedback?.type`, see
  // `ActionBarFeedbackPositive.js`). Asserting the persisted `extraMetadata`
  // instead would be vacuous: that value already survives without this fix, and
  // the bug is precisely that it never reaches the runtime.
  it('re-emits a persisted rating as submittedFeedback so a pressed thumb survives a rebuild', () => {
    const converted = toThreadMessageLike(
      msg({ id: 'a1', sender: 'agent', extraMetadata: { feedback: 'positive' } })
    );
    expect(converted.metadata?.submittedFeedback).toEqual({ type: 'positive' });
  });

  it('re-emits a negative rating', () => {
    const converted = toThreadMessageLike(
      msg({ id: 'a1', sender: 'agent', extraMetadata: { feedback: 'negative' } })
    );
    expect(converted.metadata?.submittedFeedback).toEqual({ type: 'negative' });
  });

  it('leaves an unrated message with no submittedFeedback at all', () => {
    // Absent, not `{ type: undefined }` — the runtime reads
    // `submittedFeedback?.type`, so an empty object would still be falsy, but
    // emitting one would mean every message claims a rating field it lacks.
    const converted = toThreadMessageLike(msg({ id: 'a1', sender: 'agent' }));
    expect(converted.metadata?.submittedFeedback).toBeUndefined();
  });

  it('ignores a rating value the runtime cannot accept', () => {
    // `extraMetadata` is untyped JSON from disk. A stale or hand-edited value
    // must read as unrated rather than reach the runtime as a bad type.
    for (const bad of ['up', '', 'POSITIVE', 1, true, null, {}]) {
      const converted = toThreadMessageLike(
        msg({ id: 'a1', sender: 'agent', extraMetadata: { feedback: bad } })
      );
      expect(converted.metadata?.submittedFeedback).toBeUndefined();
    }
  });

  it('does not rate a user message even if its metadata carries one', () => {
    const converted = toThreadMessageLike(
      msg({ id: 'u1', sender: 'user', extraMetadata: { feedback: 'positive' } })
    );
    expect(converted.metadata?.submittedFeedback).toBeUndefined();
  });
});

describe('next-turn history stability', () => {
  it('keeps a merged assistant reply unchanged when a new user turn streams', () => {
    const history = [
      msg({ id: 'user-1' }),
      msg({
        id: 'segment-1',
        sender: 'agent',
        content: 'First step',
        extraMetadata: { requestId: 'r1' },
      }),
      msg({
        id: 'segment-2',
        sender: 'agent',
        content: 'Final answer',
        extraMetadata: { requestId: 'r1' },
      }),
    ];
    const before = buildRuntimeMessages(history, null, { isRunning: false });
    const after = buildRuntimeMessages(
      [...history, msg({ id: 'user-2', content: 'try now' })],
      { requestId: 'r2', content: 'New response', thinking: '' },
      { isRunning: true, liveRequestId: 'r2' }
    );
    expect(after.map(message => message.id)).toEqual([
      before[0]!.id,
      before[1]!.id,
      'user-2',
      streamingMessageId('r2'),
    ]);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
  });
});

it('simulates repeated next turns without changing settled part order or identities', () => {
  const history: ThreadMessage[] = [];
  let previous: ReturnType<typeof buildRuntimeMessages> = [];
  for (let turn = 0; turn < 12; turn++) {
    history.push(msg({ id: `user-${turn}`, content: turn ? 'try now' : 'first turn' }));
    for (const text of ['Thinking', 'Working', 'Finished']) {
      const running = buildRuntimeMessages(
        history,
        { requestId: `r${turn}`, content: text, thinking: '' },
        { isRunning: true, liveRequestId: `r${turn}` }
      );
      previous.forEach((message, index) => expect(running[index]).toBe(message));
      expect(running.at(-2)?.id).toBe(`user-${turn}`);
      expect(running.at(-1)?.id).toBe(streamingMessageId(`r${turn}`));
    }
    history.push(
      msg({
        id: `agent-${turn}`,
        sender: 'agent',
        content: `Completed turn ${turn}`,
        extraMetadata: { requestId: `r${turn}` },
      })
    );
    const settled = buildRuntimeMessages(history, null, { isRunning: false });
    previous.forEach((message, index) => expect(settled[index]).toBe(message));
    expect(settled.map(message => message.role)).toEqual(
      Array.from({ length: turn + 1 }, () => ['user', 'assistant']).flat()
    );
    previous = settled;
  }
});
