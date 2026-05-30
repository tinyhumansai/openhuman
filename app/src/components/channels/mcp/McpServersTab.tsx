/**
 * Top-level MCP Servers tab component.
 * Two-pane layout: left = InstalledServerList + browse button,
 * right = selected server detail OR catalog browser OR install dialog.
 * Polls `status` every 5s while any server is connected.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import InstallDialog from './InstallDialog';
import InstalledServerDetail from './InstalledServerDetail';
import InstalledServerList from './InstalledServerList';
import McpCatalogBrowser from './McpCatalogBrowser';
import McpConnectionHealthToolbar from './McpConnectionHealthToolbar';
import McpInventoryPanel from './McpInventoryPanel';
import McpServerSearch from './McpServerSearch';
import type { ConnStatus, InstalledServer } from './types';

const log = debug('mcp-clients:tab');
const POLL_INTERVAL_MS = 5_000;

type RightPane =
  | { mode: 'none' }
  | { mode: 'detail'; serverId: string }
  | { mode: 'catalog' }
  | { mode: 'install'; qualifiedName: string; prefillEnv?: Record<string, string> };

const McpServersTab = () => {
  const { t } = useT();
  const [servers, setServers] = useState<InstalledServer[]>([]);
  const [statuses, setStatuses] = useState<ConnStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [rightPane, setRightPane] = useState<RightPane>({ mode: 'none' });
  // Local-only filter for the installed-server list. Not persisted — the
  // search is a transient scan helper, not a saved view.
  const [searchFilter, setSearchFilter] = useState('');
  // Sharable Inventory modal toggle. Local state — the manifest UX is
  // a one-off interaction, not a saved view.
  const [inventoryOpen, setInventoryOpen] = useState(false);
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const loadInstalled = useCallback(async () => {
    log('loading installed servers');
    try {
      const installed = await mcpClientsApi.installedList();
      // Defensive: API contract guarantees an array, but if a future regression
      // or malformed envelope returns `undefined`, downstream `.find` crashes
      // the entire tab. Normalise here.
      setServers(Array.isArray(installed) ? installed : []);
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
      // Defensive: same reasoning as `loadInstalled` — `.find` / `.map`
      // downstream cannot tolerate an undefined array.
      setStatuses(Array.isArray(sv) ? sv : []);
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

  // Count rejected settlements and, if any, throw a descriptive error so the
  // toolbar surfaces it through its `role="alert"` region — otherwise a bulk
  // action that partially (or wholly) fails looks identical to success and
  // the user is left re-scanning the status dots. The status refresh still
  // runs first so the dots reconcile regardless of the partial failure.
  const reportBulkFailures = useCallback(
    (results: PromiseSettledResult<unknown>[], total: number) => {
      const failed = results.filter(r => r.status === 'rejected').length;
      if (failed > 0) {
        log('bulk op partial failure: %d/%d failed', failed, total);
        throw new Error(
          t('mcp.health.bulkPartialFailure')
            .replace('{failed}', String(failed))
            .replace('{total}', String(total))
        );
      }
    },
    [t]
  );

  // Bulk Retry — iterate through errored servers, collect per-server outcomes
  // via `Promise.allSettled` so one bad apple doesn't abort the batch, then
  // refresh statuses once at the end. The toolbar shows its own disabled state
  // during the await; the next poll tick reconciles any drift. Partial/total
  // failures are surfaced via `reportBulkFailures`.
  const handleBulkReconnect = useCallback(
    async (serverIds: string[]) => {
      log('bulk reconnect ids=%o', serverIds);
      const results = await Promise.allSettled(serverIds.map(id => mcpClientsApi.connect(id)));
      await fetchStatuses();
      reportBulkFailures(results, serverIds.length);
    },
    [fetchStatuses, reportBulkFailures]
  );

  // Bulk Disconnect — same shape as bulk reconnect. The toolbar gates this
  // behind a confirmation dialog before we get here.
  const handleBulkDisconnect = useCallback(
    async (serverIds: string[]) => {
      log('bulk disconnect ids=%o', serverIds);
      const results = await Promise.allSettled(serverIds.map(id => mcpClientsApi.disconnect(id)));
      await fetchStatuses();
      reportBulkFailures(results, serverIds.length);
    },
    [fetchStatuses, reportBulkFailures]
  );

  const selectedServerId = rightPane.mode === 'detail' ? rightPane.serverId : null;
  const selectedServer = servers.find(s => s.server_id === selectedServerId) ?? null;
  const selectedConnStatus = statuses.find(s => s.server_id === selectedServerId);

  if (loading) {
    return (
      <div className="py-10 text-center text-sm text-stone-400 dark:text-neutral-500">
        {t('mcp.tab.loading')}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 h-full min-h-0">
      <div className="flex items-center gap-2">
        <div
          role="status"
          className="flex-1 flex items-start gap-2 rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
          <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[9px] font-semibold uppercase tracking-wider bg-amber-200/70 dark:bg-amber-500/30 text-amber-900 dark:text-amber-100 shrink-0 mt-0.5">
            {t('mcp.alphaBadge')}
          </span>
          <span className="leading-relaxed">{t('mcp.alphaBannerText')}</span>
        </div>
        <button
          type="button"
          onClick={() => setInventoryOpen(true)}
          aria-label={t('mcp.inventory.openAria')}
          className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 px-3 py-2 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800">
          {t('mcp.inventory.openButton')}
        </button>
      </div>
      <div className="flex gap-4 flex-1 min-h-0">
        {/* Left pane: health toolbar + search + installed list */}
        <div className="w-56 shrink-0 flex flex-col">
          {loadError && (
            <div className="mb-2 rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
              {loadError}
            </div>
          )}
          <McpConnectionHealthToolbar
            statuses={statuses}
            onReconnect={handleBulkReconnect}
            onDisconnect={handleBulkDisconnect}
          />
          {servers.length > 0 && (
            <div className="mb-2">
              <McpServerSearch value={searchFilter} onChange={setSearchFilter} />
            </div>
          )}
          <InstalledServerList
            servers={servers}
            statuses={statuses}
            selectedId={selectedServerId}
            onSelect={handleSelectServer}
            onBrowseCatalog={handleBrowseCatalog}
            filter={searchFilter}
          />
        </div>

        {/* Right pane */}
        <div className="flex-1 min-w-0 overflow-y-auto">
          {rightPane.mode === 'none' && (
            <div className="h-full flex items-center justify-center text-sm text-stone-400 dark:text-neutral-500">
              {t('mcp.tab.emptyDetail')}
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
      {inventoryOpen && (
        <McpInventoryPanel
          servers={servers}
          onInstallServer={(qualifiedName, prefillEnv) => {
            // Hand the entry off to the existing install-dialog flow.
            // The panel closes itself; here we open the dialog with the
            // env keys pre-populated so the user only has to fill values.
            setRightPane({ mode: 'install', qualifiedName, prefillEnv });
          }}
          onClose={() => setInventoryOpen(false)}
        />
      )}
    </div>
  );
};

export default McpServersTab;
