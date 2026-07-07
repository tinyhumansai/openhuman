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
 */
const STORAGE_PREFIX = 'copilot-thread:';

/** Cache key for a flow: its persisted id, or `'draft'` for an unsaved draft. */
export function copilotThreadKey(flowId: string | null): string {
  return flowId ?? 'draft';
}

function storageKey(flowId: string | null): string {
  return `${STORAGE_PREFIX}${copilotThreadKey(flowId)}`;
}

export function getCopilotThreadId(flowId: string | null): string | null {
  try {
    return window.localStorage.getItem(storageKey(flowId));
  } catch {
    return null;
  }
}

export function setCopilotThreadId(flowId: string | null, threadId: string | null): void {
  try {
    if (threadId) {
      window.localStorage.setItem(storageKey(flowId), threadId);
    } else {
      window.localStorage.removeItem(storageKey(flowId));
    }
  } catch {
    // Private-mode / quota errors are non-fatal — worst case the copilot
    // simply starts a fresh thread on the next open instead of resuming.
  }
}
