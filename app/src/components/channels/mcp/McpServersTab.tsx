/**
 * The body of the MCP page: one of its three notations, as `McpServersPage`
 * picked it in the header.
 *
 * **Servers** is the rows (`McpServerRows`) and, once a row is opened, that
 * server's detail with the credential form and its tools. **mcp.json** is the
 * same configuration as one document. **Registry** is the directories, where a
 * hosted server can be added in one step. The rows and their statuses are read
 * here rather than in the rows component so a save in the document tab or an
 * add from the directory can re-read them, and so the directory can mark what
 * is already declared.
 */
import debug from 'debug';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import { Alert, AlertDescription } from '../../ui';
import Button from '../../ui/Button';
import InstalledServerDetail from './InstalledServerDetail';
import McpJsonEditor from './McpJsonEditor';
import McpRegistryBrowser from './McpRegistryBrowser';
import McpServerRows from './McpServerRows';
import type { ConnStatus, InstalledServer } from './types';

const log = debug('mcp-clients:tab');
const POLL_INTERVAL_MS = 5_000;

/** The three notations. `id` doubles as the chip's stable identifier. */
export type McpPageTab = 'servers' | 'json' | 'registry';

type View = { mode: 'home' } | { mode: 'detail'; serverId: string };

/**
 * Collapse installed servers to one row per `qualified_name`. Declaring is
 * idempotent in the core now, but pre-existing double-installs can linger on
 * disk; the first occurrence (earliest install) is kept.
 */
const dedupeInstalledByQualifiedName = (servers: InstalledServer[]): InstalledServer[] => {
  const seen = new Set<string>();
  const out: InstalledServer[] = [];
  for (const server of servers) {
    if (seen.has(server.qualified_name)) continue;
    seen.add(server.qualified_name);
    out.push(server);
  }
  return out;
};

interface McpServersTabProps {
  tab: McpPageTab;
  onTabChange: (tab: McpPageTab) => void;
}

const McpServersTab = ({ tab, onTabChange }: McpServersTabProps) => {
  const { t } = useT();
  const [servers, setServers] = useState<InstalledServer[]>([]);
  const [statuses, setStatuses] = useState<ConnStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [view, setView] = useState<View>({ mode: 'home' });
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const loadInstalled = useCallback(async () => {
    log('loading installed servers');
    try {
      const installed = await mcpClientsApi.installedList();
      setServers(Array.isArray(installed) ? installed : []);
      setLoadError(null);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to load installed servers';
      setLoadError(msg);
    }
  }, []);

  const fetchStatuses = useCallback(async () => {
    try {
      const sv = await mcpClientsApi.status();
      setStatuses(Array.isArray(sv) ? sv : []);
    } catch (err) {
      log('status poll error: %o', err);
    }
  }, []);

  useEffect(() => {
    Promise.all([loadInstalled(), fetchStatuses()]).finally(() => setLoading(false));
  }, [loadInstalled, fetchStatuses]);

  // Poll status while anything is in a non-terminal state — not just
  // `connected`. An `unauthorized`/`error`/`connecting` server can transition
  // (the background reconnect supervisor, a completed OAuth sign-in, an
  // expiring token) and the UI must reflect that without a manual refresh.
  useEffect(() => {
    const hasActive = statuses.some(
      s =>
        s.status === 'connected' ||
        s.status === 'connecting' ||
        s.status === 'unauthorized' ||
        s.status === 'error'
    );
    if (!hasActive) {
      if (pollTimerRef.current) {
        clearTimeout(pollTimerRef.current);
        pollTimerRef.current = null;
      }
      return;
    }
    const schedule = () => {
      pollTimerRef.current = setTimeout(async () => {
        await fetchStatuses();
        schedule();
      }, POLL_INTERVAL_MS);
    };
    schedule();
    return () => {
      if (pollTimerRef.current) {
        clearTimeout(pollTimerRef.current);
        pollTimerRef.current = null;
      }
    };
  }, [statuses, fetchStatuses]);

  const handleSelectServer = useCallback((serverId: string) => {
    setView({ mode: 'detail', serverId });
  }, []);

  const handleUninstalled = useCallback(
    async (_serverId: string) => {
      await loadInstalled();
      await fetchStatuses();
      setView({ mode: 'home' });
    },
    [loadInstalled, fetchStatuses]
  );

  const handleEnabledChange = useCallback(
    async (_serverId: string, _enabled: boolean) => {
      await loadInstalled();
      await fetchStatuses();
    },
    [loadInstalled, fetchStatuses]
  );

  // A save in the document tab adds, rewrites or removes rows; re-read them so
  // the Servers tab describes the configuration as it is after the write. The
  // core connects new servers in the background, so the status poll picks the
  // rest up.
  const handleDocumentSaved = useCallback(async () => {
    await loadInstalled();
    await fetchStatuses();
  }, [loadInstalled, fetchStatuses]);

  const selectedServer =
    view.mode === 'detail' ? (servers.find(s => s.server_id === view.serverId) ?? null) : null;
  const selectedConnStatus =
    view.mode === 'detail' ? statuses.find(s => s.server_id === view.serverId) : undefined;

  // One row per service; raw `servers` is kept for server_id-keyed
  // detail/status lookups.
  const rows = useMemo(() => dedupeInstalledByQualifiedName(servers), [servers]);
  const installedNames = useMemo(() => new Set(servers.map(s => s.qualified_name)), [servers]);

  // Leaving the Servers tab closes any open detail, so coming back lands on the
  // rows rather than on a server the user has stopped looking at.
  useEffect(() => {
    if (tab !== 'servers') setView({ mode: 'home' });
  }, [tab]);

  if (tab === 'json') {
    return <McpJsonEditor onSaved={() => void handleDocumentSaved()} />;
  }

  if (tab === 'registry') {
    return <McpRegistryBrowser installedNames={installedNames} onDeclared={handleDocumentSaved} />;
  }

  if (loading) {
    return (
      <div className="py-10 text-center text-sm text-content-faint">{t('mcp.tab.loading')}</div>
    );
  }

  // Detail view — a server's own page, reached from a row's name. The page
  // body does not scroll on this tab (the rows table owns its scroll), so the
  // detail view brings its own.
  if (view.mode === 'detail' && selectedServer) {
    return (
      <div className="-mr-4 h-full space-y-3 overflow-y-auto pb-4 pr-4">
        <Button
          variant="tertiary"
          size="xs"
          onClick={() => setView({ mode: 'home' })}
          leadingIcon={
            <svg
              className="w-3.5 h-3.5"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M15 19l-7-7 7-7" />
            </svg>
          }>
          {t('mcp.tab.back')}
        </Button>
        <InstalledServerDetail
          server={selectedServer}
          connStatus={selectedConnStatus}
          onUninstalled={serverId => void handleUninstalled(serverId)}
          onEnabledChange={(serverId, enabled) => void handleEnabledChange(serverId, enabled)}
        />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      {loadError && (
        <Alert variant="destructive" density="compact">
          <AlertDescription>{loadError}</AlertDescription>
        </Alert>
      )}

      <McpServerRows
        servers={rows}
        statuses={statuses}
        onOpen={handleSelectServer}
        onChanged={handleDocumentSaved}
        onAddInJson={() => onTabChange('json')}
      />
    </div>
  );
};

export default McpServersTab;
