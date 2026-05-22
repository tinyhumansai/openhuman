/**
 * Top-level MCP Servers tab component.
 * Two-pane layout: left = InstalledServerList + browse button,
 * right = selected server detail OR catalog browser OR install dialog.
 * Polls `status` every 5s while any server is connected.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import InstallDialog from './InstallDialog';
import InstalledServerDetail from './InstalledServerDetail';
import InstalledServerList from './InstalledServerList';
import McpCatalogBrowser from './McpCatalogBrowser';
import type { ConnStatus, InstalledServer } from './types';

const log = debug('mcp-clients:tab');
const POLL_INTERVAL_MS = 5_000;

type RightPane =
  | { mode: 'none' }
  | { mode: 'detail'; serverId: string }
  | { mode: 'catalog' }
  | { mode: 'install'; qualifiedName: string; prefillEnv?: Record<string, string> };

const McpServersTab = () => {
  const [servers, setServers] = useState<InstalledServer[]>([]);
  const [statuses, setStatuses] = useState<ConnStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [rightPane, setRightPane] = useState<RightPane>({ mode: 'none' });
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const loadInstalled = useCallback(async () => {
    log('loading installed servers');
    try {
      const installed = await mcpClientsApi.installedList();
      setServers(installed);
      // Clear any previous error on successful reload.
      setLoadError(null);
      log('loaded %d installed servers', installed.length);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to load installed servers';
      log('load error: %s', msg);
      setLoadError(msg);
    }
  }, []);

  const fetchStatuses = useCallback(async () => {
    log('polling statuses');
    try {
      const sv = await mcpClientsApi.status();
      setStatuses(sv);
    } catch (err) {
      log('status poll error: %o', err);
    }
  }, []);

  // Initial load — `loading` starts as `true` so no synchronous setState
  // before the async work is needed; just kick off the loads and clear on done.
  useEffect(() => {
    Promise.all([loadInstalled(), fetchStatuses()]).finally(() => setLoading(false));
  }, [loadInstalled, fetchStatuses]);

  // Poll status every 5s while at least one server is connected.
  useEffect(() => {
    const hasConnected = statuses.some(s => s.status === 'connected');
    if (!hasConnected) {
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
    log('selected server_id=%s', serverId);
    setRightPane({ mode: 'detail', serverId });
  }, []);

  const handleBrowseCatalog = useCallback(() => {
    log('opening catalog browser');
    setRightPane({ mode: 'catalog' });
  }, []);

  const handleSelectInstall = useCallback((qualifiedName: string) => {
    log('opening install dialog for %s', qualifiedName);
    setRightPane({ mode: 'install', qualifiedName });
  }, []);

  const handleInstallSuccess = useCallback(
    async (server: InstalledServer) => {
      log('install success server_id=%s, refreshing list', server.server_id);
      await loadInstalled();
      await fetchStatuses();
      setRightPane({ mode: 'detail', serverId: server.server_id });
    },
    [loadInstalled, fetchStatuses]
  );

  const handleUninstalled = useCallback(
    async (serverId: string) => {
      log('uninstalled server_id=%s', serverId);
      await loadInstalled();
      await fetchStatuses();
      setRightPane({ mode: 'none' });
    },
    [loadInstalled, fetchStatuses]
  );

  const selectedServerId = rightPane.mode === 'detail' ? rightPane.serverId : null;
  const selectedServer = servers.find(s => s.server_id === selectedServerId) ?? null;
  const selectedConnStatus = statuses.find(s => s.server_id === selectedServerId);

  if (loading) {
    return (
      <div className="py-10 text-center text-sm text-stone-400 dark:text-neutral-500">
        Loading MCP servers...
      </div>
    );
  }

  return (
    <div className="flex gap-4 h-full min-h-0">
      {/* Left pane: installed list */}
      <div className="w-56 shrink-0 flex flex-col">
        {loadError && (
          <div className="mb-2 rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
            {loadError}
          </div>
        )}
        <InstalledServerList
          servers={servers}
          statuses={statuses}
          selectedId={selectedServerId}
          onSelect={handleSelectServer}
          onBrowseCatalog={handleBrowseCatalog}
        />
      </div>

      {/* Right pane */}
      <div className="flex-1 min-w-0 overflow-y-auto">
        {rightPane.mode === 'none' && (
          <div className="h-full flex items-center justify-center text-sm text-stone-400 dark:text-neutral-500">
            Select a server or browse the catalog.
          </div>
        )}

        {rightPane.mode === 'catalog' && (
          <McpCatalogBrowser onSelectInstall={handleSelectInstall} />
        )}

        {rightPane.mode === 'install' && (
          <InstallDialog
            qualifiedName={rightPane.qualifiedName}
            prefillEnv={rightPane.prefillEnv}
            onSuccess={server => void handleInstallSuccess(server)}
            onCancel={() => setRightPane({ mode: 'catalog' })}
          />
        )}

        {rightPane.mode === 'detail' && selectedServer && (
          <InstalledServerDetail
            server={selectedServer}
            connStatus={selectedConnStatus}
            onUninstalled={serverId => void handleUninstalled(serverId)}
          />
        )}
      </div>
    </div>
  );
};

export default McpServersTab;
