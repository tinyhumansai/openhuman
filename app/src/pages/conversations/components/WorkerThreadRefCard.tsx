import { useDispatch } from 'react-redux';

import { useT } from '../../../lib/i18n/I18nContext';
import { setActiveThread } from '../../../store/threadSlice';
import type { WorkerThreadRef } from '../utils/workerThreadRef';

/**
 * Live lifecycle phase of the worker thread referenced by the card.
 *
 * The values mirror the parent timeline entry's status (which is what
 * `ToolTimelineBlock` derives from the `subagent_spawned` /
 * `subagent_completed` / `subagent_failed` socket events). Using the
 * same source of truth means the badge can never disagree with the
 * surrounding `<details>` row's status pill — both refresh together
 * when the lifecycle event lands.
 */
export type WorkerThreadStatus = 'running' | 'completed' | 'failed';

interface WorkerThreadStatusBadgeProps {
  status: WorkerThreadStatus;
}

/**
 * Compact running/completed/failed badge rendered next to the worker
 * label inside the card. Issue #1624 acceptance criterion "Worker
 * lifecycle is visible" — the badge is the at-a-glance signal so users
 * scanning the parent transcript know whether the background work has
 * finished or is still in flight without having to open the worker.
 *
 * Tones use the existing tool-timeline status palette
 * (amber=running, sage=success, coral=error) so a worker badge inside a
 * timeline row reads as the same state as its containing `<details>`
 * status pill — no new colour vocabulary for the user to learn.
 */
function WorkerThreadStatusBadge({ status }: WorkerThreadStatusBadgeProps) {
  const tone =
    status === 'running'
      ? 'bg-amber-100 text-amber-700'
      : status === 'completed'
        ? 'bg-sage-100 text-sage-700'
        : 'bg-coral-100 text-coral-700';
  const label = status === 'running' ? 'running' : status === 'completed' ? 'done' : 'failed';
  return (
    <span
      className={`flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide ${tone}`}
      data-testid="worker-thread-status-badge"
      data-status={status}
      role="status"
      aria-label={`Worker ${label}`}>
      {status === 'running' ? (
        // Inline animated dot — purely decorative; the visible label
        // carries the meaning so screen readers don't need to parse it.
        <span aria-hidden="true" className="h-1.5 w-1.5 animate-pulse rounded-full bg-amber-500" />
      ) : null}
      {label}
    </span>
  );
}

/**
 * Compact card rendered inside a parent thread's tool timeline when the
 * orchestrator delegated a sub-task into a dedicated worker thread.
 * Clicking the card swaps the active thread so the user can read the
 * sub-agent's full transcript without losing the parent conversation.
 *
 * `status` is optional so the card stays renderable from ad-hoc parsers
 * that don't have a parent timeline context (today only the historical
 * test fixtures hit that path). When present, the badge surfaces live
 * running / completed / failed state derived from the parent timeline
 * entry's status — issue #1624.
 */
export function WorkerThreadRefCard({
  ref,
  status,
}: {
  ref: WorkerThreadRef;
  status?: WorkerThreadStatus;
}) {
  const { t } = useT();
  const dispatch = useDispatch();
  const meta: string[] = [];
  if (ref.agentId) meta.push(ref.agentId);
  if (typeof ref.iterations === 'number') {
    meta.push(`${ref.iterations} ${ref.iterations === 1 ? t('chat.turn') : t('chat.turns')}`);
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
          <span className="truncate text-xs font-medium text-primary-900">
            {t('chat.openWorkerThread')}
          </span>
          {status ? <WorkerThreadStatusBadge status={status} /> : null}
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
