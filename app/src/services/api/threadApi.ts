import debug from 'debug';

import type {
  DerivedTranscriptGetOptions,
  DerivedTranscriptPage,
} from '../../types/derivedTranscript';
import type {
  PurgeResultData,
  Thread,
  ThreadDeleteData,
  ThreadMessage,
  ThreadMessagesData,
  ThreadSearchHit,
  ThreadsListData,
} from '../../types/thread';
import type {
  AgentRun,
  AgentRunGetResponse,
  AgentRunListResponse,
  ClearTurnStateResponse,
  GetTurnStateResponse,
  ListTurnStatesResponse,
  PersistedTurnState,
  RunEvent,
  RunEventListResponse,
} from '../../types/turnState';
import type { ChatThreadTodoItem, ThreadGoal } from '../chatService';
import type { ContentPart } from '../chatWire';
import { callCoreRpc } from '../coreRpcClient';
import { attachArtifactRows } from './artifactRows';

interface Envelope<T> {
  data?: T;
}

function unwrapEnvelope<T>(response: Envelope<T> | T): T {
  if (response && typeof response === 'object' && 'data' in response) {
    const envelope = response as Envelope<T>;
    if (envelope.data === undefined) {
      throw new Error('RPC envelope contains undefined data');
    }
    return envelope.data;
  }
  return response as T;
}

const generateTitleLog = debug('threadApi.generateTitleIfNeeded');

/**
 * The core's `sender` vocabulary is `user` | `agent`, but some core writers
 * stored the assistant side as `assistant` (legacy system writers and
 * channel-session mirrors). Fold that alias onto `agent` at the transport
 * boundary so every `sender === 'agent'` check in the renderers — and the
 * assistant-ui role mapping — treats such a row as the assistant instead of
 * painting it as a user turn.
 */
function normalizeThreadMessage(message: ThreadMessage): ThreadMessage {
  return (message.sender as string) === 'assistant' ? { ...message, sender: 'agent' } : message;
}

export const threadApi = {
  createNewThread: async (labels?: string[]): Promise<Thread> => {
    const response = await callCoreRpc<Envelope<Thread>>({
      method: 'openhuman.threads_create_new',
      params: { labels },
    });
    return unwrapEnvelope(response);
  },

  getThreads: async (): Promise<ThreadsListData> => {
    const response = await callCoreRpc<Envelope<ThreadsListData>>({
      method: 'openhuman.threads_list',
    });
    return unwrapEnvelope(response);
  },

  getThreadMessages: async (threadId: string): Promise<ThreadMessagesData> => {
    const response = await callCoreRpc<Envelope<ThreadMessagesData>>({
      method: 'openhuman.threads_messages_list',
      params: { thread_id: threadId },
    });
    const data = unwrapEnvelope(response);
    // Artifact rows place artifact cards; they are attached to their replies
    // here so no caller ever sees them as messages.
    const messages = attachArtifactRows(data.messages.map(normalizeThreadMessage));
    return { ...data, messages, count: messages.length };
  },

  /** Message text search across every thread, newest first. */
  searchMessages: async (query: string, limit?: number): Promise<ThreadSearchHit[]> => {
    const response = await callCoreRpc<Envelope<{ hits?: ThreadSearchHit[] }>>({
      method: 'openhuman.threads_search',
      params: { query, limit },
    });
    return unwrapEnvelope(response)?.hits ?? [];
  },

  appendMessage: async (
    threadId: string,
    message: ThreadMessage,
    attachments?: ContentPart[]
  ): Promise<ThreadMessage> => {
    const response = await callCoreRpc<Envelope<ThreadMessage>>({
      method: 'openhuman.threads_message_append',
      params: {
        thread_id: threadId,
        message,
        ...(attachments && attachments.length > 0 ? { attachments } : {}),
      },
    });
    return normalizeThreadMessage(unwrapEnvelope(response));
  },

  generateTitleIfNeeded: async (threadId: string, assistantMessage?: string): Promise<Thread> => {
    generateTitleLog('enter threadId=%s assistantMessage=%o', threadId, assistantMessage);
    try {
      const response = await callCoreRpc<Envelope<Thread>>({
        method: 'openhuman.threads_generate_title',
        params: { thread_id: threadId, assistant_message: assistantMessage },
      });
      const thread = unwrapEnvelope(response);
      generateTitleLog('success threadId=%s response=%o thread=%o', threadId, response, thread);
      return thread;
    } catch (error) {
      generateTitleLog(
        'error threadId=%s assistantMessage=%o error=%O',
        threadId,
        assistantMessage,
        error
      );
      throw error;
    }
  },

  updateMessage: async (
    threadId: string,
    messageId: string,
    extraMetadata: Record<string, unknown>
  ): Promise<ThreadMessage> => {
    const response = await callCoreRpc<Envelope<ThreadMessage>>({
      method: 'openhuman.threads_message_update',
      params: { thread_id: threadId, message_id: messageId, extra_metadata: extraMetadata },
    });
    return normalizeThreadMessage(unwrapEnvelope(response));
  },

  deleteThread: async (threadId: string): Promise<ThreadDeleteData> => {
    const response = await callCoreRpc<Envelope<ThreadDeleteData>>({
      method: 'openhuman.threads_delete',
      params: { thread_id: threadId, deleted_at: new Date().toISOString() },
    });
    return unwrapEnvelope(response);
  },

  purge: async (): Promise<PurgeResultData> => {
    const response = await callCoreRpc<Envelope<PurgeResultData>>({
      method: 'openhuman.threads_purge',
    });
    return unwrapEnvelope(response);
  },

  getTurnState: async (threadId: string): Promise<PersistedTurnState | null> => {
    const response = await callCoreRpc<{ data?: GetTurnStateResponse }>({
      method: 'openhuman.threads_turn_state_get',
      params: { thread_id: threadId },
    });
    const data = unwrapEnvelope(response);
    return data?.turnState ?? null;
  },

  listTurnStates: async (): Promise<PersistedTurnState[]> => {
    const response = await callCoreRpc<{ data?: ListTurnStatesResponse }>({
      method: 'openhuman.threads_turn_state_list',
    });
    const data = unwrapEnvelope(response);
    return data?.turnStates ?? [];
  },

  /**
   * Per-turn history for one thread, newest first — each turn's own tool
   * timeline (Phase 4). Cheap enough to call on thread open; full timelines can
   * be lazily re-fetched per turn via {@link getTurnStateForRequest}.
   */
  getTurnStateHistory: async (threadId: string): Promise<PersistedTurnState[]> => {
    const response = await callCoreRpc<{ data?: ListTurnStatesResponse }>({
      method: 'openhuman.threads_turn_state_history',
      params: { thread_id: threadId },
    });
    const data = unwrapEnvelope(response);
    return data?.turnStates ?? [];
  },

  /**
   * The thread's current live todo list, for thread open / reconnect (the
   * `thread_todos_changed` socket event covers every update after that).
   * Wire method `openhuman.threads_todos_get`.
   */
  getTodos: async (threadId: string): Promise<ChatThreadTodoItem[]> => {
    const response = await callCoreRpc<{ data?: { todos?: ChatThreadTodoItem[] } }>({
      method: 'openhuman.threads_todos_get',
      params: { thread_id: threadId },
    });
    const data = unwrapEnvelope(response);
    return data?.todos ?? [];
  },

  /**
   * The thread's current goal (or `null`), for thread open / reconnect (the
   * `thread_goal_updated` / `thread_goal_cleared` socket events cover every
   * update after that). Wire method `openhuman.threads_goal_get`.
   */
  getGoal: async (threadId: string): Promise<ThreadGoal | null> => {
    const response = await callCoreRpc<{ data?: { goal?: ThreadGoal | null } }>({
      method: 'openhuman.threads_goal_get',
      params: { thread_id: threadId },
    });
    const data = unwrapEnvelope(response);
    return data?.goal ?? null;
  },

  /** One specific past turn of a thread, by its producing request id (Phase 4). */
  getTurnStateForRequest: async (
    threadId: string,
    requestId: string
  ): Promise<PersistedTurnState | null> => {
    const response = await callCoreRpc<{ data?: GetTurnStateResponse }>({
      method: 'openhuman.threads_turn_state_get_turn',
      params: { thread_id: threadId, request_id: requestId },
    });
    const data = unwrapEnvelope(response);
    return data?.turnState ?? null;
  },

  clearTurnState: async (threadId: string): Promise<boolean> => {
    const response = await callCoreRpc<{ data?: ClearTurnStateResponse }>({
      method: 'openhuman.threads_turn_state_clear',
      params: { thread_id: threadId },
    });
    const data = unwrapEnvelope(response);
    return Boolean(data?.cleared);
  },

  listRuns: async (filters?: {
    status?: string;
    kind?: string;
    parentRunId?: string;
    parentThreadId?: string;
    limit?: number;
    offset?: number;
  }): Promise<AgentRun[]> => {
    const response = await callCoreRpc<{ data?: AgentRunListResponse }>({
      method: 'openhuman.run_ledger_list',
      params: filters ?? {},
    });
    const data = unwrapEnvelope(response);
    return data?.runs ?? [];
  },

  getRun: async (id: string): Promise<AgentRun | null> => {
    const response = await callCoreRpc<{ data?: AgentRunGetResponse }>({
      method: 'openhuman.run_ledger_get',
      params: { id },
    });
    const data = unwrapEnvelope(response);
    return data?.run ?? null;
  },

  listRunEvents: async (
    runId: string,
    options?: { afterSequence?: number; limit?: number }
  ): Promise<RunEvent[]> => {
    const response = await callCoreRpc<{ data?: RunEventListResponse }>({
      method: 'openhuman.run_ledger_events',
      params: { runId, ...options },
    });
    const data = unwrapEnvelope(response);
    return data?.events ?? [];
  },

  updateLabels: async (threadId: string, labels: string[]): Promise<Thread> => {
    const response = await callCoreRpc<Envelope<Thread>>({
      method: 'openhuman.threads_update_labels',
      params: { thread_id: threadId, labels },
    });
    return unwrapEnvelope(response);
  },

  /**
   * Bind (absolute path) or clear (`''`) the working folder of a thread that
   * has no messages yet. The core validates the folder and refuses once the
   * conversation has started.
   */
  updateWorkingDir: async (threadId: string, actionDir: string): Promise<Thread> => {
    const response = await callCoreRpc<Envelope<Thread>>({
      method: 'openhuman.threads_update_working_dir',
      params: { thread_id: threadId, action_dir: actionDir },
    });
    return unwrapEnvelope(response);
  },

  updateTitle: async (threadId: string, title: string): Promise<Thread> => {
    const response = await callCoreRpc<Envelope<Thread>>({
      method: 'openhuman.threads_update_title',
      params: { thread_id: threadId, title },
    });
    return unwrapEnvelope(response);
  },

  /**
   * Transcript-derived view (Phase B/C): project the thread's append-only
   * `session_raw/*.jsonl` source of truth into typed display items for the
   * settled-turn restore path. Newest-first paginated; `cursor` comes from a
   * prior page's `nextCursor`, `limit` defaults to 50 (core-clamped to 500).
   *
   * A thread with no persisted transcript yet returns an empty page with
   * `hasTranscript: false` (not an error) — the caller then falls back to the
   * legacy `turn_state_history` hydration.
   */
  getDerivedTranscript: async (
    threadId: string,
    options?: DerivedTranscriptGetOptions
  ): Promise<DerivedTranscriptPage> => {
    const response = await callCoreRpc<Envelope<DerivedTranscriptPage>>({
      method: 'openhuman.threads_transcript_get',
      params: { thread_id: threadId, cursor: options?.cursor, limit: options?.limit },
    });
    return unwrapEnvelope(response);
  },
};
