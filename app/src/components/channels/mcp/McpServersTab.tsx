/**
 * Top-level MCP Servers tab.
 *
 * Unified table view: shows both installed servers and registry catalog
 * results in a single table. Filter chips at the top let users toggle
 * between "All", "Installed", and "Registry" views.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import InstallDialog from './InstallDialog';
import InstalledServerDetail from './InstalledServerDetail';
import McpInventoryPanel from './McpInventoryPanel';
import { deriveAuthor } from './McpServerCard';
import type { ConnStatus, InstalledServer, ServerStatus, SmitheryServer } from './types';

const log = debug('mcp-clients:tab');
const POLL_INTERVAL_MS = 5_000;
const DEBOUNCE_MS = 300;
const PAGE_SIZE = 30;

type View =
  | { mode: 'home' }
  | { mode: 'detail'; serverId: string }
  | { mode: 'install'; qualifiedName: string; prefillEnv?: Record<string, string> };

type FilterChip = 'all' | 'installed' | 'registry';

const STATUS_DOT: Record<ServerStatus, string> = {
  connected: 'bg-sage-500',
  connecting: 'bg-amber-400',
  disconnected: 'bg-stone-300 dark:bg-neutral-600',
  error: 'bg-coral-500',
  disabled: 'bg-stone-200 dark:bg-neutral-700',
};

const McpServersTab = () => {
  const { t } = useT();
  const [servers, setServers] = useState<InstalledServer[]>([]);
  const [statuses, setStatuses] = useState<ConnStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [view, setView] = useState<View>({ mode: 'home' });
  const [inventoryOpen, setInventoryOpen] = useState(false);

  // Unified search + filter
  const [searchQuery, setSearchQuery] = useState('');
  const [activeChip, setActiveChip] = useState<FilterChip>('all');

  // Registry catalog results
  const [catalogServers, setCatalogServers] = useState<SmitheryServer[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogPage, setCatalogPage] = useState(1);
  const [catalogTotalPages, setCatalogTotalPages] = useState(1);

  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const requestSeqRef = useRef(0);

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

  const fetchCatalog = useCallback(async (query: string, page: number, append: boolean) => {
    const seq = ++requestSeqRef.current;
    setCatalogLoading(true);
    try {
      const result = await mcpClientsApi.registrySearch({
        query: query || undefined,
        page,
        page_size: PAGE_SIZE,
      });
      if (seq !== requestSeqRef.current) return;
      const incoming = result.servers ?? [];
      setCatalogServers(prev => (append ? [...prev, ...incoming] : incoming));
      setCatalogPage(result.page);
      setCatalogTotalPages(result.total_pages);
    } catch (err) {
      if (seq !== requestSeqRef.current) return;
      log('catalog fetch error: %o', err);
    } finally {
      if (seq === requestSeqRef.current) setCatalogLoading(false);
    }
  }, []);

  useEffect(() => {
    Promise.all([loadInstalled(), fetchStatuses()]).finally(() => setLoading(false));
  }, [loadInstalled, fetchStatuses]);

  // Fetch catalog on mount and when search changes
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      void fetchCatalog(searchQuery, 1, false);
    }, DEBOUNCE_MS);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [searchQuery, fetchCatalog]);

  // Poll status
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
    setView({ mode: 'detail', serverId });
  }, []);

  const handleSelectInstall = useCallback((qualifiedName: string) => {
    setView({ mode: 'install', qualifiedName });
  }, []);

  const handleInstallSuccess = useCallback(
    async (server: InstalledServer) => {
      await loadInstalled();
      await fetchStatuses();
      setView({ mode: 'detail', serverId: server.server_id });
    },
    [loadInstalled, fetchStatuses]
  );

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

  const handleLoadMore = () => {
    void fetchCatalog(searchQuery, catalogPage + 1, true);
  };

  const selectedServer =
    view.mode === 'detail' ? (servers.find(s => s.server_id === view.serverId) ?? null) : null;
  const selectedConnStatus =
    view.mode === 'detail' ? statuses.find(s => s.server_id === view.serverId) : undefined;

  // Filter installed servers by search
  const filteredInstalled = servers.filter(s => {
    if (!searchQuery.trim()) return true;
    const q = searchQuery.toLowerCase();
    return (
      s.display_name.toLowerCase().includes(q) ||
      s.qualified_name.toLowerCase().includes(q) ||
      (s.description ?? '').toLowerCase().includes(q)
    );
  });

  // Filter out catalog servers already installed
  const installedNames = new Set(servers.map(s => s.qualified_name));
  const filteredCatalog = catalogServers.filter(s => !installedNames.has(s.qualified_name));

  const statusMap = new Map(statuses.map(s => [s.server_id, s]));

  if (loading) {
    return (
      <div className="py-10 text-center text-sm text-stone-400 dark:text-neutral-500">
        {t('mcp.tab.loading')}
      </div>
    );
  }

  // Detail view
  if (view.mode === 'detail' && selectedServer) {
    return (
      <div className="space-y-3">
        <button
          type="button"
          onClick={() => setView({ mode: 'home' })}
          className="inline-flex items-center gap-1.5 text-xs font-medium text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 transition-colors">
          <svg
            className="w-3.5 h-3.5"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M15 19l-7-7 7-7" />
          </svg>
          {t('mcp.install.back')}
        </button>
        <InstalledServerDetail
          server={selectedServer}
          connStatus={selectedConnStatus}
          onUninstalled={serverId => void handleUninstalled(serverId)}
          onEnabledChange={(serverId, enabled) => void handleEnabledChange(serverId, enabled)}
        />
      </div>
    );
  }

  // Install view
  if (view.mode === 'install') {
    return (
      <div className="space-y-3">
        <button
          type="button"
          onClick={() => setView({ mode: 'home' })}
          className="inline-flex items-center gap-1.5 text-xs font-medium text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 transition-colors">
          <svg
            className="w-3.5 h-3.5"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M15 19l-7-7 7-7" />
          </svg>
          {t('mcp.install.back')}
        </button>
        <InstallDialog
          qualifiedName={view.qualifiedName}
          prefillEnv={view.prefillEnv}
          onSuccess={server => void handleInstallSuccess(server)}
          onCancel={() => setView({ mode: 'home' })}
        />
      </div>
    );
  }

  // Home view — unified table
  return (
    <div className="space-y-3">
      {/* Search + filter chips */}
      <div className="flex items-center gap-3">
        <input
          type="search"
          value={searchQuery}
          onChange={e => setSearchQuery(e.target.value)}
          placeholder={t('mcp.catalog.searchPlaceholder')}
          aria-label={t('mcp.catalog.searchAria')}
          className="flex-1 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40"
        />
        <button
          type="button"
          onClick={() => setInventoryOpen(true)}
          aria-label={t('mcp.inventory.openAria')}
          className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 px-3 py-2 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:bg-stone-50 dark:hover:bg-neutral-800">
          {t('mcp.inventory.openButton')}
        </button>
      </div>

      {/* Filter chips */}
      <div className="flex items-center gap-2">
        {(['all', 'installed', 'registry'] as FilterChip[]).map(chip => (
          <button
            key={chip}
            type="button"
            onClick={() => setActiveChip(chip)}
            className={`rounded-full px-3 py-1 text-xs font-medium transition-colors ${
              activeChip === chip
                ? 'bg-primary-500 text-white'
                : 'bg-stone-100 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300 hover:bg-stone-200 dark:hover:bg-neutral-700'
            }`}>
            {chip === 'all' && t('mcp.tab.filter.all')}
            {chip === 'installed' &&
              t('mcp.tab.filter.installed').replace('{count}', String(filteredInstalled.length))}
            {chip === 'registry' && t('mcp.tab.filter.registry')}
          </button>
        ))}
      </div>

      {loadError && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
          {loadError}
        </div>
      )}

      {/* Table */}
      <div className="rounded-lg border border-stone-200 dark:border-neutral-800 overflow-hidden">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-stone-100 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-900">
              <th className="text-left px-4 py-2.5 text-xs font-medium text-stone-500 dark:text-neutral-400">
                {t('mcp.tab.column.name')}
              </th>
              <th className="text-left px-4 py-2.5 text-xs font-medium text-stone-500 dark:text-neutral-400 hidden sm:table-cell w-36">
                {t('mcp.tab.column.author')}
              </th>
              <th className="text-right px-4 py-2.5 text-xs font-medium text-stone-500 dark:text-neutral-400 w-28">
                {t('mcp.tab.column.action')}
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-stone-100 dark:divide-neutral-800">
            {/* Installed servers */}
            {(activeChip === 'all' || activeChip === 'installed') &&
              filteredInstalled.map(server => {
                const status: ServerStatus =
                  statusMap.get(server.server_id)?.status ?? 'disconnected';
                return (
                  <tr
                    key={`installed-${server.server_id}`}
                    className="hover:bg-stone-50 dark:hover:bg-neutral-800/40 cursor-pointer transition-colors"
                    tabIndex={0}
                    role="button"
                    aria-label={t('mcp.tab.aria.viewDetails').replace(
                      '{name}',
                      server.display_name
                    )}
                    onClick={() => handleSelectServer(server.server_id)}
                    onKeyDown={e => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        handleSelectServer(server.server_id);
                      }
                    }}>
                    <td className="px-4 py-3">
                      <div className="flex items-center gap-2.5">
                        <span
                          className={`w-2 h-2 rounded-full shrink-0 ${STATUS_DOT[status]}`}
                          title={status}
                        />
                        <div className="min-w-0">
                          <span className="font-medium text-stone-900 dark:text-neutral-100 truncate block">
                            {server.display_name}
                          </span>
                          {server.description && (
                            <span className="text-xs text-stone-400 dark:text-neutral-500 line-clamp-4 block">
                              {server.description}
                            </span>
                          )}
                        </div>
                      </div>
                    </td>
                    <td className="px-4 py-3 hidden sm:table-cell">
                      <span className="text-xs text-stone-500 dark:text-neutral-400 truncate block">
                        {deriveAuthor(server.qualified_name) ?? '—'}
                      </span>
                    </td>
                    <td className="px-4 py-3 text-right">
                      <span className="text-xs text-primary-600 dark:text-primary-400 font-medium">
                        {t('mcp.tab.action.manage')}
                      </span>
                    </td>
                  </tr>
                );
              })}

            {/* Registry servers */}
            {(activeChip === 'all' || activeChip === 'registry') &&
              filteredCatalog.map(server => (
                <tr
                  key={`catalog-${server.qualified_name}`}
                  className="hover:bg-stone-50 dark:hover:bg-neutral-800/40 cursor-pointer transition-colors"
                  tabIndex={0}
                  role="button"
                  aria-label={t('mcp.tab.aria.installServer').replace(
                    '{name}',
                    server.display_name
                  )}
                  onClick={() => handleSelectInstall(server.qualified_name)}
                  onKeyDown={e => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      handleSelectInstall(server.qualified_name);
                    }
                  }}>
                  <td className="px-4 py-3">
                    <div className="flex items-center gap-2.5">
                      {server.icon_url ? (
                        <img
                          src={server.icon_url}
                          alt=""
                          className="w-5 h-5 rounded shrink-0 object-contain"
                        />
                      ) : (
                        <span className="w-5 h-5 rounded shrink-0 bg-primary-100 dark:bg-primary-500/20 flex items-center justify-center text-[10px]">
                          🔌
                        </span>
                      )}
                      <div className="min-w-0">
                        <span className="font-medium text-stone-900 dark:text-neutral-100 truncate block">
                          {server.display_name}
                        </span>
                        {server.description && (
                          <span className="text-xs text-stone-400 dark:text-neutral-500 line-clamp-4 block">
                            {server.description}
                          </span>
                        )}
                      </div>
                    </div>
                  </td>
                  <td className="px-4 py-3 hidden sm:table-cell">
                    <span className="text-xs text-stone-500 dark:text-neutral-400 truncate block">
                      {deriveAuthor(server.qualified_name) ?? '—'}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-right">
                    <span className="text-xs text-primary-600 dark:text-primary-400 font-medium">
                      {t('mcp.install.button')}
                    </span>
                  </td>
                </tr>
              ))}
          </tbody>
        </table>

        {/* Empty states */}
        {activeChip === 'installed' && filteredInstalled.length === 0 && (
          <div className="py-8 text-center text-sm text-stone-400 dark:text-neutral-500">
            {t('mcp.installed.empty')}
          </div>
        )}
        {activeChip === 'registry' && filteredCatalog.length === 0 && !catalogLoading && (
          <div className="py-8 text-center text-sm text-stone-400 dark:text-neutral-500">
            {searchQuery
              ? t('mcp.catalog.noResultsFor').replace('{query}', searchQuery)
              : t('mcp.catalog.noResults')}
          </div>
        )}
        {activeChip === 'all' &&
          filteredInstalled.length === 0 &&
          filteredCatalog.length === 0 &&
          !catalogLoading && (
            <div className="py-8 text-center text-sm text-stone-400 dark:text-neutral-500">
              {searchQuery
                ? t('mcp.catalog.noResultsFor').replace('{query}', searchQuery)
                : t('mcp.catalog.noResults')}
            </div>
          )}

        {/* Loading / load more */}
        {catalogLoading && (
          <div className="py-4 text-center text-xs text-stone-400 dark:text-neutral-500">
            {t('common.loading')}
          </div>
        )}
        {!catalogLoading &&
          catalogPage < catalogTotalPages &&
          (activeChip === 'all' || activeChip === 'registry') && (
            <div className="py-3 text-center border-t border-stone-100 dark:border-neutral-800">
              <button
                type="button"
                onClick={handleLoadMore}
                className="text-xs font-medium text-primary-600 dark:text-primary-400 hover:underline">
                {t('mcp.catalog.loadMore')}
              </button>
            </div>
          )}
      </div>

      {inventoryOpen && (
        <McpInventoryPanel
          servers={servers}
          onInstallServer={(qualifiedName, prefillEnv) => {
            setInventoryOpen(false);
            setView({ mode: 'install', qualifiedName, prefillEnv });
          }}
          onClose={() => setInventoryOpen(false)}
        />
      )}
    </div>
  );
};

export default McpServersTab;
