/**
 * Regression guard: a streamed turn and the same turn reopened from history
 * must render the same way, and the stream must never reshape what is already
 * on screen.
 *
 * The defect this pins: a live thread glitched — tools reordered, text
 * restarted its reveal, cards collapsed, the view jumped — and the same thread
 * opened fresh looked right. Each glitch was a part that moved or remounted,
 * because assistant-ui keys text/reasoning parts and whole messages by INDEX.
 * The properties below are the ones whose violation produced those glitches:
 *
 * 1. **Parity** — the settled message's parts equal the parts the core
 *    projection (`mapDisplayItems`) produces for the same turn on reload.
 * 2. **Append-only** — while the turn streams, no part that is on screen moves
 *    to another index or changes kind/id; text only grows.
 * 3. **Atomic settle** — across every store update of `chat_done`, the turn
 *    stays at one message index, never renders twice, never loses its answer,
 *    and the swap from live tail to persisted reply keeps every part in place.
 *
 * Both tellings of the turn come from one fixture (`test/fixtures/streamedTurn`),
 * driven through the real socket handlers of `ChatRuntimeProvider`.
 */
import type { ThreadMessageLike } from '@assistant-ui/react';
import { render, waitFor } from '@testing-library/react';
import { act } from 'react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import * as chatService from '../../services/chatService';
import { mapDisplayItems } from '../../features/conversations/derived/mapDisplayItems';
import { selectBackgroundProcesses } from '../../features/conversations/selectors/backgroundProcesses';
import { threadApi } from '../../services/api/threadApi';
import { store } from '../../store';
import { beginInferenceTurn, clearAllChatRuntime } from '../../store/chatRuntimeSlice';
import { setStatusForUser } from '../../store/socketSlice';
import { addMessageLocal, clearAllThreads } from '../../store/threadSlice';
import {
  DONE_EVENT,
  FINAL_ANSWER,
  HISTORY_ITEMS,
  LIVE_TURN_STEPS,
  type SocketStep,
  TURN_REQUEST,
  TURN_THREAD,
} from '../../test/fixtures/streamedTurn';
import type { ThreadMessage } from '../../types/thread';
import {
  buildRuntimeMessages,
  STREAMING_TAIL_ID,
  streamingMessageId,
} from '../assistantUiMessages';
import ChatRuntimeProvider from '../ChatRuntimeProvider';

vi.mock('../../services/chatService', async () => {
  const actual = await vi.importActual<typeof chatService>('../../services/chatService');
  return { ...actual, subscribeChatEvents: vi.fn() };
});

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: vi.fn(),
    getThreads: vi.fn(),
    getThreadMessages: vi.fn(),
    appendMessage: vi.fn(),
    generateTitleIfNeeded: vi.fn(),
    updateMessage: vi.fn(),
    deleteThread: vi.fn(),
    purge: vi.fn(),
    getTurnState: vi.fn(),
    listRuns: vi.fn(),
  },
}));

vi.mock('../../services/socketService', () => ({
  socketService: { subscribeThread: vi.fn(() => Promise.resolve(true)), on: vi.fn(), off: vi.fn() },
}));

vi.mock('../../hooks/usageRefresh', () => ({ requestUsageRefresh: vi.fn() }));

vi.mock('../../hooks/useRefetchSnapshotOnTurnEnd', () => ({
  useRefetchSnapshotOnTurnEnd: () => ({ refetch: vi.fn() }),
}));

const USER_MESSAGE: ThreadMessage = {
  id: 'user-1',
  content: 'What is on today?',
  type: 'text',
  extraMetadata: {},
  sender: 'user',
  createdAt: '2026-09-24T10:00:00.000Z',
};

function renderProvider(): chatService.ChatEventListeners {
  let captured: chatService.ChatEventListeners = {};
  vi.mocked(chatService.subscribeChatEvents).mockImplementation(listeners => {
    captured = listeners;
    return () => {};
  });
  store.dispatch(setStatusForUser({ userId: '__pending__', status: 'connected' }));
  render(
    <Provider store={store}>
      <ChatRuntimeProvider>
        <div />
      </ChatRuntimeProvider>
    </Provider>
  );
  return captured;
}

function fire(listeners: chatService.ChatEventListeners, step: SocketStep) {
  const listener = listeners[step.listener] as ((event: unknown) => void) | undefined;
  if (!listener) throw new Error(`provider does not handle ${step.listener}`);
  act(() => listener(step.event));
}

/**
 * The runtime messages for the thread, built from the store exactly as
 * `useOpenHumanExternalStore` builds them (minus the core-transcript fetch,
 * which a settled turn of this session does not read — its frozen trail wins).
 */
function project(): ThreadMessageLike[] {
  const state = store.getState();
  const runtime = state.chatRuntime;
  const streaming = runtime.streamingAssistantByThread[TURN_THREAD] ?? null;
  const lifecycle = runtime.inferenceTurnLifecycleByThread[TURN_THREAD];
  return buildRuntimeMessages(state.thread.messagesByThreadId[TURN_THREAD] ?? [], streaming, {
    isRunning: lifecycle === 'started' || lifecycle === 'streaming',
    liveTimeline: runtime.toolTimelineByThread[TURN_THREAD] ?? [],
    liveTranscript: runtime.processingByThread[TURN_THREAD] ?? [],
    liveTimelineRequestId: runtime.toolTimelineRequestByThread[TURN_THREAD],
    pendingApproval: runtime.pendingApprovalByThread[TURN_THREAD] ?? null,
    settledTurns: runtime.settledTurnsByThread?.[TURN_THREAD] ?? {},
    liveRequestId: runtime.liveRequestIdByThread?.[TURN_THREAD] ?? streaming?.requestId,
  });
}

type PartShape =
  | { type: 'text' | 'reasoning'; text: string }
  | { type: 'tool-call'; toolName: string; args?: unknown; settled: boolean }
  | { type: string };

/**
 * What a reader sees of a part. Tool ids are compared separately where they
 * must match; here a sub-agent's id legitimately differs between the socket
 * (`<thread>:subagent:<task>:<agent>`) and the core (`subagent:<agent>`),
 * which is exactly why a settled turn keeps its frozen live trail in-session.
 */
function shape(parts: ThreadMessageLike['content']): PartShape[] {
  if (typeof parts === 'string') return [{ type: 'text', text: parts }];
  return parts.map(part => {
    if (part.type === 'text' || part.type === 'reasoning') {
      return { type: part.type, text: part.text };
    }
    if (part.type === 'tool-call') {
      return {
        type: 'tool-call',
        toolName: part.toolName,
        // A delegation's args carry live progress while it runs; compare the
        // plain calls' input only.
        ...(part.toolName === 'task' ? {} : { args: part.args }),
        settled: part.result !== undefined,
      };
    }
    return { type: part.type };
  });
}

function partsOf(message: ThreadMessageLike | undefined): ThreadMessageLike['content'] {
  if (!message) throw new Error('message missing');
  return message.content;
}

function arrayParts(message: ThreadMessageLike | undefined) {
  const parts = partsOf(message);
  if (typeof parts === 'string') throw new Error('expected part array');
  return parts;
}

/** The key assistant-ui gives a part: tool id for tool calls, index otherwise. */
function partKey(part: ReturnType<typeof arrayParts>[number], index: number): string {
  return part.type === 'tool-call' ? `tool:${part.toolCallId}` : `${part.type}@${index}`;
}

describe('live ≡ history rendering of one turn', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    store.dispatch(clearAllThreads());
    store.dispatch(clearAllChatRuntime());
    vi.mocked(threadApi.appendMessage).mockImplementation(async (_tid, message) => message);
    vi.mocked(threadApi.getThreads).mockResolvedValue({ threads: [], count: 0 });
    vi.mocked(threadApi.getTurnState).mockResolvedValue(null);
    vi.mocked(threadApi.listRuns).mockResolvedValue([]);
    vi.mocked(threadApi.generateTitleIfNeeded).mockResolvedValue({
      id: TURN_THREAD,
      title: 'Today',
    } as never);
  });

  afterEach(() => {
    store.dispatch(setStatusForUser({ userId: '__pending__', status: 'disconnected' }));
  });

  async function startTurn() {
    await store.dispatch(addMessageLocal({ threadId: TURN_THREAD, message: USER_MESSAGE }));
    store.dispatch(beginInferenceTurn({ threadId: TURN_THREAD }));
  }

  /** Stream the whole fixture, recording the tail's parts after every event. */
  async function streamTurn(listeners: chatService.ChatEventListeners) {
    const snapshots: ReturnType<typeof arrayParts>[] = [];
    for (const step of LIVE_TURN_STEPS) {
      fire(listeners, step);
      // Deltas are coalesced per frame; the next non-delta event flushes them,
      // and the last run is flushed by waiting for the frame.
      const tail = project().at(-1);
      if (tail?.id === streamingMessageId(TURN_REQUEST)) snapshots.push(arrayParts(tail));
    }
    await waitFor(() => {
      const tail = project().at(-1);
      expect(tail?.id).toBe(streamingMessageId(TURN_REQUEST));
      expect(shape(partsOf(tail)).at(-1)).toEqual({ type: 'text', text: FINAL_ANSWER });
    });
    snapshots.push(arrayParts(project().at(-1)));
    return snapshots;
  }

  it('streams append-only: nothing on screen moves, changes kind, or loses text', async () => {
    const listeners = renderProvider();
    await startTurn();
    const snapshots = await streamTurn(listeners);

    expect(snapshots.length).toBeGreaterThan(5);
    for (let at = 1; at < snapshots.length; at += 1) {
      const before = snapshots[at - 1];
      const after = snapshots[at];
      expect(after.length).toBeGreaterThanOrEqual(before.length);
      before.forEach((part, index) => {
        const next = after[index];
        // The one sanctioned change: `subagent_spawned` promotes the
        // `spawn_subagent` call, in its own slot, into the delegation card.
        // It becomes a different component either way; what matters is that
        // it stays where the agent issued it and nothing around it moves.
        const promoted =
          part.type === 'tool-call' &&
          part.toolName === 'spawn_subagent' &&
          next.type === 'tool-call' &&
          next.toolName === 'task';
        if (promoted) return;
        // Same key at the same index — what keeps React from remounting it.
        expect(partKey(next, index)).toBe(partKey(part, index));
        if ((part.type === 'text' || part.type === 'reasoning') && next.type === part.type) {
          expect(next.text.startsWith(part.text)).toBe(true);
        }
      });
    }
  });

  it('settles into exactly what the core projection renders on reload', async () => {
    const listeners = renderProvider();
    await startTurn();
    await streamTurn(listeners);

    act(() => listeners.onDone?.(DONE_EVENT));
    await waitFor(() =>
      expect(store.getState().chatRuntime.inferenceTurnLifecycleByThread[TURN_THREAD]).toBe(
        undefined
      )
    );

    const settled = project();
    expect(settled.map(message => message.role)).toEqual(['user', 'assistant']);
    const settledAnswer = settled[1];
    expect(settledAnswer?.id).not.toBe(STREAMING_TAIL_ID);

    // The same turn, reopened: the persisted reply plus the core projection.
    const reloadedMessages = store.getState().thread.messagesByThreadId[TURN_THREAD] ?? [];
    const history = mapDisplayItems(HISTORY_ITEMS);
    const reloaded = buildRuntimeMessages(reloadedMessages, null, {
      isRunning: false,
      turnTimelines: history.timelines,
      turnTranscripts: history.transcripts,
    });

    expect(shape(partsOf(settledAnswer))).toEqual(shape(partsOf(reloaded[1])));
    // And that shape is the turn as it happened, narration included.
    expect(shape(partsOf(reloaded[1])).map(part => part.type)).toEqual([
      'reasoning',
      'text',
      'tool-call',
      'tool-call',
      'reasoning',
      'tool-call',
      'text',
    ]);
  });

  it('settles in one store update: same index, same parts, never doubled, never blank', async () => {
    const listeners = renderProvider();
    await startTurn();
    await streamTurn(listeners);

    const lastTail = arrayParts(project().at(-1));
    const turnIndex = project().length - 1;

    // Record the projection at EVERY store notification through `chat_done`.
    const seen: ThreadMessageLike[][] = [];
    const unsubscribe = store.subscribe(() => seen.push(project()));
    try {
      act(() => listeners.onDone?.(DONE_EVENT));
      await waitFor(() =>
        expect(store.getState().chatRuntime.inferenceTurnLifecycleByThread[TURN_THREAD]).toBe(
          undefined
        )
      );
    } finally {
      unsubscribe();
    }

    expect(seen.length).toBeGreaterThan(0);
    let swaps = 0;
    let previousId = streamingMessageId(TURN_REQUEST);
    for (const messages of seen) {
      const assistants = messages.filter(message => message.role === 'assistant');
      // Never the reply AND the tail at once.
      expect(assistants).toHaveLength(1);
      // Always at the tail's index — a shift is a remount.
      expect(messages.length - 1).toBe(turnIndex);
      const turn = messages[turnIndex];
      const parts = arrayParts(turn);
      // Never blank: the answer is on screen in every intermediate state.
      expect(shape(parts).at(-1)).toEqual({ type: 'text', text: FINAL_ANSWER });
      // Every part stays at its index under its key.
      expect(parts.map(partKey)).toEqual(lastTail.map(partKey));
      if (turn?.id !== previousId) swaps += 1;
      previousId = turn?.id ?? previousId;
    }
    // Exactly one transition from the live tail to the persisted reply.
    expect(swaps).toBe(0);
    expect(previousId).not.toBe(STREAMING_TAIL_ID);
    expect(
      store.getState().chatRuntime.settledTurnsByThread[TURN_THREAD]?.[TURN_REQUEST]
    ).toBeDefined();
  });

  it('keeps the settled turn on its frozen trail when the core projection arrives', async () => {
    const listeners = renderProvider();
    await startTurn();
    await streamTurn(listeners);
    act(() => listeners.onDone?.(DONE_EVENT));
    await waitFor(() =>
      expect(store.getState().chatRuntime.inferenceTurnLifecycleByThread[TURN_THREAD]).toBe(
        undefined
      )
    );

    const before = arrayParts(project()[1]);
    // The core projection lands with its own ids for the same turn.
    const history = mapDisplayItems(HISTORY_ITEMS);
    const runtime = store.getState().chatRuntime;
    const after = arrayParts(
      buildRuntimeMessages(store.getState().thread.messagesByThreadId[TURN_THREAD] ?? [], null, {
        isRunning: false,
        turnTimelines: history.timelines,
        turnTranscripts: history.transcripts,
        settledTurns: runtime.settledTurnsByThread[TURN_THREAD] ?? {},
      })[1]
    );
    // Same keys: nothing remounts when the projection replaces nothing.
    expect(after.map(partKey)).toEqual(before.map(partKey));
  });
});

/**
 * A detached (`async`) delegation outlives the turn that spawned it. Seen
 * live: after the child finished and the background delivery posted its
 * result, a "Delegated to … · async" card kept spinning with "Cancel task"
 * under that delivery reply. The turn's `chat_done` hydrates the completed
 * snapshot, whose async row carries the core's id (`subagent:<task>`), so the
 * late `subagent_completed` (socket row id) missed it; the delivery then
 * arrived as a bare `chat_done` and froze that stale row as its own trail.
 */
describe('async delegation settles after its turn', () => {
  const spawnedStep = LIVE_TURN_STEPS.find(step => step.listener === 'onSubagentSpawned');
  const doneStep = LIVE_TURN_STEPS.find(step => step.listener === 'onSubagentDone');
  if (!spawnedStep || !doneStep) throw new Error('fixture lost its delegation');
  const asyncSteps = LIVE_TURN_STEPS.filter(step => step !== doneStep).map(step =>
    step === spawnedStep
      ? ({
          ...step,
          event: { ...step.event, subagent: { mode: 'async', parent_call_id: 'call-spawn' } },
        } as SocketStep)
      : step
  );
  const DELIVERY_REQUEST = 'bgdeliver-1';
  // The completed snapshot as the core persists it: same delegation, core ids.
  const snapshot = (status: 'running' | 'success') => ({
    threadId: TURN_THREAD,
    requestId: TURN_REQUEST,
    lifecycle: 'completed' as const,
    iteration: 3,
    maxIterations: 10,
    streamingText: '',
    thinking: '',
    startedAt: '2026-09-28T11:03:05Z',
    updatedAt: '2026-09-28T11:04:06Z',
    toolTimeline: [
      { id: 'call-spawn', name: 'spawn_subagent', status: 'success' as const, round: 2 },
      {
        id: 'subagent:sub-1',
        name: 'subagent:researcher',
        status,
        round: 2,
        subagent: {
          taskId: 'sub-1',
          agentId: 'researcher',
          mode: 'async',
          parentCallId: 'call-spawn',
          toolCalls: [],
        },
      },
    ],
  });

  /** Every delegation card on screen: `[message, settled status | 'running']`. */
  function delegationCards(): [string | undefined, string][] {
    return project().flatMap(message =>
      typeof message.content === 'string'
        ? []
        : message.content.flatMap(part =>
            part.type === 'tool-call' && part.toolName === 'task'
              ? [
                  [
                    message.id,
                    (part.result as { status?: string } | undefined)?.status ?? 'running',
                  ] as [string | undefined, string],
                ]
              : []
          )
    );
  }

  beforeEach(() => {
    vi.clearAllMocks();
    store.dispatch(clearAllThreads());
    store.dispatch(clearAllChatRuntime());
    vi.mocked(threadApi.appendMessage).mockImplementation(async (_tid, message) => message);
    vi.mocked(threadApi.getThreads).mockResolvedValue({ threads: [], count: 0 });
    vi.mocked(threadApi.listRuns).mockResolvedValue([]);
    vi.mocked(threadApi.generateTitleIfNeeded).mockResolvedValue({
      id: TURN_THREAD,
      title: 'Today',
    } as never);
  });

  afterEach(() => {
    store.dispatch(setStatusForUser({ userId: '__pending__', status: 'disconnected' }));
  });

  it('a late subagent_completed settles the card, and the delivery turn adds no spinner', async () => {
    vi.mocked(threadApi.getTurnState).mockResolvedValue(snapshot('running') as never);
    const listeners = renderProvider();
    await store.dispatch(addMessageLocal({ threadId: TURN_THREAD, message: USER_MESSAGE }));
    store.dispatch(beginInferenceTurn({ threadId: TURN_THREAD }));
    for (const step of asyncSteps) fire(listeners, step);
    act(() => listeners.onDone?.(DONE_EVENT));
    // The completed snapshot (child still running) replaces the live rows.
    await waitFor(() =>
      expect(store.getState().chatRuntime.toolTimelineByThread[TURN_THREAD]?.[1]?.id).toBe(
        'subagent:sub-1'
      )
    );
    expect(delegationCards()).toEqual([[`agent:${TURN_REQUEST}`, 'running']]);

    // Minutes later the child finishes, then its result is delivered as a
    // host-authored turn: a bare `chat_done`, no `inference_start`.
    fire(listeners, doneStep);
    // The hydrated row (core id) settles too: the background-process panel
    // reads the live timeline, and a missed row read "running" there forever.
    expect(
      selectBackgroundProcesses(
        store.getState().chatRuntime.toolTimelineByThread[TURN_THREAD] ?? []
      ).map(process => [process.taskId, process.status])
    ).toEqual([['sub-1', 'success']]);
    vi.mocked(threadApi.getTurnState).mockResolvedValue(snapshot('success') as never);
    act(() =>
      listeners.onDone?.({
        ...DONE_EVENT,
        request_id: DELIVERY_REQUEST,
        full_response: 'The research finished.',
      })
    );
    await waitFor(() =>
      expect(
        store
          .getState()
          .thread.messagesByThreadId[
            TURN_THREAD
          ]?.some(message => message.id === `agent:${DELIVERY_REQUEST}`)
      ).toBe(true)
    );

    const cards = delegationCards();
    expect(cards).toContainEqual([`agent:${TURN_REQUEST}`, 'success']);
    expect(cards.filter(([, status]) => status === 'running')).toEqual([]);
  });
});

/**
 * A reply that settles with no `inference_start` of its own — a background
 * delivery — must not adopt the previous turn's rows as its trail. Seen live:
 * after an async delegation finished, the delivery reply showed a second copy
 * of that turn's "Delegated to … · async" card (spinning, before #6729).
 */
describe("a bare chat_done does not borrow the previous turn's trail", () => {
  const spawnedStep = LIVE_TURN_STEPS.find(step => step.listener === 'onSubagentSpawned');
  const doneStep = LIVE_TURN_STEPS.find(step => step.listener === 'onSubagentDone');
  if (!spawnedStep || !doneStep) throw new Error('fixture lost its delegation');
  const asyncSteps = LIVE_TURN_STEPS.filter(step => step !== doneStep).map(step =>
    step === spawnedStep
      ? ({
          ...step,
          event: { ...step.event, subagent: { mode: 'async', parent_call_id: 'call-spawn' } },
        } as SocketStep)
      : step
  );
  const DELIVERY_REQUEST = 'bgdeliver-1';
  // A completed snapshot as the core persists it (core row ids).
  const snapshot = (requestId: string, subagentStatus: 'running' | 'success') => ({
    threadId: TURN_THREAD,
    requestId,
    lifecycle: 'completed' as const,
    iteration: 3,
    maxIterations: 10,
    streamingText: '',
    thinking: '',
    startedAt: '2026-09-28T11:03:05Z',
    updatedAt: '2026-09-28T11:04:06Z',
    toolTimeline: [
      { id: 'call-spawn', name: 'spawn_subagent', status: 'success' as const, round: 2 },
      {
        id: 'subagent:sub-1',
        name: 'subagent:researcher',
        status: subagentStatus,
        round: 2,
        subagent: {
          taskId: 'sub-1',
          agentId: 'researcher',
          mode: 'async',
          parentCallId: 'call-spawn',
          toolCalls: [],
        },
      },
    ],
  });

  /** Tool-call parts on screen per message: `[messageId, toolName, settled status | 'running']`. */
  function toolCards(): [string | undefined, string, string][] {
    return project().flatMap(message =>
      typeof message.content === 'string'
        ? []
        : message.content.flatMap(part =>
            part.type === 'tool-call'
              ? [
                  [
                    message.id,
                    part.toolName,
                    (part.result as { status?: string } | undefined)?.status ??
                      (part.result === undefined ? 'running' : 'done'),
                  ] as [string | undefined, string, string],
                ]
              : []
          )
    );
  }

  beforeEach(() => {
    vi.clearAllMocks();
    store.dispatch(clearAllThreads());
    store.dispatch(clearAllChatRuntime());
    vi.mocked(threadApi.appendMessage).mockImplementation(async (_tid, message) => message);
    vi.mocked(threadApi.getThreads).mockResolvedValue({ threads: [], count: 0 });
    vi.mocked(threadApi.listRuns).mockResolvedValue([]);
    vi.mocked(threadApi.generateTitleIfNeeded).mockResolvedValue({
      id: TURN_THREAD,
      title: 'Today',
    } as never);
  });

  afterEach(() => {
    store.dispatch(setStatusForUser({ userId: '__pending__', status: 'disconnected' }));
  });

  async function settleAsyncTurn(listeners: chatService.ChatEventListeners) {
    await store.dispatch(addMessageLocal({ threadId: TURN_THREAD, message: USER_MESSAGE }));
    store.dispatch(beginInferenceTurn({ threadId: TURN_THREAD }));
    for (const step of asyncSteps) fire(listeners, step);
    act(() => listeners.onDone?.(DONE_EVENT));
    await waitFor(() =>
      expect(store.getState().chatRuntime.toolTimelineByThread[TURN_THREAD]?.[1]?.id).toBe(
        'subagent:sub-1'
      )
    );
  }

  it('the delivery reply shows no tool cards of the turn before it', async () => {
    vi.mocked(threadApi.getTurnState).mockResolvedValue(snapshot(TURN_REQUEST, 'running') as never);
    const listeners = renderProvider();
    await settleAsyncTurn(listeners);
    const turnCards = toolCards();
    expect(turnCards.every(([messageId]) => messageId === `agent:${TURN_REQUEST}`)).toBe(true);

    // The child finishes; its result arrives as a host-authored turn: a bare
    // `chat_done` with no `inference_start`.
    fire(listeners, doneStep);
    vi.mocked(threadApi.getTurnState).mockResolvedValue(snapshot(TURN_REQUEST, 'success') as never);
    act(() =>
      listeners.onDone?.({
        ...DONE_EVENT,
        request_id: DELIVERY_REQUEST,
        full_response: 'The research finished.',
      })
    );
    await waitFor(() =>
      expect(
        store
          .getState()
          .thread.messagesByThreadId[
            TURN_THREAD
          ]?.some(message => message.id === `agent:${DELIVERY_REQUEST}`)
      ).toBe(true)
    );
    // Let the delivery's completed-snapshot hydrate land.
    await waitFor(() =>
      expect(store.getState().chatRuntime.toolTimelineByThread[TURN_THREAD]?.[1]?.status).toBe(
        'success'
      )
    );

    const cards = toolCards();
    expect(cards.filter(([messageId]) => messageId === `agent:${DELIVERY_REQUEST}`)).toEqual([]);
    expect(store.getState().chatRuntime.settledTurnsByThread[TURN_THREAD]?.[DELIVERY_REQUEST]).toBe(
      undefined
    );
    // The turn that ran the delegation keeps its own trail.
    expect(cards.filter(([messageId]) => messageId === `agent:${TURN_REQUEST}`).length).toBe(
      turnCards.length
    );
  });

  it("a nested spawn reported after its turn settled does not hand that turn's rows to the delivery", async () => {
    // A detached child that delegates again reports the nested spawn on its
    // parent turn's channel, minutes after that turn settled
    // (`attach_parent` hands it the turn's progress sender).
    vi.mocked(threadApi.getTurnState).mockResolvedValue(snapshot(TURN_REQUEST, 'running') as never);
    const listeners = renderProvider();
    await settleAsyncTurn(listeners);
    fire(listeners, {
      listener: 'onSubagentSpawned',
      event: {
        thread_id: TURN_THREAD,
        request_id: TURN_REQUEST,
        round: 2,
        tool_name: 'researcher',
        skill_id: 'sub-nested',
        message: '',
        seq: 99,
        subagent: { mode: 'async' },
      },
    } as SocketStep);
    act(() =>
      listeners.onDone?.({
        ...DONE_EVENT,
        request_id: DELIVERY_REQUEST,
        full_response: 'The research finished.',
      })
    );
    await waitFor(() =>
      expect(
        store
          .getState()
          .thread.messagesByThreadId[
            TURN_THREAD
          ]?.some(message => message.id === `agent:${DELIVERY_REQUEST}`)
      ).toBe(true)
    );
    expect(toolCards().filter(([messageId]) => messageId === `agent:${DELIVERY_REQUEST}`)).toEqual(
      []
    );
  });

  it('a late tool call of a settled turn renders on that turn, not on the live one', async () => {
    // Through the real socket listeners: req-1 settled, req-2 is live and
    // calls a tool, then req-1's late tool_call and tool_result arrive (a
    // bridge that had not drained). The late call renders settled on req-1's
    // message and never on req-2's.
    vi.mocked(threadApi.getTurnState).mockResolvedValue(snapshot(TURN_REQUEST, 'running') as never);
    const listeners = renderProvider();
    await settleAsyncTurn(listeners);
    vi.mocked(threadApi.getTurnState).mockResolvedValue(null);
    const second = { thread_id: TURN_THREAD, request_id: 'req-2' };
    const late = { thread_id: TURN_THREAD, request_id: TURN_REQUEST };
    fire(listeners, { listener: 'onInferenceStart', event: { ...second } } as SocketStep);
    fire(listeners, {
      listener: 'onToolCall',
      event: { ...second, round: 1, tool_name: 'web_search', args: {}, tool_call_id: 'call-r2' },
    } as SocketStep);
    fire(listeners, {
      listener: 'onToolCall',
      event: { ...late, round: 4, tool_name: 'calendar_list', args: {}, tool_call_id: 'call-late' },
    } as SocketStep);
    fire(listeners, {
      listener: 'onToolResult',
      event: {
        ...late,
        round: 4,
        tool_name: 'calendar_list',
        output: 'done',
        success: true,
        tool_call_id: 'call-late',
      },
    } as SocketStep);
    act(() =>
      listeners.onDone?.({ ...DONE_EVENT, request_id: 'req-2', full_response: 'Second answer.' })
    );
    await waitFor(() =>
      expect(
        store
          .getState()
          .thread.messagesByThreadId[TURN_THREAD]?.some(message => message.id === 'agent:req-2')
      ).toBe(true)
    );

    const callsOn = (messageId: string) => {
      const message = project().find(m => m.id === messageId);
      if (!message || typeof message.content === 'string') return [];
      return message.content.flatMap(part =>
        part.type === 'tool-call'
          ? [[part.toolCallId, part.result === undefined ? 'running' : 'settled']]
          : []
      );
    };
    expect(callsOn(`agent:${TURN_REQUEST}`)).toContainEqual(['call-late', 'settled']);
    expect(callsOn('agent:req-2').map(([id]) => id)).toEqual(['call-r2']);
  });

  it("an older snapshot arriving after chat_done does not hide the settled turn's own trail", async () => {
    // Snapshot lag: the completed-snapshot fetch returns the PREVIOUS turn's
    // snapshot, so the live rows (and their owner) become that turn's.
    vi.mocked(threadApi.getTurnState).mockResolvedValue(snapshot('req-older', 'success') as never);
    const listeners = renderProvider();
    await store.dispatch(addMessageLocal({ threadId: TURN_THREAD, message: USER_MESSAGE }));
    store.dispatch(beginInferenceTurn({ threadId: TURN_THREAD }));
    for (const step of LIVE_TURN_STEPS) fire(listeners, step);
    const live = toolCards().map(([, toolName, status]) => [toolName, status]);
    act(() => listeners.onDone?.(DONE_EVENT));
    await waitFor(() =>
      expect(store.getState().chatRuntime.toolTimelineRequestByThread[TURN_THREAD]).toBe(
        'req-older'
      )
    );

    // Same cards, same statuses, on the settled message — nothing hidden.
    const settled = toolCards();
    expect(settled.every(([messageId]) => messageId === `agent:${TURN_REQUEST}`)).toBe(true);
    expect(settled.map(([, toolName, status]) => [toolName, status])).toEqual(live);
  });
});
