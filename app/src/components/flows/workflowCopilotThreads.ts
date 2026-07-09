/**
 * Persisted cache of each workflow's copilot chat thread id, keyed by flow
 * (a persisted flow id, or `'draft'` for an unsaved draft). The copilot panel
 * unmounts when closed and `FlowEditor` remounts when switching flows, so
 * without this the `workflow_builder` thread — and its transcript — would be
 * lost on every open/close. Persisting the thread id lets the panel reseed the
 * same thread (its messages live in the core, rehydrated into the Redux
 * `messagesByThreadId` store by `useWorkflowBuilderChat`'s mount effect), so
 * reopening the copilot restores the conversation for that workflow.
 *
 * Backed by `localStorage` (not a module-level `Map`) so the mapping survives
 * a full app reload — the durable half of "Copilot chat not persistent": the
 * transcript itself is already durable server-side via `threadApi`, this file
 * is what makes the panel know WHICH thread to reload. `localStorage` access
 * is wrapped in try/catch — private-mode / quota errors degrade to a no-op
 * (the copilot just starts a fresh thread on the next open) rather than
 * throwing.
 *
 * Keys are namespaced by the active user id (`${userId}:copilot-thread:<flow>`),
 * the same `${userId}:` convention `userScopedStorage`/`clearAllAppData` use for
 * every other per-user localStorage blob (#900, #983). Without this an
 * identity flip (or a "clear my data" on account B that only purges B's
 * `${userId}:*` keys) would leave account A's thread id readable by whoever
 * opens the same flow/draft next, pointing them at A's builder thread instead
 * of starting fresh. `getActiveUserId()` is synchronous (primed at boot by
 * `main.tsx`), so this stays a plain sync read/write unlike the async
 * redux-persist storage contract in `userScopedStorage.ts`.
 */
import createDebug from 'debug';

import { getActiveUserId } from '../../store/userScopedStorage';

const log = createDebug('app:flows:copilot-threads');

const STORAGE_PREFIX = 'copilot-thread:';

/** Cache key for a flow: its persisted id, or `'draft'` for an unsaved draft. */
export function copilotThreadKey(flowId: string | null): string {
  return flowId ?? 'draft';
}

function storageKey(flowId: string | null): string {
  const userId = getActiveUserId();
  const scope = userId ? `${userId}:` : '';
  return `${scope}${STORAGE_PREFIX}${copilotThreadKey(flowId)}`;
}

export function getCopilotThreadId(flowId: string | null): string | null {
  const flowKey = copilotThreadKey(flowId);
  try {
    const threadId = window.localStorage.getItem(storageKey(flowId));
    log('get flow=%s -> %s', flowKey, threadId ?? '(none)');
    return threadId;
  } catch (err) {
    log('get flow=%s failed: %o', flowKey, err);
    return null;
  }
}

export function setCopilotThreadId(flowId: string | null, threadId: string | null): void {
  const flowKey = copilotThreadKey(flowId);
  try {
    if (threadId) {
      window.localStorage.setItem(storageKey(flowId), threadId);
      log('set flow=%s -> thread=%s', flowKey, threadId);
    } else {
      window.localStorage.removeItem(storageKey(flowId));
      log('clear flow=%s', flowKey);
    }
  } catch (err) {
    // Private-mode / quota errors are non-fatal — worst case the copilot
    // simply starts a fresh thread on the next open instead of resuming.
    log('set flow=%s failed: %o', flowKey, err);
  }
}
