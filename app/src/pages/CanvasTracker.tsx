import { useMemo, useState } from 'react';

import { useCanvasTracker } from '../lib/canvasTracker/hooks';
import type { CanvasTask, LocalStatus } from '../lib/canvasTracker/types';

const statuses: LocalStatus[] = [
  'not_started',
  'in_progress',
  'waiting',
  'submitted',
  'done',
  'unclear',
];

function formatDate(value?: string | null, unclear?: boolean): string {
  if (unclear || !value) return 'unclear';
  return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(
    new Date(value)
  );
}

function formatStatus(status: LocalStatus | 'all'): string {
  return status.replaceAll('_', ' ');
}

function urgencyClass(level: string): string {
  if (level === 'critical') return 'bg-red-50 text-red-700 border-red-200';
  if (level === 'high') return 'bg-orange-50 text-orange-700 border-orange-200';
  if (level === 'medium') return 'bg-amber-50 text-amber-700 border-amber-200';
  if (level === 'unclear') return 'bg-stone-100 text-stone-700 border-stone-300';
  return 'bg-emerald-50 text-emerald-700 border-emerald-200';
}

function TaskRow({
  task,
  onStatus,
}: {
  task: CanvasTask;
  onStatus: (task: CanvasTask, status: LocalStatus) => void;
}) {
  return (
    <tr className="border-b border-stone-200 align-top last:border-b-0">
      <td className="px-3 py-3 text-sm text-stone-700">{task.course_name}</td>
      <td className="px-3 py-3">
        <div className="text-sm font-semibold text-stone-900">{task.assignment_name}</div>
        <div className="mt-1 text-xs text-stone-500">
          {task.instructions_summary || 'No summary visible.'}
        </div>
      </td>
      <td className="px-3 py-3 text-sm text-stone-700">
        {formatDate(task.due_at, task.due_at_unclear)}
      </td>
      <td className="px-3 py-3 text-sm text-stone-700">{task.submission_type || 'not visible'}</td>
      <td className="px-3 py-3">
        <select
          aria-label={`Local status for ${task.assignment_name}`}
          className="rounded-sm border border-stone-300 bg-white px-2 py-1 text-sm"
          value={task.local_status}
          onChange={event => onStatus(task, event.target.value as LocalStatus)}>
          {statuses.map(status => (
            <option key={status} value={status}>
              {formatStatus(status)}
            </option>
          ))}
        </select>
      </td>
      <td className="px-3 py-3">
        <span
          className={`inline-flex rounded-sm border px-2 py-1 text-xs font-semibold ${urgencyClass(
            task.urgency_level
          )}`}>
          {task.urgency_level}
        </span>
      </td>
      <td className="px-3 py-3 text-sm text-stone-700">
        {formatDate(task.recommended_start_at, task.due_at_unclear)}
      </td>
      <td className="px-3 py-3 text-xs text-stone-600">
        {task.reminders_needed.length === 0
          ? 'none'
          : task.reminders_needed.map(reminder => reminder.message).join(' ')}
      </td>
    </tr>
  );
}

export default function CanvasTracker() {
  const { settings, tasks, loading, syncing, lastSync, error, syncNow, updateStatus } =
    useCanvasTracker();
  const [filter, setFilter] = useState<LocalStatus | 'all'>('all');

  const visibleTasks = useMemo(
    () => (filter === 'all' ? tasks : tasks.filter(task => task.local_status === filter)),
    [filter, tasks]
  );

  return (
    <main className="h-full overflow-auto bg-stone-50 px-6 py-6 text-stone-900">
      <div className="mx-auto max-w-7xl pb-24">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold">Canvas Tracker</h1>
            <p className="mt-1 text-sm text-stone-600">
              Read-only assignment tracking for approved Canvas courses.
            </p>
          </div>
          <button
            type="button"
            onClick={() => void syncNow()}
            disabled={syncing}
            className="rounded-sm bg-stone-900 px-4 py-2 text-sm font-semibold text-white disabled:opacity-50">
            {syncing ? 'Syncing...' : 'Sync now'}
          </button>
        </div>

        {error && (
          <div className="mt-4 rounded-sm border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700">
            {error}
          </div>
        )}

        <section className="mt-6 grid gap-4 md:grid-cols-3">
          <div className="rounded-sm border border-stone-200 bg-white p-4">
            <div className="text-xs font-semibold uppercase text-stone-500">Connection</div>
            <div className="mt-2 text-sm text-stone-800">
              {settings?.token_set ? 'Token saved locally' : 'Token not configured'}
            </div>
            <div className="mt-1 text-xs text-stone-500">
              {settings?.host ?? 'https://mango-cmu.instructure.com'}
            </div>
          </div>
          <div className="rounded-sm border border-stone-200 bg-white p-4 md:col-span-2">
            <div className="text-xs font-semibold uppercase text-stone-500">
              Allowlisted courses
            </div>
            <ul className="mt-2 space-y-1 text-sm text-stone-800">
              {(settings?.allowlisted_courses ?? []).map(course => (
                <li key={course.name}>{course.name}</li>
              ))}
            </ul>
          </div>
        </section>

        <section className="mt-6 rounded-sm border border-stone-200 bg-white">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-stone-200 px-4 py-3">
            <div>
              <h2 className="text-sm font-semibold">Tasks</h2>
              <p className="text-xs text-stone-500">
                {lastSync ? `Last sync ${formatDate(lastSync.synced_at)}` : 'Manual sync only'}
              </p>
              <p className="mt-1 text-xs text-stone-500">
                Local status changes update OpenHuman only; they never submit to Canvas.
              </p>
            </div>
            <select
              aria-label="Filter tasks"
              className="rounded-sm border border-stone-300 bg-white px-2 py-1 text-sm"
              value={filter}
              onChange={event => setFilter(event.target.value as LocalStatus | 'all')}>
              <option value="all">all</option>
              {statuses.map(status => (
                <option key={status} value={status}>
                  {formatStatus(status)}
                </option>
              ))}
            </select>
          </div>

          {loading ? (
            <div className="px-4 py-8 text-sm text-stone-500">Loading Canvas tracker...</div>
          ) : visibleTasks.length === 0 ? (
            <div className="px-4 py-8 text-sm text-stone-500">
              No tasks found for the selected filter.
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="min-w-full table-fixed">
                <thead className="bg-stone-100 text-left text-xs font-semibold uppercase text-stone-500">
                  <tr>
                    <th className="w-56 px-3 py-2">Course</th>
                    <th className="w-80 px-3 py-2">Assignment</th>
                    <th className="w-40 px-3 py-2">Due</th>
                    <th className="w-36 px-3 py-2">Submission</th>
                    <th className="w-40 px-3 py-2">Local status</th>
                    <th className="w-28 px-3 py-2">Urgency</th>
                    <th className="w-40 px-3 py-2">Start</th>
                    <th className="w-64 px-3 py-2">Reminders</th>
                  </tr>
                </thead>
                <tbody>
                  {visibleTasks.map(task => (
                    <TaskRow
                      key={`${task.course_id}:${task.assignment_id}`}
                      task={task}
                      onStatus={updateStatus}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      </div>
    </main>
  );
}
