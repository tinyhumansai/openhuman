/**
 * Task-plan approval action, extracted from `Conversations` so the whole
 * guard → decide → persist → refresh → error-advisory flow is unit-testable
 * without mounting the conversations page.
 */
import debugFactory from 'debug';

import { threadApi } from '../../services/api/threadApi';
import { setTaskBoardForThread } from '../../store/chatRuntimeSlice';
import type { TaskBoardCard } from '../../types/turnState';

const debug = debugFactory('conversations:taskPlan');

export interface RunDecidePlanArgs {
  /** Active thread; a no-op when null (nothing to decide against). */
  threadId: string | null;
  card: TaskBoardCard;
  approve: boolean;
  /** Redux dispatch (kept loosely typed so callers don't need the store type). */
  dispatch: (action: unknown) => void;
  /** Surface a user-facing advisory on failure. */
  notify: (message: string) => void;
  /** Translator for the advisory message. */
  t: (key: string) => string;
}

/**
 * Approve or reject a card's plan via the core RPC, then optimistically refresh
 * the thread's board from the returned snapshot. A failure is logged and
 * surfaced via `notify` (never thrown) so a failed decision degrades
 * gracefully. No-op when `threadId` is null.
 */
export async function runDecidePlan({
  threadId,
  card,
  approve,
  dispatch,
  notify,
  t,
}: RunDecidePlanArgs): Promise<void> {
  if (!threadId) return;
  try {
    const saved = await threadApi.decidePlan(threadId, card.id, approve);
    if (saved) {
      dispatch(setTaskBoardForThread({ threadId, board: saved }));
    }
  } catch (error) {
    debug('decidePlan failed: %o', error);
    notify(t('conversations.taskKanban.updateFailed'));
  }
}
