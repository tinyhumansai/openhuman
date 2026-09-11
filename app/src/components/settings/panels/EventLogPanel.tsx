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
  /**
   * One already-redacted line the backend attaches to the variants whose
   * point is a failure reason (`DomainEvent::log_detail`) — an MCP transport
   * that broke and one that timed out are otherwise the same row. Empty for
   * every other event, which renders exactly as it did before (#5931).
   */
  detail: string;
  timestamp: string;
  /**
   * Opaque handle for the workspace this event belongs to, or `null` when the
   * event is not workspace-bound (#5966).
   *
   * A handle, never a path: the core hashes `workspace_dir` before it reaches
   * this envelope, because the log renders in a settings panel and downloads
   * as NDJSON, and the path is under the user's home directory.
   */
  workspace: string | null;
}

/**
 * Which rows the log shows (#5966).
 *
 * One core process serves more than one workspace over its life — a switch
 * leaves the previous one open — so this single stream mixes them, and until
 * now a row from a workspace the reader had left was indistinguishable from
 * one belonging to the workspace they were in. `active` is the default
 * because someone watching a live log is almost always asking about the
 * workspace they are in; `all` keeps the process-wide view for debugging.
 */
type WorkspaceScope = 'active' | 'all';

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

/**
 * Domain tone table. Eleven domains, four themeable ramps — so the hue is spent
 * on the three readings a reader scans for in a live log (who acted: the agent
 * or a tool; and which rows are waiting on a human) and every other domain
 * takes the neutral pair `system` already used. Coral is deliberately left
 * unassigned: nothing here means "failure", and painting an ordinary domain in
 * the danger ramp would make routine events read as errors. The badge prints
 * the domain name either way. See `gitbooks/developing/theming.md`.
 */
const DOMAIN_NEUTRAL_TONE = { bg: 'bg-content-muted/20', text: 'text-content-secondary' } as const;

const DOMAIN_BADGE_COLORS: Record<string, { bg: string; text: string }> = {
  tool: { bg: 'bg-primary-500/20', text: 'text-primary-400' },
  agent: { bg: 'bg-sage-500/20', text: 'text-sage-400' },
  system: DOMAIN_NEUTRAL_TONE,
  memory: DOMAIN_NEUTRAL_TONE,
  channel: DOMAIN_NEUTRAL_TONE,
  cron: DOMAIN_NEUTRAL_TONE,
  webhook: DOMAIN_NEUTRAL_TONE,
  approval: { bg: 'bg-amber-500/20', text: 'text-amber-400' },
  skill: DOMAIN_NEUTRAL_TONE,
  composio: DOMAIN_NEUTRAL_TONE,
  mcp_client: DOMAIN_NEUTRAL_TONE,
};

const MAX_ENTRIES = 200;
const RECONNECT_DELAY_MS = 3000;

const EventLogPanel = () => {
  const { t } = useT();
  const [entries, setEntries] = useState<EventEntry[]>([]);
  const [isLive, setIsLive] = useState(false);
  const [filterType, setFilterType] = useState<string>('');
  const [filterText, setFilterText] = useState('');
  const [scope, setScope] = useState<WorkspaceScope>('active');
  /**
   * Handle of the workspace the core is serving right now, or `null` while
   * that is unknown — the core could not resolve it, or has not resolved it
   * since a workspace marker was rewritten.
   *
   * Tracked as state rather than a ref because the row filter reads it: a
   * switch has to re-render the list so the previous workspace's rows fall
   * out of the default view. It is only ever *set* when the value actually
   * changes, so an idle stream does not re-render on every event.
   */
  const [activeWorkspace, setActiveWorkspace] = useState<string | null>(null);
  const activeWorkspaceRef = useRef<string | null>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const containerRef = useRef<HTMLDivElement>(null);
  const idRef = useRef(0);
  const controllerRef = useRef<AbortController | null>(null);
  const reconnectRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const unmountedRef = useRef(false);
  const maxEntriesRef = useRef(MAX_ENTRIES);
  const newEntriesRef = useRef<'top' | 'bottom'>('top');

  const connectRef = useRef<(() => Promise<void>) | null>(null);

  /**
   * Record which workspace the core says is current, if it said anything.
   *
   * The ref guard matters: this runs for every streamed row, and calling
   * `setActiveWorkspace` unconditionally would re-render the whole list on
   * each one. A missing or non-string value is ignored rather than treated as
   * `null` — the core omits the field when it could not resolve the
   * workspace, and forgetting a handle we already know would silently widen
   * the default view back to every workspace.
   */
  const rememberActiveWorkspace = (value: unknown) => {
    if (typeof value !== 'string' || !value) return;
    if (activeWorkspaceRef.current === value) return;
    activeWorkspaceRef.current = value;
    setActiveWorkspace(value);
  };

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
                // The connect-time answer, so a client that joins between
                // switches can scope the log immediately instead of waiting
                // for an event to tell it which workspace is current.
                rememberActiveWorkspace(data.active_workspace);
                continue;
              }
              // Every row also carries the workspace that was active when it
              // was emitted. That is what makes a *switch* visible on a
              // connection that stays open: the next row after one says a
              // different workspace is current, and the previous workspace's
              // rows drop out of the default view.
              rememberActiveWorkspace(data.active_workspace);
              const entry: EventEntry = {
                id: ++idRef.current,
                domain: data.domain || 'unknown',
                event: data.event || '',
                agent: data.agent || '',
                detail: data.detail || '',
                timestamp: data.timestamp || '',
                workspace: typeof data.workspace === 'string' ? data.workspace : null,
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
    // Workspace scope first — it is the one filter that changes what the log
    // *means* rather than narrowing what it shows, and it also scopes the
    // NDJSON download, which exports exactly these rows.
    //
    // Two rows always survive it: one with no workspace of its own (most
    // events are process-wide and belong wherever they land), and every row
    // while `activeWorkspace` is still unknown — with nothing to compare
    // against, hiding rows would empty the panel and give the reader no way
    // to tell that from a quiet process.
    if (scope === 'active' && activeWorkspace && e.workspace && e.workspace !== activeWorkspace) {
      return false;
    }
    if (filterType && e.domain !== filterType) return false;
    if (filterText) {
      const q = filterText.toLowerCase();
      if (
        !e.event.toLowerCase().includes(q) &&
        !e.agent.toLowerCase().includes(q) &&
        !e.detail.toLowerCase().includes(q)
      )
        return false;
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
    <SettingsPanel
      testId="event-log-panel"
      scrollable={false}
      bodyClassName="flex h-full min-h-0 flex-col gap-4"
      description={t('settings.developerMenu.eventLog.desc')}>
      {/* Status bar */}
      <div className="flex flex-wrap items-center gap-2">
        <SettingsSelect
          value={scope}
          onChange={e => setScope(e.target.value === 'all' ? 'all' : 'active')}
          aria-label={t('settings.developerMenu.eventLog.workspaceScope')}
          inputSize="sm">
          <option value="active">
            {t('settings.developerMenu.eventLog.workspaceScopeActive')}
          </option>
          <option value="all">{t('settings.developerMenu.eventLog.workspaceScopeAll')}</option>
        </SettingsSelect>
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

      {/* Event stream.
          This is a live region, not a document, so it claims the height rather
          than taking a narrow measure: it was a `max-h-[60vh]` box that sized
          to its content, so with no events the whole panel was a filter bar,
          one grey sentence, and 700px of nothing -- indistinguishable from a
          page that failed to load. Bounded and framed, the same emptiness reads
          as a log that is connected and has not received anything yet, which is
          what it is. */}
      <section className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-line">
        <div
          ref={containerRef}
          onScroll={handleScroll}
          data-testid="event-log-scroll"
          className="min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
          {filteredEntries.length === 0 && (
            <div className="flex h-full flex-col items-center justify-center gap-1 text-center">
              <p className="text-sm text-content-secondary">
                {isLive
                  ? t('settings.developerMenu.eventLog.waiting')
                  : t('settings.developerMenu.eventLog.notConnected')}
              </p>
              <p className="max-w-[44ch] text-xs text-content-faint">
                {isLive
                  ? t('settings.developerMenu.eventLog.waitingHint')
                  : t('settings.developerMenu.eventLog.notConnectedHint')}
              </p>
            </div>
          )}
          {filteredEntries.map(entry => {
            const colors = DOMAIN_BADGE_COLORS[entry.domain] || {
              bg: 'bg-content-muted/20',
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
                {/* `min-w-0` is load-bearing: a flex item with `truncate` cannot
                    shrink below min-content without it, so this span would hold its
                    full width and the detail span beside it (which does set
                    `min-w-0`) would absorb every pixel of overflow and render as a
                    few characters — defeating the column it was added for. */}
                <span className="text-xs text-content truncate min-w-0">{entry.event}</span>
                {entry.detail && (
                  <span
                    className="text-[10px] text-content-muted truncate min-w-0 pt-0.5"
                    title={entry.detail}>
                    {entry.detail}
                  </span>
                )}
              </div>
            );
          })}
        </div>
      </section>
    </SettingsPanel>
  );
};

export default EventLogPanel;
