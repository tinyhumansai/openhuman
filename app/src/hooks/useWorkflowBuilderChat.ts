/**
 * useWorkflowBuilderChat (Phase 5c) — a thin driver around the existing chat
 * runtime for the Flows prompt bar and canvas copilot. It owns a DEDICATED
 * thread (created lazily on first send) so a workflow-authoring conversation
 * never collides with the user's main chat, sends turns phrased to route to the
 * `workflow_builder` specialist (see `lib/flows/workflowBuilderPrompt.ts`), and
 * exposes the resulting `WorkflowProposal` the global `ChatRuntimeProvider`
 * parses onto this thread.
 *
 * It deliberately does NOT reimplement the chat runtime: the same
 * `addMessageLocal` → `chatSend` path and the same `pendingWorkflowProposalsByThread`
 * store slice that `Conversations.tsx` uses drive this. The only new concept is
 * per-surface thread scoping.
 *
 * Invariant: nothing here persists or enables a flow. The proposal is
 * validate-only; saving stays behind the explicit `WorkflowProposalCard`
 * "Save & enable" click.
 */
import createDebug from 'debug';
import { useCallback, useMemo, useState } from 'react';

import { chatSend } from '../services/chatService';
import {
  beginInferenceTurn,
  clearRuntimeForThread,
  clearWorkflowProposalForThread,
  setToolTimelineForThread,
  type WorkflowProposal,
} from '../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';
import {
  addMessageLocal,
  clearThreadInferenceActive,
  createNewThread,
  markThreadInferenceActive,
} from '../store/threadSlice';
import type { ThreadMessage } from '../types/thread';

const log = createDebug('app:flows:builder-chat');

/** A single builder turn: what the user sees vs. what the agent receives. */
export interface WorkflowBuilderSendParams {
  /** Human-readable text shown as the user's message in the thread transcript. */
  displayText: string;
  /** The full delegation prompt actually sent to the core (may inject graph/context). */
  prompt: string;
}

export interface UseWorkflowBuilderChat {
  /** The dedicated thread id, or `null` before the first send creates it. */
  threadId: string | null;
  /** True while a builder turn is in flight on this thread. */
  sending: boolean;
  /** The latest proposal the agent returned on this thread, or `null`. */
  proposal: WorkflowProposal | null;
  /** Last send error (thread create / RPC failure), or `null`. */
  error: string | null;
  /** Send a builder turn, creating the dedicated thread on first use. */
  send: (params: WorkflowBuilderSendParams) => Promise<void>;
  /** Clear the current proposal (e.g. after Accept/Reject) without persisting. */
  clearProposal: () => void;
}

/**
 * @param seedThreadId Optional existing thread to bind to instead of creating a
 *   fresh one — lets a caller reuse a thread across mounts (unused today; the
 *   prompt bar and copilot each start clean).
 */
export function useWorkflowBuilderChat(seedThreadId?: string | null): UseWorkflowBuilderChat {
  const dispatch = useAppDispatch();
  const socketStatus = useAppSelector(selectSocketStatus);
  const [threadId, setThreadId] = useState<string | null>(seedThreadId ?? null);
  const [localSending, setLocalSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const activeThreadIds = useAppSelector(state => state.thread.activeThreadIds);
  const proposalsByThread = useAppSelector(
    state => state.chatRuntime.pendingWorkflowProposalsByThread
  );

  const proposal = useMemo(
    () => (threadId ? (proposalsByThread[threadId] ?? null) : null),
    [threadId, proposalsByThread]
  );

  // "Sending" = we're mid-dispatch OR the runtime still marks the thread active.
  const runtimeActive = threadId ? Boolean(activeThreadIds[threadId]) : false;
  const sending = localSending || runtimeActive;

  const send = useCallback(
    async ({ displayText, prompt }: WorkflowBuilderSendParams) => {
      if (localSending) {
        log('send: ignored — a turn is already dispatching');
        return;
      }
      if (socketStatus !== 'connected') {
        log('send: blocked — socket not connected (%s)', socketStatus);
        setError('offline');
        return;
      }
      setLocalSending(true);
      setError(null);
      // Declared outside the try so the catch block can see a thread created
      // during THIS call — `threadId` state doesn't update synchronously
      // within the same closure invocation, so a failure after creation (but
      // before this call returns) would otherwise see the stale `null` and
      // skip cleanup, leaving that new thread's active markers dangling.
      let targetThreadId = threadId;
      try {
        if (!targetThreadId) {
          log('send: creating dedicated builder thread');
          const thread = await dispatch(createNewThread(['workflow-builder'])).unwrap();
          targetThreadId = thread.id;
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

        dispatch(setToolTimelineForThread({ threadId: targetThreadId, entries: [] }));
        dispatch(beginInferenceTurn({ threadId: targetThreadId }));
        dispatch(markThreadInferenceActive(targetThreadId));

        log('send: dispatching builder turn thread=%s', targetThreadId);
        await chatSend({ threadId: targetThreadId, message: prompt });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('send: failed err=%o', err);
        setError(msg);
        // The runtime never got a turn to end, so release the active markers we
        // optimistically set (guarded: targetThreadId is still null only when
        // thread creation itself failed, in which case there's nothing to clear).
        if (targetThreadId) {
          dispatch(clearRuntimeForThread({ threadId: targetThreadId }));
          dispatch(clearThreadInferenceActive(targetThreadId));
        }
      } finally {
        setLocalSending(false);
      }
    },
    [dispatch, localSending, socketStatus, threadId]
  );

  const clearProposal = useCallback(() => {
    if (threadId) dispatch(clearWorkflowProposalForThread({ threadId }));
  }, [dispatch, threadId]);

  return { threadId, sending, proposal, error, send, clearProposal };
}
