import type { WebhookActivityEntry } from '../../features/webhooks/types';

interface WebhookActivityProps {
  activity: WebhookActivityEntry[];
}

const METHOD_COLORS: Record<string, string> = {
  GET: 'text-primary-600 bg-primary-50',
  POST: 'text-sage-700 bg-sage-50',
  PUT: 'text-amber-700 bg-amber-50',
  PATCH: 'text-amber-700 bg-amber-50',
  DELETE: 'text-coral-700 bg-coral-50',
};

function statusColor(code: number | null): string {
  if (code === null) return 'text-stone-400';
  if (code >= 200 && code < 300) return 'text-sage-600';
  if (code >= 400 && code < 500) return 'text-amber-600';
  if (code >= 500) return 'text-coral-600';
  return 'text-stone-600';
}

function formatTime(ts: number): string {
  return new Date(ts).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

export default function WebhookActivity({ activity }: WebhookActivityProps) {
  if (activity.length === 0) {
    return (
      <div className="space-y-3">
        <h3 className="text-lg font-semibold text-stone-900">Recent Activity</h3>
        <p className="text-sm text-stone-500 text-center py-6">
          No webhook activity yet. Events will appear here when webhooks are received.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h3 className="text-lg font-semibold text-stone-900">
        Recent Activity{' '}
        <span className="text-sm font-normal text-stone-400">({activity.length})</span>
      </h3>
      <div className="space-y-1">
        {activity.map(entry => (
          <div
            key={entry.correlation_id}
            className="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-stone-50 transition-colors text-sm">
            <span className="text-xs text-stone-400 font-mono w-20 shrink-0">
              {formatTime(entry.timestamp)}
            </span>
            <span
              className={`text-xs font-mono font-medium px-1.5 py-0.5 rounded w-14 text-center shrink-0 ${
                METHOD_COLORS[entry.method] || 'text-stone-600 bg-stone-50'
              }`}>
              {entry.method}
            </span>
            <span className="text-stone-700 truncate flex-1 font-mono text-xs">
              {entry.path || '/'}
            </span>
            <span
              className={`text-xs font-mono w-8 text-right shrink-0 ${statusColor(entry.status_code)}`}>
              {entry.status_code ?? '---'}
            </span>
            {entry.skill_id && (
              <span className="text-xs text-primary-600 bg-primary-50 px-1.5 py-0.5 rounded shrink-0">
                {entry.skill_id}
              </span>
            )}
            <span className="text-xs text-stone-400 truncate max-w-[120px]">
              {entry.tunnel_name}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
