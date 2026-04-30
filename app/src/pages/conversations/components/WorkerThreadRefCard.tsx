import { useDispatch } from 'react-redux';

import { setActiveThread } from '../../../store/threadSlice';
import type { WorkerThreadRef } from '../utils/workerThreadRef';

/**
 * Compact card rendered inside a parent thread's tool timeline when the
 * orchestrator delegated a sub-task into a dedicated worker thread.
 * Clicking the card swaps the active thread so the user can read the
 * sub-agent's full transcript without losing the parent conversation.
 */
export function WorkerThreadRefCard({ ref }: { ref: WorkerThreadRef }) {
  const dispatch = useDispatch();
  const meta: string[] = [];
  if (ref.agentId) meta.push(ref.agentId);
  if (typeof ref.iterations === 'number') {
    meta.push(`${ref.iterations} ${ref.iterations === 1 ? 'turn' : 'turns'}`);
  }
  if (typeof ref.elapsedMs === 'number') {
    meta.push(`${Math.round(ref.elapsedMs)}ms`);
  }

  return (
    <button
      type="button"
      onClick={() => dispatch(setActiveThread(ref.threadId))}
      className="mt-1 flex w-full items-center justify-between gap-3 rounded-xl border border-primary-200 bg-primary-50 px-3 py-2 text-left transition-colors hover:bg-primary-100">
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="rounded-full bg-primary-200 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary-800">
            {ref.label}
          </span>
          <span className="truncate text-xs font-medium text-primary-900">Open worker thread</span>
        </div>
        {meta.length > 0 ? (
          <div className="mt-0.5 text-[10px] text-primary-700/80">{meta.join(' · ')}</div>
        ) : null}
      </div>
      <svg
        className="h-3 w-3 shrink-0 text-primary-700"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M5 12h14M13 6l6 6-6 6"
        />
      </svg>
    </button>
  );
}
