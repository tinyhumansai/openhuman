import { ArrowUpToLine, Download } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { getCoreHttpBaseUrl, getCoreRpcToken } from '../../../services/coreRpcClient';
import { Badge, type BadgeVariant } from '../../ui';
import Button from '../../ui/Button';
import DataTable, { type DataTableColumn } from '../../ui/DataTable';
import { SettingsSelect } from '../controls';
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
 * takes the neutral variant already used. Danger is deliberately left
 * unassigned: nothing here means "failure", and painting an ordinary domain in
 * the danger ramp would make routine events read as errors. The badge prints
 * the domain name either way. See `docs/gitbooks/en/developing/theming.md`.
 */
const DOMAIN_BADGE_VARIANT: Record<string, BadgeVariant> = {
  tool: 'primary',
  agent: 'success',
  system: 'neutral',
  memory: 'neutral',
  channel: 'neutral',
  cron: 'neutral',
  webhook: 'neutral',
  approval: 'warning',
  skill: 'neutral',
  composio: 'neutral',
  mcp_client: 'neutral',
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
  // Controlled paging so "Jump to latest" can return to page 1.
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
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

  // Newest first regardless of the stream's `new_entries` order: page 1 is
  // always "latest", so following the live tail is just staying on page 1.
  const orderedEntries =
    newEntriesRef.current === 'top' ? filteredEntries : [...filteredEntries].reverse();
  const pageCount = Math.max(1, Math.ceil(orderedEntries.length / pageSize));
  const currentPage = Math.min(page, pageCount);
  const pageRows = orderedEntries.slice((currentPage - 1) * pageSize, currentPage * pageSize);

  const columns: DataTableColumn<EventEntry>[] = [
    {
      id: 'time',
      header: t('settings.developerMenu.eventLog.column.time'),
      className: 'w-px whitespace-nowrap font-mono text-[11px] text-content-muted',
      cell: entry => entry.timestamp,
    },
    {
      id: 'domain',
      header: t('settings.developerMenu.eventLog.column.domain'),
      className: 'w-px whitespace-nowrap',
      cell: entry => (
        <Badge variant={DOMAIN_BADGE_VARIANT[entry.domain] ?? 'neutral'}>
          {DOMAIN_BADGE_KEYS[entry.domain]
            ? t(DOMAIN_BADGE_KEYS[entry.domain])
            : entry.domain.toUpperCase()}
        </Badge>
      ),
    },
    {
      id: 'agent',
      header: t('settings.developerMenu.eventLog.column.agent'),
      className: 'w-px whitespace-nowrap font-mono text-[11px] text-content-muted',
      cell: entry => entry.agent,
    },
    {
      id: 'event',
      header: t('settings.developerMenu.eventLog.column.event'),
      // `max-w-0 w-full` lets the cell truncate instead of widening the table.
      className: 'w-full max-w-0',
      cell: entry => (
        <div className="min-w-0 space-y-0.5">
          <p className="truncate text-xs text-content" title={entry.event}>
            {entry.event}
          </p>
          {entry.detail && (
            <p className="truncate text-[11px] text-content-muted" title={entry.detail}>
              {entry.detail}
            </p>
          )}
        </div>
      ),
    },
  ];

  return (
    // Non-scrolling page body: the table card fills it and only rows scroll.
    <SettingsPanel
      testId="event-log-panel"
      scrollable={false}
      bodyClassName="flex h-full min-h-0 flex-col gap-4"
      description={t('settings.developerMenu.eventLog.desc')}>
      <DataTable<EventEntry>
        testId="event-log-scroll"
        title={t('settings.developerMenu.eventLog.tableTitle')}
        description={
          <span className="inline-flex items-center gap-2">
            <Badge variant={isLive ? 'success' : 'neutral'} data-testid="event-log-status">
              {isLive
                ? t('settings.developerMenu.eventLog.live')
                : t('settings.developerMenu.eventLog.disconnected')}
            </Badge>
            <span>
              {filteredEntries.length} {t('settings.developerMenu.eventLog.events')}
            </span>
          </span>
        }
        actions={
          <>
            {currentPage > 1 && (
              <Button
                type="button"
                variant="tertiary"
                size="sm"
                leadingIcon={<ArrowUpToLine className="h-3.5 w-3.5" aria-hidden />}
                onClick={() => setPage(1)}>
                {t('settings.developerMenu.eventLog.jumpToLatest')}
              </Button>
            )}
            <Button
              type="button"
              variant="secondary"
              size="sm"
              leadingIcon={<Download className="h-3.5 w-3.5" aria-hidden />}
              onClick={exportLog}
              disabled={filteredEntries.length === 0}>
              {t('settings.developerMenu.eventLog.download')}
            </Button>
          </>
        }
        toolbarStart={
          <>
            <SettingsSelect
              value={scope}
              onChange={e => {
                setScope(e.target.value === 'all' ? 'all' : 'active');
                setPage(1);
              }}
              aria-label={t('settings.developerMenu.eventLog.workspaceScope')}
              inputSize="sm">
              <option value="active">
                {t('settings.developerMenu.eventLog.workspaceScopeActive')}
              </option>
              <option value="all">{t('settings.developerMenu.eventLog.workspaceScopeAll')}</option>
            </SettingsSelect>
            <SettingsSelect
              value={filterType}
              onChange={e => {
                setFilterType(e.target.value);
                setPage(1);
              }}
              aria-label={t('settings.developerMenu.eventLog.allTypes')}
              inputSize="sm">
              <option value="">{t('settings.developerMenu.eventLog.allTypes')}</option>
              {domains.map(d => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </SettingsSelect>
          </>
        }
        search={{
          value: filterText,
          onChange: value => {
            setFilterText(value);
            setPage(1);
          },
          placeholder: t('settings.developerMenu.eventLog.filterAgent'),
          ariaLabel: t('settings.developerMenu.eventLog.filterAgent'),
        }}
        columns={columns}
        rows={pageRows}
        rowKey={entry => String(entry.id)}
        pagination={{
          page: currentPage,
          pageSize,
          total: orderedEntries.length,
          pageSizeOptions: [25, 50, 100, 200],
          onPageChange: setPage,
          onPageSizeChange: setPageSize,
          testId: 'event-log-pagination',
        }}
        empty={
          <div className="flex flex-col items-center gap-1 text-center">
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
        }
        ariaLabel={t('settings.developerMenu.eventLog.title')}
      />
    </SettingsPanel>
  );
};

export default EventLogPanel;
