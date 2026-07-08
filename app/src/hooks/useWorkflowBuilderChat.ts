/**
 * useWorkflowBuilderChat — drives the Flows prompt bar and canvas copilot by
 * running the `workflow_builder` agent server-side. It owns a DEDICATED thread
 * (created lazily on first send) so an authoring conversation never collides
 * with the user's main chat, sends a STRUCTURED turn request to
 * `openhuman.flows_build` (which renders the brief and runs the agent), and
 * surfaces the returned `WorkflowProposal` on this thread.
 *
 * The builder is now a first-class backend agent (like the Flow Scout): the core
 * constructs the prompt and drives the agent to completion. Phase B streams that
 * turn onto the copilot's dedicated thread (text / thinking / tool events +
 * a terminal `chat_done`), so this hook passes its `threadId` into
 * `openhuman.flows_build` and lets the GLOBAL `ChatRuntimeProvider` own the
 * transcript: the provider appends the final assistant message on `chat_done`
 * and populates `streamingAssistantByThread` / `toolTimelineByThread` /
 * `pendingWorkflowProposalsByThread` for this thread as the turn runs. This hook
 * only appends the local USER turn (the web channel never persists user
 * messages) and reads the streamed state back out; the blocking
 * `{proposal, error}` return is a fallback for when streaming isn't wired
 * (CLI / tests / a missed socket event).
 *
 * Invariant: `create`/`revise`/`repair` never persist; only a `build` turn (with
 * a real flow id) may save onto an existing flow. Nothing here enables a flow.
 */
import createDebug from 'debug';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { type BuilderTurnRequest, buildWorkflow } from '../services/api/flowsApi';
import { store } from '../store';
import {
  clearWorkflowProposalForThread,
  fetchAndHydrateTurnHistory,
  fetchAndHydrateTurnState,
  setWorkflowProposalForThread,
  type ToolTimelineEntry,
  type WorkflowProposal,
} from '../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';
import {
  addMessageLocal,
  createNewThread,
  loadThreadMessages,
  THREAD_NOT_FOUND_MESSAGE,
} from '../store/threadSlice';
import type { ThreadMessage } from '../types/thread';

const log = createDebug('app:flows:builder-chat');

/** A single builder turn: what the user sees vs. the structured turn request. */
export interface WorkflowBuilderSendParams {
  /** Human-readable text shown as the user's message in the thread transcript. */
  displayText: string;
  /**
   * The structured builder-turn request. The core renders the agent's brief
   * from this and runs `workflow_builder` directly (via `openhuman.flows_build`)
   * — the frontend no longer crafts delegate prompt strings.
   */
  request: BuilderTurnRequest;
}

export interface UseWorkflowBuilderChat {
  /** The dedicated thread id, or `null` before the first send creates it. */
  threadId: string | null;
  /** True while a builder turn is in flight on this thread. */
  sending: boolean;
  /** The latest proposal the agent returned on this thread, or `null`. */
  proposal: WorkflowProposal | null;
  /**
   * The dedicated thread's FULL transcript (user + agent turns, including
   * between-tool narration bubbles), so a caller that needs the complete
   * history (e.g. persistence/rehydration) can still get it. Empty until the
   * first send. Sourced from the same `messagesByThreadId` store the main chat
   * transcript reads.
   */
  messages: ThreadMessage[];
  /**
   * `messages` filtered for RENDERING as chat bubbles: drops agent messages
   * tagged `extraMetadata.isInterim` (the between-tool "Let me check…/Now let
   * me build…" narration `ChatRuntimeProvider`'s `onInterim` persists) since
   * that narration already renders live via `toolTimeline`/`liveResponse`
   * below — showing it again as a bubble double-renders it. User messages and
   * any non-interim agent message (the turn's terminal answer, including a
   * clarifying question appended via the `assistantText` fallback in `send`)
   * are always kept.
   */
  displayMessages: ThreadMessage[];
  /**
   * The dedicated thread's live tool timeline (streamed by `ChatRuntimeProvider`
   * as the builder turn runs) — bound straight into the shared
   * `ToolTimelineBlock`. Empty when nothing has streamed on this thread.
   */
  toolTimeline: ToolTimelineEntry[];
  /**
   * The builder turn's in-flight assistant text (the shared streaming lane), for
   * `ToolTimelineBlock`'s `liveResponse`. Empty string once the turn settles —
   * the final answer then lives in `messages`.
   */
  liveResponse: string;
  /** Last send error (thread create / RPC failure), or `null`. */
  error: string | null;
  /**
   * Send a builder turn, creating the dedicated thread on first use. Resolves
   * with `proposed: true` iff this turn's `flows_build` call returned a
   * proposal — `false` for a clarifying question, an error, or a call that
   * never ran (already sending / offline). Callers that loop a conversation
   * (the copilot's free-text follow-ups) use this to know whether the turn's
   * instruction is still "unresolved" and must be carried into the next turn
   * — see `WorkflowCopilotPanel`'s `pendingAskRef`.
   */
  send: (params: WorkflowBuilderSendParams) => Promise<{ proposed: boolean }>;
  /** Clear the current proposal (e.g. after Accept/Reject) without persisting. */
  clearProposal: () => void;
}

const EMPTY_MESSAGES: ThreadMessage[] = [];
const EMPTY_TIMELINE: ToolTimelineEntry[] = [];

/**
 * @param seedThreadId Optional existing thread to bind to instead of creating
 *   a fresh one — lets a caller reuse a thread across mounts. When this
 *   identifies a genuinely pre-existing thread (i.e. not one `send()` just
 *   created on this hook instance — see `createdThreadIdRef`), this hook
 *   rehydrates that thread's messages + turn state/history from the core on
 *   mount (mirroring `Conversations.tsx`'s thread-switch effect) so a
 *   persisted copilot thread (`workflowCopilotThreads.ts`) resumes its full
 *   transcript after a reload instead of starting empty (issue: Copilot chat
 *   not persistent).
 */
export function useWorkflowBuilderChat(seedThreadId?: string | null): UseWorkflowBuilderChat {
  const dispatch = useAppDispatch();
  const socketStatus = useAppSelector(selectSocketStatus);
  const [threadId, setThreadId] = useState<string | null>(seedThreadId ?? null);
  const [localSending, setLocalSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Tracks a thread id this hook created itself via `send()`'s `createNewThread`
  // call — as opposed to one that arrived from `seedThreadId` because a caller
  // (e.g. `WorkflowCopilotPanel`) reports every `threadId` change back up via
  // `onThreadIdChange` and re-passes it in as `seedThreadId` on the next
  // render. Without this distinction, the rehydrate effect below would treat
  // that echo as "an existing persisted thread just got selected" and refetch
  // it from the core mid-turn — a redundant `loadThreadMessages` GET racing
  // the in-flight turn's own append. `loadThreadMessages.fulfilled` merges a
  // locally-appended message that predates the fetch's snapshot back in
  // (rather than wholesale-replacing), so this guard is defense-in-depth
  // against the unnecessary refetch itself, not a correctness requirement.
  const createdThreadIdRef = useRef<string | null>(null);

  const proposalsByThread = useAppSelector(
    state => state.chatRuntime.pendingWorkflowProposalsByThread
  );
  const messagesByThreadId = useAppSelector(state => state.thread.messagesByThreadId);
  const toolTimelineByThread = useAppSelector(state => state.chatRuntime.toolTimelineByThread);
  const streamingAssistantByThread = useAppSelector(
    state => state.chatRuntime.streamingAssistantByThread
  );

  // Prefer the runtime's streamed proposal (populated on this thread by
  // `ChatRuntimeProvider` as the builder's `propose_workflow`/`revise_workflow`
  // tool result lands); the blocking `send` result is only a fallback that
  // writes into the same slice.
  const proposal = useMemo(
    () => (threadId ? (proposalsByThread[threadId] ?? null) : null),
    [threadId, proposalsByThread]
  );

  const messages = useMemo(
    () => (threadId ? (messagesByThreadId[threadId] ?? EMPTY_MESSAGES) : EMPTY_MESSAGES),
    [threadId, messagesByThreadId]
  );

  // Render-layer filter: drop interim narration bubbles (already shown live
  // via `toolTimeline`/`liveResponse`), keeping every user turn and every
  // non-interim agent turn (the terminal answer for a round, including a
  // clarifying question with no `isInterim` tag). `messages` itself stays the
  // full set — rehydration (below) and any future persistence need it intact.
  const displayMessages = useMemo(
    () => messages.filter(m => m.sender === 'user' || !m.extraMetadata?.isInterim),
    [messages]
  );

  const toolTimeline = useMemo(
    () => (threadId ? (toolTimelineByThread[threadId] ?? EMPTY_TIMELINE) : EMPTY_TIMELINE),
    [threadId, toolTimelineByThread]
  );

  const liveResponse = useMemo(
    () => (threadId ? (streamingAssistantByThread[threadId]?.content ?? '') : ''),
    [threadId, streamingAssistantByThread]
  );

  // Rehydrate a persisted thread's transcript + turn state/history from the
  // core on mount. Messages ARE durable server-side (`threadApi.appendMessage`
  // persists every turn) — redux-persist only whitelists `selectedThreadId`
  // for the thread slice, so `messagesByThreadId` starts empty on a fresh app
  // load and this thread's messages would otherwise never come back.
  //
  // Skips when `seedThreadId` is one THIS hook created via `send()` (tracked
  // by `createdThreadIdRef`): `WorkflowCopilotPanel` reports every `threadId`
  // change back up through `onThreadIdChange`, and a caller like
  // `FlowCanvasPage` re-passes that straight back in as `seedThreadId` on the
  // next render — so `seedThreadId` also changes right after a fresh thread is
  // created by a first send, not just when a truly pre-existing persisted
  // thread is (re)selected. Rehydrating in that case would be a wasted GET
  // racing the in-flight turn's own append; `loadThreadMessages.fulfilled`
  // merges rather than wholesale-replaces (see `threadSlice.ts`), so a
  // straggler fetch can no longer wipe out a newer local append, but skipping
  // it here still avoids the redundant round trip.
  useEffect(() => {
    if (!seedThreadId) return;
    if (createdThreadIdRef.current === seedThreadId) {
      log('rehydrate: skipping — this hook created thread=%s locally', seedThreadId);
      return;
    }
    log('rehydrate: loading persisted messages + turn state/history thread=%s', seedThreadId);
    // A persisted seed (`workflowCopilotThreads.ts`) can point at a thread
    // that no longer exists (e.g. deleted/purged since it was cached).
    // `loadThreadMessages` evicts the stale thread from Redux in that case
    // and rejects with `THREAD_NOT_FOUND_MESSAGE` — null out this hook's
    // `threadId` in response so the effect below that reports it back up
    // (`WorkflowCopilotPanel` -> `onThreadIdChange` -> `FlowCanvasPage`)
    // clears the stale cached id too, letting the next `send()` create a
    // fresh thread instead of retrying the dead one forever.
    void dispatch(loadThreadMessages(seedThreadId)).then((action: { payload?: unknown }) => {
      if (action?.payload === THREAD_NOT_FOUND_MESSAGE) {
        log(
          'rehydrate: thread=%s no longer exists — clearing seed so a future send starts fresh',
          seedThreadId
        );
        setThreadId(current => (current === seedThreadId ? null : current));
      }
    });
    void dispatch(fetchAndHydrateTurnState(seedThreadId));
    void dispatch(fetchAndHydrateTurnHistory(seedThreadId));
  }, [seedThreadId, dispatch]);

  // The turn is a single request/response RPC (no streaming runtime), so
  // "sending" is simply whether that call is in flight.
  const sending = localSending;

  const send = useCallback(
    async ({ displayText, request }: WorkflowBuilderSendParams) => {
      if (localSending) {
        log('send: ignored — a turn is already dispatching');
        return { proposed: false };
      }
      if (socketStatus !== 'connected') {
        log('send: blocked — socket not connected (%s)', socketStatus);
        setError('offline');
        return { proposed: false };
      }
      setLocalSending(true);
      setError(null);
      let targetThreadId = threadId;
      let proposed = false;
      try {
        if (!targetThreadId) {
          log('send: creating dedicated builder thread');
          const thread = await dispatch(createNewThread(['workflow-builder'])).unwrap();
          targetThreadId = thread.id;
          createdThreadIdRef.current = targetThreadId;
          setThreadId(targetThreadId);
        }
        // A fresh turn supersedes any prior proposal on this thread.
        dispatch(clearWorkflowProposalForThread({ threadId: targetThreadId }));

        const userMessage: ThreadMessage = {
          id: `msg_${globalThis.crypto.randomUUID()}`,
          content: displayText,
          type: 'text',
          extraMetadata: {},
          sender: 'user',
          createdAt: new Date().toISOString(),
        };
        await dispatch(
          addMessageLocal({ threadId: targetThreadId, message: userMessage })
        ).unwrap();

        // Run the workflow_builder agent server-side, streaming its turn onto
        // this thread (Phase B): passing `targetThreadId` makes the core emit
        // text/thinking/tool events + a terminal `chat_done` keyed by it. The
        // GLOBAL `ChatRuntimeProvider` owns that transcript — it appends the
        // final assistant message on `chat_done` and fills the streaming/tool
        // slices as the turn runs, so in the normal (streaming-wired) case this
        // hook must NOT also append the agent reply (doing so would double
        // it) — see the dedup check below. We still await the blocking result
        // for its `proposal`/`error`/`assistantText` fallback.
        log('send: running flows_build thread=%s mode=%s', targetThreadId, request.mode);
        const result = await buildWorkflow(request, targetThreadId);

        // Surface the proposal via the same store slice the streamed path used,
        // so `WorkflowProposalCard` / the copilot preview render unchanged. This
        // is a fallback: when streaming is wired the runtime already populated
        // `pendingWorkflowProposalsByThread` from the tool result; re-writing the
        // same value here is idempotent and covers a missed socket event / CLI.
        if (result.proposal) {
          proposed = true;
          dispatch(
            setWorkflowProposalForThread({ threadId: targetThreadId, proposal: result.proposal })
          );
        } else if (result.error) {
          setError(result.error);
        } else if (result.assistantText?.trim()) {
          // Neither a proposal nor an error: the agent replied with plain
          // text instead of proposing this turn — most commonly a clarifying
          // question (the "ask" branch of the clarify/verify posture). When
          // streaming is wired (the normal case) `ChatRuntimeProvider` already
          // appended this exact text on the turn's `chat_done` — the Rust
          // side (`finalize_flow_stream`) delivers it unconditionally,
          // independent of whether a proposal was made — so re-appending here
          // would double the bubble. Read the live store (not the stale
          // closed-over `messages`) to check whether that already landed;
          // only append when it hasn't, which is the actual fallback case
          // (streaming not wired: CLI / tests / a missed socket event).
          const latest = store.getState().thread.messagesByThreadId[targetThreadId] ?? [];
          const lastMessage = latest[latest.length - 1];
          const alreadyStreamed =
            lastMessage?.sender === 'agent' && lastMessage.content === result.assistantText;
          log(
            'send: assistantText fallback thread=%s alreadyStreamed=%s',
            targetThreadId,
            alreadyStreamed
          );
          if (!alreadyStreamed) {
            const assistantMessage: ThreadMessage = {
              id: `msg_${globalThis.crypto.randomUUID()}`,
              content: result.assistantText,
              type: 'text',
              extraMetadata: {},
              sender: 'agent',
              createdAt: new Date().toISOString(),
            };
            dispatch(addMessageLocal({ threadId: targetThreadId, message: assistantMessage }));
          }
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('send: failed err=%o', err);
        setError(msg);
        // The pre-existing/seeded thread this turn targeted no longer exists
        // server-side (deleted/purged since it was cached) — `addMessageLocal`
        // already evicted it from Redux; clear it here too so the next send
        // creates a fresh thread instead of retrying the dead one forever.
        // Scoped to `targetThreadId === threadId` (the thread this hook was
        // seeded/already bound to) so a failure on a thread just created this
        // same call doesn't get misattributed.
        if (msg === THREAD_NOT_FOUND_MESSAGE && targetThreadId === threadId) {
          log(
            'send: thread=%s no longer exists — clearing cached id so the next send starts fresh',
            targetThreadId
          );
          setThreadId(null);
        }
      } finally {
        setLocalSending(false);
      }
      return { proposed };
    },
    [dispatch, localSending, socketStatus, threadId]
  );

  const clearProposal = useCallback(() => {
    if (threadId) dispatch(clearWorkflowProposalForThread({ threadId }));
  }, [dispatch, threadId]);

  return {
    threadId,
    sending,
    proposal,
    messages,
    displayMessages,
    toolTimeline,
    liveResponse,
    error,
    send,
    clearProposal,
  };
}
