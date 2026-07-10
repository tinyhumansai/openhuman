import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { getCoreHttpBaseUrl, getCoreRpcToken } from '../../../services/coreRpcClient';
import Button from '../../ui/Button';
import { SettingsSelect, SettingsTextField } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

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
    <SettingsPanel testId="event-log-panel" description={t('settings.developerMenu.eventLog.desc')}>
      {/* Status bar */}
      <div className="flex flex-wrap items-center gap-2">
        <SettingsSelect
          value={filterType}
          onChange={e => setFilterType(e.target.value)}
          aria-label={t('settings.developerMenu.eventLog.allTypes')}
          inputSize="sm">
          <option value="">{t('settings.developerMenu.eventLog.allTypes')}</option>
          {domains.map(d => (
            <option key={d} value={d}>
              {d}
            </option>
          ))}
        </SettingsSelect>
        <SettingsTextField
          className="w-40"
          placeholder={t('settings.developerMenu.eventLog.filterAgent')}
          value={filterText}
          onChange={e => setFilterText(e.target.value)}
          aria-label={t('settings.developerMenu.eventLog.filterAgent')}
          inputSize="sm"
        />
        <Button
          type="button"
          variant="secondary"
          size="xs"
          onClick={exportLog}
          disabled={filteredEntries.length === 0}>
          {t('settings.developerMenu.eventLog.download')}
        </Button>
        <span className="text-xs text-content-muted">
          {filteredEntries.length} {t('settings.developerMenu.eventLog.events')} &middot;{' '}
          <span className={isLive ? 'text-sage-600 dark:text-sage-300' : 'text-content-muted'}>
            {isLive
              ? t('settings.developerMenu.eventLog.live')
              : t('settings.developerMenu.eventLog.disconnected')}
          </span>
        </span>
      </div>

      {/* Jump to latest */}
      {!autoScroll && (
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          onClick={() => {
            setAutoScroll(true);
            const el = containerRef.current;
            if (el) {
              el.scrollTop = newEntriesRef.current === 'top' ? 0 : el.scrollHeight;
            }
          }}>
          {t('settings.developerMenu.eventLog.jumpToLatest')}
        </Button>
      )}

      {/* Event stream */}
      <section className="space-y-1">
        <div
          ref={containerRef}
          onScroll={handleScroll}
          className="max-h-[60vh] overflow-y-auto space-y-1">
          {filteredEntries.length === 0 && (
            <p className="text-xs text-content-muted py-4 text-center">
              {isLive
                ? t('settings.developerMenu.eventLog.waiting')
                : t('settings.developerMenu.eventLog.notConnected')}
            </p>
          )}
          {filteredEntries.map(entry => {
            const colors = DOMAIN_BADGE_COLORS[entry.domain] || {
              bg: 'bg-neutral-500/20',
              text: 'text-content-faint',
            };
            return (
              <div
                key={entry.id}
                className="rounded-xl border border-line bg-surface-muted px-3 py-2 flex items-start gap-2">
                <span className="text-[10px] text-content-muted font-mono shrink-0 pt-0.5">
                  {entry.timestamp}
                </span>
                <span
                  className={`rounded-full ${colors.bg} px-2 py-0.5 text-[10px] ${colors.text} shrink-0`}>
                  {DOMAIN_BADGE_KEYS[entry.domain]
                    ? t(DOMAIN_BADGE_KEYS[entry.domain])
                    : entry.domain.toUpperCase()}
                </span>
                {entry.agent && (
                  <span className="text-[10px] text-content-muted shrink-0 font-mono">
                    {entry.agent}
                  </span>
                )}
                <span className="text-xs text-content truncate">{entry.event}</span>
              </div>
            );
          })}
        </div>
      </section>
    </SettingsPanel>
  );
};

export default EventLogPanel;
