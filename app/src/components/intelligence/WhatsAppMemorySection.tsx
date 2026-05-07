import { useCallback, useEffect, useState } from 'react';

import { whatsappListChats } from '../../utils/tauriCommands/memory';

interface WhatsAppMemorySectionProps {
  pollIntervalMs?: number;
}

export function WhatsAppMemorySection({ pollIntervalMs = 30000 }: WhatsAppMemorySectionProps) {
  const [chatCount, setChatCount] = useState<number | null>(null);
  const [lastSyncTs, setLastSyncTs] = useState<number | null>(null);
  const [syncing, setSyncing] = useState(false);

  const load = useCallback(async () => {
    try {
      const chats = await whatsappListChats({ limit: 200 });
      setChatCount(chats.length);
      const latest = chats.reduce((max, c) => Math.max(max, c.updated_at), 0);
      setLastSyncTs(latest > 0 ? latest : null);
    } catch {
      // Scanner may not have data yet — stay hidden.
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!pollIntervalMs) return undefined;
    const id = setInterval(() => void load(), pollIntervalMs);
    return () => clearInterval(id);
  }, [pollIntervalMs, load]);

  const handleSync = useCallback(async () => {
    setSyncing(true);
    try {
      await load();
    } finally {
      setSyncing(false);
    }
  }, [load]);

  if (chatCount === null || chatCount === 0) return null;

  return (
    <div
      className="rounded-xl border border-stone-100 bg-white px-4 py-3 shadow-sm"
      data-testid="whatsapp-memory-section">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <WhatsAppIcon />
          <span className="text-sm font-semibold text-stone-800">WhatsApp</span>
          <span className="text-xs text-stone-500">
            {chatCount.toLocaleString()} chat{chatCount !== 1 ? 's' : ''} synced
            {lastSyncTs !== null && <> · {relativeTime(lastSyncTs)}</>}
          </span>
        </div>
        <button
          type="button"
          onClick={() => void handleSync()}
          disabled={syncing}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-stone-200
                     bg-white px-3 py-1.5 text-xs font-semibold text-stone-700 shadow-sm
                     transition-colors hover:bg-stone-50
                     disabled:cursor-not-allowed disabled:opacity-50
                     focus:outline-none focus:ring-2 focus:ring-primary-200">
          <RefreshIcon spinning={syncing} />
          {syncing ? 'Syncing…' : 'Sync'}
        </button>
      </div>
    </div>
  );
}

function relativeTime(secs: number): string {
  const delta = Date.now() / 1000 - secs;
  if (delta < 60) return 'just now';
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}

function WhatsAppIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="#25D366" aria-hidden="true">
      <path d="M17.472 14.382c-.297-.149-1.758-.867-2.03-.967-.273-.099-.471-.148-.67.15-.197.297-.767.966-.94 1.164-.173.199-.347.223-.644.075-.297-.15-1.255-.463-2.39-1.475-.883-.788-1.48-1.761-1.653-2.059-.173-.297-.018-.458.13-.606.134-.133.298-.347.446-.52.149-.174.198-.298.298-.497.099-.198.05-.371-.025-.52-.075-.149-.669-1.612-.916-2.207-.242-.579-.487-.5-.669-.51-.173-.008-.371-.01-.57-.01-.198 0-.52.074-.792.372-.272.297-1.04 1.016-1.04 2.479 0 1.462 1.065 2.875 1.213 3.074.149.198 2.096 3.2 5.077 4.487.709.306 1.262.489 1.694.625.712.227 1.36.195 1.871.118.571-.085 1.758-.719 2.006-1.413.248-.694.248-1.289.173-1.413-.074-.124-.272-.198-.57-.347m-5.421 7.403h-.004a9.87 9.87 0 01-5.031-1.378l-.361-.214-3.741.982.998-3.648-.235-.374a9.86 9.86 0 01-1.51-5.26c.001-5.45 4.436-9.884 9.888-9.884 2.64 0 5.122 1.03 6.988 2.898a9.825 9.825 0 012.893 6.994c-.003 5.45-4.437 9.884-9.885 9.884m8.413-18.297A11.815 11.815 0 0012.05 0C5.495 0 .16 5.335.157 11.892c0 2.096.547 4.142 1.588 5.945L.057 24l6.305-1.654a11.882 11.882 0 005.683 1.448h.005c6.554 0 11.89-5.335 11.893-11.893a11.821 11.821 0 00-3.48-8.413z" />
    </svg>
  );
}

function RefreshIcon({ spinning }: { spinning: boolean }) {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={spinning ? 'animate-spin' : ''}
      aria-hidden="true">
      <polyline points="23 4 23 10 17 10" />
      <polyline points="1 20 1 14 7 14" />
      <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
    </svg>
  );
}
