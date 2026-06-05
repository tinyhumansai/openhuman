/**
 * Frontend client for the per-board todo CRUD surface
 * (`openhuman.todos_*`). The Rust handlers persist to the same
 * `<workspace>/agent_task_boards/<hex(thread_id)>.json` store used by the
 * agent task board, so user-driven edits made here and agent-driven edits
 * made by the `todo` tool stay in lock-step.
 *
 * Boards are keyed by an arbitrary `thread_id` string — the handlers never
 * check that the thread exists — so a reserved id (see
 * {@link USER_TASKS_THREAD_ID}) backs a personal task board that is not
 * attached to any conversation.
 */
import debug from 'debug';

import type {
  TaskApprovalMode,
  TaskBoard,
  TaskBoardCard,
  TaskBoardCardStatus,
} from '../../types/turnState';
import { callCoreRpc } from '../coreRpcClient';

const log = debug('todosApi');

/**
 * Reserved board id for the user's personal task list — tasks created
 * without attaching to a conversation live here. Thread ids issued by the
 * core are UUID/hex, so this human-readable sentinel never collides.
 */
export const USER_TASKS_THREAD_ID = 'user-tasks';

/**
 * Reserved board id used by the task source ingestion flow. Source-backed
 * tasks land here before they are pulled into an agent workstream.
 */
export const TASK_SOURCES_THREAD_ID = 'task-sources';

/** Wire shape returned by every `todos_*` handler (`TodosSnapshot`). */
interface TodosSnapshotWire {
  threadId?: string | null;
  cards?: TaskBoardCard[];
  markdown?: string;
}

/** Fields accepted when creating a card. */
export interface AddTodoInput {
  threadId: string;
  content: string;
  status?: TaskBoardCardStatus;
  objective?: string | null;
  notes?: string | null;
  assignedAgent?: string | null;
  approvalMode?: TaskApprovalMode | null;
  /** Originating task-source identifiers, stamped onto the promoted card so the
   *  inbox can detect it was already picked up. */
  sourceMetadata?: Record<string, unknown> | null;
}

/** Fields accepted when editing a card. Omitted fields are left unchanged. */
export interface EditTodoInput {
  threadId: string;
  id: string;
  content?: string;
  status?: TaskBoardCardStatus;
  objective?: string | null;
  notes?: string | null;
  blocker?: string | null;
  assignedAgent?: string | null;
  approvalMode?: TaskApprovalMode | null;
  plan?: string[];
  allowedTools?: string[];
  acceptanceCriteria?: string[];
  evidence?: string[];
}

/**
 * Build a `TaskBoard` from a snapshot. The snapshot omits a board-level
 * timestamp, so we derive `updatedAt` from the most-recently-touched card
 * (falling back to "now") purely for ordering in the UI.
 */
function snapshotToBoard(snap: TodosSnapshotWire, fallbackThreadId: string): TaskBoard {
  const cards = snap.cards ?? [];
  const latest = cards.reduce<string>(
    (acc, card) => (card.updatedAt && card.updatedAt > acc ? card.updatedAt : acc),
    ''
  );
  return {
    threadId: snap.threadId ?? fallbackThreadId,
    cards,
    updatedAt: latest || new Date().toISOString(),
  };
}

/**
 * Strip `undefined` params so we only send fields the caller set —
 * `null` is preserved because the edit handler treats it as "clear".
 */
function pruneParams(params: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) out[key] = value;
  }
  return out;
}

export const todosApi = {
  /** List the cards for a board. */
  list: async (threadId: string): Promise<TaskBoard> => {
    log('list threadId=%s', threadId);
    const snap = await callCoreRpc<TodosSnapshotWire>({
      method: 'openhuman.todos_list',
      params: { thread_id: threadId },
    });
    return snapshotToBoard(snap, threadId);
  },

  /** Append a new card to a board. */
  add: async (input: AddTodoInput): Promise<TaskBoard> => {
    log('add threadId=%s', input.threadId);
    const snap = await callCoreRpc<TodosSnapshotWire>({
      method: 'openhuman.todos_add',
      params: pruneParams({
        thread_id: input.threadId,
        content: input.content,
        status: input.status,
        objective: input.objective,
        notes: input.notes,
        assignedAgent: input.assignedAgent,
        approvalMode: input.approvalMode,
        sourceMetadata: input.sourceMetadata,
      }),
    });
    return snapshotToBoard(snap, input.threadId);
  },

  /** Edit an existing card by id. */
  edit: async (input: EditTodoInput): Promise<TaskBoard> => {
    log('edit threadId=%s id=%s', input.threadId, input.id);
    const snap = await callCoreRpc<TodosSnapshotWire>({
      method: 'openhuman.todos_edit',
      params: pruneParams({
        thread_id: input.threadId,
        id: input.id,
        content: input.content,
        status: input.status,
        objective: input.objective,
        notes: input.notes,
        blocker: input.blocker,
        assignedAgent: input.assignedAgent,
        approvalMode: input.approvalMode,
        plan: input.plan,
        allowedTools: input.allowedTools,
        acceptanceCriteria: input.acceptanceCriteria,
        evidence: input.evidence,
      }),
    });
    return snapshotToBoard(snap, input.threadId);
  },

  /** Update only the status of a card. */
  updateStatus: async (
    threadId: string,
    id: string,
    status: TaskBoardCardStatus
  ): Promise<TaskBoard> => {
    log('updateStatus threadId=%s id=%s status=%s', threadId, id, status);
    const snap = await callCoreRpc<TodosSnapshotWire>({
      method: 'openhuman.todos_update_status',
      params: { thread_id: threadId, id, status },
    });
    return snapshotToBoard(snap, threadId);
  },

  /** Link a card to its agent session's conversation thread (drives the board
   *  "View session" jump). Pass `null` to clear the link. */
  setSessionThread: async (
    threadId: string,
    id: string,
    sessionThreadId: string | null
  ): Promise<TaskBoard> => {
    log('setSessionThread threadId=%s id=%s sessionThreadId=%s', threadId, id, sessionThreadId);
    const snap = await callCoreRpc<TodosSnapshotWire>({
      method: 'openhuman.todos_set_session_thread',
      params: { thread_id: threadId, id, sessionThreadId },
    });
    return snapshotToBoard(snap, threadId);
  },

  /** Remove a card by id. */
  remove: async (threadId: string, id: string): Promise<TaskBoard> => {
    log('remove threadId=%s id=%s', threadId, id);
    const snap = await callCoreRpc<TodosSnapshotWire>({
      method: 'openhuman.todos_remove',
      params: { thread_id: threadId, id },
    });
    return snapshotToBoard(snap, threadId);
  },
};
