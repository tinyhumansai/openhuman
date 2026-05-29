import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { getCoreHttpBaseUrl, getCoreRpcToken } from '../../../services/coreRpcClient';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface EventEntry {
  id: number;
  domain: string;
  event: string;
  agent: string;
  timestamp: string;
}

const DOMAIN_BADGE_KEYS: Record<string, string> = {
  tool: 'settings.developerMenu.eventLog.badge.tool',
  agent: 'settings.developerMenu.eventLog.badge.agent',
  system: 'settings.developerMenu.eventLog.badge.info',
  memory: 'settings.developerMenu.eventLog.badge.mem',
  channel: 'settings.developerMenu.eventLog.badge.chan',
  cron: 'settings.developerMenu.eventLog.badge.cron',
  webhook: 'settings.developerMenu.eventLog.badge.hook',
  approval: 'settings.developerMenu.eventLog.badge.warn',
  skill: 'settings.developerMenu.eventLog.badge.skill',
  composio: 'settings.developerMenu.eventLog.badge.comp',
  mcp_client: 'settings.developerMenu.eventLog.badge.mcp',
};

const DOMAIN_BADGE_COLORS: Record<string, { bg: string; text: string }> = {
  tool: { bg: 'bg-blue-500/20', text: 'text-blue-400' },
  agent: { bg: 'bg-green-500/20', text: 'text-green-400' },
  system: { bg: 'bg-slate-500/20', text: 'text-slate-400' },
  memory: { bg: 'bg-purple-500/20', text: 'text-purple-400' },
  channel: { bg: 'bg-cyan-500/20', text: 'text-cyan-400' },
  cron: { bg: 'bg-orange-500/20', text: 'text-orange-400' },
  webhook: { bg: 'bg-indigo-500/20', text: 'text-indigo-400' },
  approval: { bg: 'bg-amber-500/20', text: 'text-amber-400' },
  skill: { bg: 'bg-teal-500/20', text: 'text-teal-400' },
  composio: { bg: 'bg-pink-500/20', text: 'text-pink-400' },
  mcp_client: { bg: 'bg-violet-500/20', text: 'text-violet-400' },
};

const MAX_ENTRIES = 200;
const RECONNECT_DELAY_MS = 3000;

const EventLogPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [entries, setEntries] = useState<EventEntry[]>([]);
  const [isLive, setIsLive] = useState(false);
  const [filterType, setFilterType] = useState<string>('');
  const [filterText, setFilterText] = useState('');
  const [autoScroll, setAutoScroll] = useState(true);
  const containerRef = useRef<HTMLDivElement>(null);
  const idRef = useRef(0);
  const controllerRef = useRef<AbortController | null>(null);
  const reconnectRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const unmountedRef = useRef(false);
  const maxEntriesRef = useRef(MAX_ENTRIES);
  const newEntriesRef = useRef<'top' | 'bottom'>('top');

  const connectRef = useRef<(() => Promise<void>) | null>(null);

  const connect = async () => {
    if (unmountedRef.current) return;
    try {
      const [baseUrl, token] = await Promise.all([getCoreHttpBaseUrl(), getCoreRpcToken()]);
      if (!token) {
        setIsLive(false);
        return;
      }

      const url = `${baseUrl}/events/domain`;
      const controller = new AbortController();
      controllerRef.current = controller;

      const response = await fetch(url, {
        headers: { Authorization: `Bearer ${token}` },
        signal: controller.signal,
      });

      if (!response.ok || !response.body) {
        setIsLive(false);
        return;
      }

      setIsLive(true);
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          if (line.startsWith('event:')) {
            const eventType = line.slice(6).trim();
            if (eventType === 'config') {
              // Next data: line is config — handled below
              continue;
            }
          }
          if (line.startsWith('data:')) {
            const jsonStr = line.slice(5).trim();
            if (!jsonStr) continue;
            try {
              const data = JSON.parse(jsonStr);
              // Config message from server
              if (data.max_entries !== undefined) {
                maxEntriesRef.current = data.max_entries;
                if (data.new_entries === 'top' || data.new_entries === 'bottom') {
                  newEntriesRef.current = data.new_entries;
                }
                continue;
              }
              const entry: EventEntry = {
                id: ++idRef.current,
                domain: data.domain || 'unknown',
                event: data.event || '',
                agent: data.agent || '',
                timestamp: data.timestamp || '',
              };
              setEntries(prev => {
                const next = newEntriesRef.current === 'top' ? [entry, ...prev] : [...prev, entry];
                return next.length > maxEntriesRef.current
                  ? newEntriesRef.current === 'top'
                    ? next.slice(0, maxEntriesRef.current)
                    : next.slice(-maxEntriesRef.current)
                  : next;
              });
            } catch {
              // skip malformed
            }
          }
        }
      }
      setIsLive(false);
    } catch {
      setIsLive(false);
    } finally {
      controllerRef.current = null;
      // Auto-reconnect unless unmounted
      if (!unmountedRef.current) {
        reconnectRef.current = setTimeout(() => void connectRef.current?.(), RECONNECT_DELAY_MS);
      }
    }
  };

  connectRef.current = connect;

  useEffect(() => {
    unmountedRef.current = false;
    void connectRef.current?.();
    return () => {
      unmountedRef.current = true;
      controllerRef.current?.abort();
      controllerRef.current = null;
      if (reconnectRef.current) {
        clearTimeout(reconnectRef.current);
        reconnectRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (autoScroll && containerRef.current) {
      const el = containerRef.current;
      el.scrollTop = newEntriesRef.current === 'top' ? 0 : el.scrollHeight;
    }
  }, [entries, autoScroll]);

  const handleScroll = () => {
    const el = containerRef.current;
    if (!el) return;
    const atAnchor =
      newEntriesRef.current === 'top'
        ? el.scrollTop < 10
        : el.scrollHeight - el.scrollTop - el.clientHeight < 10;
    setAutoScroll(atAnchor);
  };

  const filteredEntries = entries.filter(e => {
    if (filterType && e.domain !== filterType) return false;
    if (filterText) {
      const q = filterText.toLowerCase();
      if (!e.event.toLowerCase().includes(q) && !e.agent.toLowerCase().includes(q)) return false;
    }
    return true;
  });

  const exportLog = () => {
    const blob = new Blob([filteredEntries.map(e => JSON.stringify(e)).join('\n')], {
      type: 'application/x-ndjson',
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `event-log-${new Date().toISOString().slice(0, 19).replace(/:/g, '-')}.ndjson`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const domains = [...new Set(entries.map(e => e.domain))].sort();

  return (
    <div data-testid="event-log-panel">
      <SettingsHeader
        title={t('settings.developerMenu.eventLog.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* Status bar */}
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <select
            className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-1.5 font-medium text-stone-700 dark:text-neutral-200"
            value={filterType}
            onChange={e => setFilterType(e.target.value)}>
            <option value="">{t('settings.developerMenu.eventLog.allTypes')}</option>
            {domains.map(d => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
          <input
            type="text"
            className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-1.5 font-medium text-stone-700 dark:text-neutral-200 w-40"
            placeholder={t('settings.developerMenu.eventLog.filterAgent')}
            value={filterText}
            onChange={e => setFilterText(e.target.value)}
          />
          <button
            type="button"
            onClick={exportLog}
            disabled={filteredEntries.length === 0}
            className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-1.5 font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800 disabled:opacity-50">
            {t('settings.developerMenu.eventLog.download')}
          </button>
          <span className="text-stone-500 dark:text-neutral-400">
            {filteredEntries.length} {t('settings.developerMenu.eventLog.events')} &middot;{' '}
            <span
              className={
                isLive ? 'text-sage-600 dark:text-sage-300' : 'text-stone-400 dark:text-neutral-500'
              }>
              {isLive
                ? t('settings.developerMenu.eventLog.live')
                : t('settings.developerMenu.eventLog.disconnected')}
            </span>
          </span>
        </div>

        {/* Jump to latest */}
        {!autoScroll && (
          <button
            type="button"
            onClick={() => {
              setAutoScroll(true);
              const el = containerRef.current;
              if (el) {
                el.scrollTop = newEntriesRef.current === 'top' ? 0 : el.scrollHeight;
              }
            }}
            className="text-xs rounded-lg border border-primary-300 dark:border-primary-500/40 bg-primary-50 dark:bg-primary-500/10 px-3 py-1 text-primary-700 dark:text-primary-300">
            {t('settings.developerMenu.eventLog.jumpToLatest')}
          </button>
        )}

        {/* Event stream */}
        <section className="space-y-1">
          <div
            ref={containerRef}
            onScroll={handleScroll}
            className="max-h-[60vh] overflow-y-auto space-y-1">
            {filteredEntries.length === 0 && (
              <p className="text-xs text-stone-400 dark:text-neutral-500 py-4 text-center">
                {isLive
                  ? t('settings.developerMenu.eventLog.waiting')
                  : t('settings.developerMenu.eventLog.notConnected')}
              </p>
            )}
            {filteredEntries.map(entry => {
              const colors = DOMAIN_BADGE_COLORS[entry.domain] || {
                bg: 'bg-stone-500/20',
                text: 'text-stone-400',
              };
              return (
                <div
                  key={entry.id}
                  className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2 flex items-start gap-2">
                  <span className="text-[10px] text-stone-400 dark:text-neutral-500 font-mono shrink-0 pt-0.5">
                    {entry.timestamp}
                  </span>
                  <span
                    className={`rounded-full ${colors.bg} px-2 py-0.5 text-[10px] ${colors.text} shrink-0`}>
                    {DOMAIN_BADGE_KEYS[entry.domain]
                      ? t(DOMAIN_BADGE_KEYS[entry.domain])
                      : entry.domain.toUpperCase()}
                  </span>
                  {entry.agent && (
                    <span className="text-[10px] text-stone-500 dark:text-neutral-400 shrink-0 font-mono">
                      {entry.agent}
                    </span>
                  )}
                  <span className="text-xs text-stone-900 dark:text-neutral-100 truncate">
                    {entry.event}
                  </span>
                </div>
              );
            })}
          </div>
        </section>
      </div>
    </div>
  );
};

export default EventLogPanel;
