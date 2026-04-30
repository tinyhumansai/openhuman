import type { RespondQueueItem } from '../../types/providerSurfaces';
import { openUrl } from '../../utils/openUrl';

interface RespondQueuePanelProps {
  items: RespondQueueItem[];
  count: number;
  status: 'idle' | 'loading' | 'succeeded' | 'failed';
  error: string | null;
  onRefresh: () => void;
}

function relativeTime(iso: string): string {
  const ts = new Date(iso).getTime();
  if (!Number.isFinite(ts)) return 'unknown time';
  const deltaMs = Date.now() - ts;
  if (deltaMs < 60_000) return 'just now';
  const mins = Math.floor(deltaMs / 60_000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function queueTitle(item: RespondQueueItem): string {
  return item.title || item.senderName || item.eventKind || item.provider;
}

export default function RespondQueuePanel({
  items,
  count,
  status,
  error,
  onRefresh,
}: RespondQueuePanelProps) {
  return (
    <aside className="flex w-80 flex-none flex-col border-l border-stone-200 bg-white">
      <div className="flex flex-none items-center justify-between border-b border-stone-100 px-4 py-3">
        <div>
          <h3 className="text-sm font-semibold text-stone-800">Respond queue</h3>
          <p className="text-xs text-stone-500">{count} pending</p>
        </div>
        <button
          type="button"
          onClick={onRefresh}
          className="rounded-lg border border-stone-200 px-2 py-1 text-xs text-stone-600 hover:bg-stone-50">
          Refresh
        </button>
      </div>
      <div className="flex-1 overflow-y-auto px-3 py-3">
        {status === 'loading' && items.length === 0 ? (
          <p className="rounded-lg bg-stone-50 px-3 py-2 text-xs text-stone-500">Loading queue…</p>
        ) : null}

        {status === 'failed' ? (
          <p className="rounded-lg bg-coral-50 px-3 py-2 text-xs text-coral-600">
            {error ?? 'Failed to load respond queue'}
          </p>
        ) : null}

        {items.length === 0 && status !== 'loading' ? (
          <p className="rounded-lg bg-stone-50 px-3 py-2 text-xs text-stone-500">
            No pending provider events yet.
          </p>
        ) : null}

        <div className="space-y-2">
          {items.slice(0, 30).map(item => (
            <button
              key={item.id}
              type="button"
              onClick={() => {
                if (item.deepLink) {
                  void openUrl(item.deepLink);
                }
              }}
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 text-left transition-colors hover:bg-stone-50 disabled:cursor-default"
              disabled={!item.deepLink}>
              <div className="flex items-center justify-between gap-2">
                <p className="truncate text-xs font-medium text-stone-800">{queueTitle(item)}</p>
                <span className="rounded-full bg-stone-100 px-2 py-0.5 text-[10px] uppercase text-stone-600">
                  {item.provider}
                </span>
              </div>
              {item.snippet ? (
                <p className="mt-1 line-clamp-2 text-xs text-stone-600">{item.snippet}</p>
              ) : null}
              <div className="mt-1 flex items-center justify-between text-[10px] text-stone-500">
                <span>{item.senderName ?? item.senderHandle ?? item.accountId}</span>
                <span>{relativeTime(item.timestamp)}</span>
              </div>
            </button>
          ))}
        </div>
      </div>
    </aside>
  );
}
