/**
 * Top-level MCP Servers tab.
 *
 * Unified table view: shows both installed servers and registry catalog
 * results in a single table. Filter chips at the top let users toggle
 * between "All", "Installed", and "Registry" views.
 */
import debug from 'debug';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import { openUrl } from '../../../utils/openUrl';
import ChipTabs from '../../layout/ChipTabs';
import Button from '../../ui/Button';
import InstallDialog from './InstallDialog';
import InstalledServerDetail from './InstalledServerDetail';
import McpConnectionHealthToolbar from './McpConnectionHealthToolbar';
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

/**
 * Collapse catalog entries to a single row per `qualified_name`. The registry
 * can return the same server within a page or across paginated "load more"
 * fetches; without this the unified table renders duplicate rows and React
 * collides on the `catalog-<qualified_name>` key. First occurrence wins so the
 * earliest (highest-ranked) result is the one kept.
 */
const dedupeByQualifiedName = (servers: SmitheryServer[]): SmitheryServer[] => {
  const seen = new Set<string>();
  const out: SmitheryServer[] = [];
  for (const server of servers) {
    if (seen.has(server.qualified_name)) continue;
    seen.add(server.qualified_name);
    out.push(server);
  }
  return out;
};

/**
 * Collapse installed servers to one row per `qualified_name`. Install is
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

const STATUS_DOT: Record<ServerStatus, string> = {
  connected: 'bg-sage-500',
  connecting: 'bg-amber-400',
  disconnected: 'bg-surface-strong',
  unauthorized: 'bg-amber-500',
  error: 'bg-coral-500',
  disabled: 'bg-surface-strong',
};

/** Transport classification a catalog row can be filtered by. */
type Transport = 'hosted' | 'stdio';

/**
 * Classify a catalog row by how it runs: `hosted` = reachable over an HTTP
 * endpoint (cloud-run); `stdio` = installed and run on-device as a subprocess.
 * `is_deployed` is set by the registry adapter when the server exposes a remote.
 */
const transportOf = (server: SmitheryServer): Transport =>
  server.is_deployed ? 'hosted' : 'stdio';

/**
 * Derive a browsable source-repository URL from the registry slug. The official
 * registry namespaces community servers as `io.github.<user>/<repo>` (and
 * `io.gitlab.<user>/…`), which maps 1:1 to a repo page. Returns `null` for
 * vendor reverse-DNS slugs that don't encode a code host.
 */
export const deriveRepoUrl = (qualifiedName: string): string | null => {
  const slash = qualifiedName.indexOf('/');
  if (slash < 1) return null;
  const prefix = qualifiedName.slice(0, slash);
  const repo = qualifiedName.slice(slash + 1);
  if (!repo) return null;
  if (prefix.startsWith('io.github.')) {
    return `https://github.com/${prefix.slice('io.github.'.length)}/${repo}`;
  }
  if (prefix.startsWith('io.gitlab.')) {
    return `https://gitlab.com/${prefix.slice('io.gitlab.'.length)}/${repo}`;
  }
  return null;
};

/** Transport pill (Stdio vs Hosted) — the catalog's primary classification. */
const TransportBadge = ({ transport }: { transport: Transport }) => {
  const { t } = useT();
  const hosted = transport === 'hosted';
  return (
    <span
      title={t(hosted ? 'mcp.tab.transport.hostedHint' : 'mcp.tab.transport.localHint')}
      className={`inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
        hosted
          ? 'bg-primary-100 text-primary-700 dark:bg-primary-500/15 dark:text-primary-300'
          : 'bg-surface-strong text-content-muted'
      }`}>
      {t(hosted ? 'mcp.tab.transport.hosted' : 'mcp.tab.transport.local')}
    </span>
  );
};

/**
 * External link that opens in the system browser. Stops propagation so clicking
 * a server's website/repo never also triggers the row's install action.
 */
const ExternalLink = ({ href, label }: { href: string; label: string }) => (
  <button
    type="button"
    onClick={e => {
      e.stopPropagation();
      void openUrl(href).catch(() => {});
    }}
    className="inline-flex items-center gap-0.5 text-[11px] text-primary-600 dark:text-primary-400 hover:underline">
    {label}
    <svg
      className="w-2.5 h-2.5"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
      aria-hidden="true">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"
      />
    </svg>
  </button>
);

/**
 * One catalog (registry) row. Memoized so the 5s status poll — which re-renders
 * the parent tab — doesn't re-render the (potentially large) catalog list: the
 * catalog data and the install handler are stable across polls, so memo skips
 * every row. This is the main lever against the "rendering is slow" report.
 */
const CatalogRow = memo(
  ({
    server,
    onInstall,
  }: {
    server: SmitheryServer;
    onInstall: (qualifiedName: string) => void;
  }) => {
    const { t } = useT();
    const repoUrl = deriveRepoUrl(server.qualified_name);
    const author = deriveAuthor(server.qualified_name);
    return (
      <tr
        className="hover:bg-surface-muted dark:hover:bg-surface-muted/40 cursor-pointer transition-colors"
        tabIndex={0}
        role="button"
        aria-label={t('mcp.tab.aria.installServer').replace('{name}', server.display_name)}
        onClick={() => onInstall(server.qualified_name)}
        onKeyDown={e => {
          // Only act on keys aimed at the row itself — Enter/Space bubble up
          // from the nested Website/Repository buttons, which must not open
          // the install flow.
          if (e.target !== e.currentTarget) return;
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onInstall(server.qualified_name);
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
              <span className="flex items-center gap-1.5">
                <span className="font-medium text-content truncate">{server.display_name}</span>
                {server.official && (
                  <span
                    title={t('mcp.tab.officialHint')}
                    className="inline-flex items-center gap-0.5 rounded-full bg-sage-100 px-1.5 py-0.5 text-[10px] font-medium text-sage-700 dark:bg-sage-500/15 dark:text-sage-300">
                    ✓ {t('mcp.tab.officialBadge')}
                  </span>
                )}
              </span>
              {/* The registry is full of look-alike names (a dozen "gmail"
                  servers); the slug is the unique identifier that tells them
                  apart. */}
              <span className="text-[11px] font-mono text-content-faint truncate block">
                {server.qualified_name}
              </span>
              {server.description && (
                <span className="text-xs text-content-faint line-clamp-3 block">
                  {server.description}
                </span>
              )}
              {(server.website_url || repoUrl) && (
                <span className="flex items-center gap-3 mt-1">
                  {server.website_url && (
                    <ExternalLink href={server.website_url} label={t('mcp.tab.link.website')} />
                  )}
                  {repoUrl && <ExternalLink href={repoUrl} label={t('mcp.tab.link.repo')} />}
                </span>
              )}
            </div>
          </div>
        </td>
        <td className="px-4 py-3 hidden sm:table-cell">
          <TransportBadge transport={transportOf(server)} />
        </td>
        <td className="px-4 py-3 hidden sm:table-cell">
          <span className="text-xs text-content-muted truncate block">{author ?? '—'}</span>
        </td>
        <td className="px-4 py-3 text-right">
          <span className="text-xs text-primary-600 dark:text-primary-400 font-medium">
            {t('mcp.install.button')}
          </span>
        </td>
      </tr>
    );
  }
);
CatalogRow.displayName = 'CatalogRow';

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
  // Secondary classification filter over catalog rows: by transport.
  const [transportFilter, setTransportFilter] = useState<'all' | Transport>('all');

  // Registry catalog results
  const [catalogServers, setCatalogServers] = useState<SmitheryServer[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogPage, setCatalogPage] = useState(1);
  const [catalogTotalPages, setCatalogTotalPages] = useState(1);
  // Set when a registry fetch fails so the Registry view shows an error state
  // (with retry) instead of silently falling back to an empty/stale catalog.
  const [catalogError, setCatalogError] = useState(false);

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

  const fetchCatalog = useCallback(
    async (query: string, transport: 'all' | Transport, page: number, append: boolean) => {
      const seq = ++requestSeqRef.current;
      setCatalogLoading(true);
      try {
        const result = await mcpClientsApi.registrySearch({
          query: query || undefined,
          transport: transport === 'all' ? undefined : transport,
          page,
          page_size: PAGE_SIZE,
        });
        if (seq !== requestSeqRef.current) return;
        const incoming = result.servers ?? [];
        setCatalogServers(prev =>
          dedupeByQualifiedName(append ? [...prev, ...incoming] : incoming)
        );
        setCatalogPage(result.page);
        setCatalogTotalPages(result.total_pages);
        setCatalogError(false);
      } catch (err) {
        if (seq !== requestSeqRef.current) return;
        log('catalog fetch error: %o', err);
        // A fresh (non-append) fetch that fails leaves no usable rows — surface
        // the error. A failed "load more" keeps the rows already shown.
        if (!append) setCatalogError(true);
      } finally {
        if (seq === requestSeqRef.current) setCatalogLoading(false);
      }
    },
    []
  );

  useEffect(() => {
    Promise.all([loadInstalled(), fetchStatuses()]).finally(() => setLoading(false));
  }, [loadInstalled, fetchStatuses]);

  // Fetch catalog (page 1) on mount and whenever the query or transport filter
  // changes. Search + transport now run in the core over the cached full
  // catalog, so changing either re-queries from the top.
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      void fetchCatalog(searchQuery, transportFilter, 1, false);
    }, DEBOUNCE_MS);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [searchQuery, transportFilter, fetchCatalog]);

  // Poll status
  useEffect(() => {
    // Poll while anything is in a non-terminal state — not just `connected`.
    // An `unauthorized`/`error`/`connecting` server can transition (the
    // background reconnect supervisor, a completed OAuth sign-in, an expiring
    // token) and the UI must reflect that without a manual refresh (#3719 RC5).
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
    void fetchCatalog(searchQuery, transportFilter, catalogPage + 1, true);
  };

  // Bulk lifecycle actions for the health toolbar. One failure doesn't abort the
  // batch (allSettled), and we always refresh status so the dots reflect reality
  // — but if any call rejected we then throw so the toolbar can surface the
  // failure (otherwise a partial/total failure would look like success).
  const handleReconnectAll = useCallback(
    async (serverIds: string[]) => {
      log('reconnect all: %o', serverIds);
      const results = await Promise.allSettled(serverIds.map(id => mcpClientsApi.connect(id)));
      await fetchStatuses();
      if (results.some(r => r.status === 'rejected')) {
        throw new Error(t('mcp.health.opErrorGeneric'));
      }
    },
    [fetchStatuses, t]
  );

  const handleDisconnectAll = useCallback(
    async (serverIds: string[]) => {
      log('disconnect all: %o', serverIds);
      const results = await Promise.allSettled(serverIds.map(id => mcpClientsApi.disconnect(id)));
      await fetchStatuses();
      if (results.some(r => r.status === 'rejected')) {
        throw new Error(t('mcp.health.opErrorGeneric'));
      }
    },
    [fetchStatuses, t]
  );

  const selectedServer =
    view.mode === 'detail' ? (servers.find(s => s.server_id === view.serverId) ?? null) : null;
  const selectedConnStatus =
    view.mode === 'detail' ? statuses.find(s => s.server_id === view.serverId) : undefined;

  // One installed row per service. Install is idempotent server-side now, but
  // legacy double-installs can still exist on disk; collapse them by
  // qualified_name (servers arrive earliest-first) so the list stays "one per
  // thing". Raw `servers` is kept for server_id-keyed detail/status lookups.
  // Memoized so the 5s status poll doesn't rebuild + refilter the list.
  const filteredInstalled = useMemo(() => {
    const view = dedupeInstalledByQualifiedName(servers);
    const q = searchQuery.trim().toLowerCase();
    if (!q) return view;
    return view.filter(
      s =>
        s.display_name.toLowerCase().includes(q) ||
        s.qualified_name.toLowerCase().includes(q) ||
        (s.description ?? '').toLowerCase().includes(q)
    );
  }, [servers, searchQuery]);

  // Catalog rows minus already-installed servers. Search + transport filtering
  // now happen in the core over the cached full catalog (so relevance and
  // pagination are accurate); the only thing left to do client-side is hide
  // servers the user already installed. Memoized so the status poll doesn't
  // recompute it.
  const availableCatalog = useMemo(() => {
    const installedNames = new Set(servers.map(s => s.qualified_name));
    return catalogServers.filter(s => !installedNames.has(s.qualified_name));
  }, [catalogServers, servers]);

  const showRegistry = activeChip === 'all' || activeChip === 'registry';

  // Render catalog rows from memoized data so they survive parent re-renders
  // (the status poll) untouched — `CatalogRow` is itself memoized.
  const catalogRows = useMemo(
    () =>
      availableCatalog.map(server => (
        <CatalogRow
          key={`catalog-${server.qualified_name}`}
          server={server}
          onInstall={handleSelectInstall}
        />
      )),
    [availableCatalog, handleSelectInstall]
  );

  const statusMap = new Map(statuses.map(s => [s.server_id, s]));

  if (loading) {
    return (
      <div className="py-10 text-center text-sm text-content-faint">{t('mcp.tab.loading')}</div>
    );
  }

  // Detail view
  if (view.mode === 'detail' && selectedServer) {
    return (
      <div className="space-y-3">
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
          {t('mcp.install.back')}
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

  // Install view — InstallDialog renders its own "← Go back", so the tab does
  // NOT add a second one here (that was the duplicate back button).
  if (view.mode === 'install') {
    return (
      <div className="space-y-3">
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
          className="flex-1 rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40"
        />
        <Button
          variant="secondary"
          size="md"
          onClick={() => setInventoryOpen(true)}
          aria-label={t('mcp.inventory.openAria')}
          className="shrink-0">
          {t('mcp.inventory.openButton')}
        </Button>
      </div>

      {/* Filters on one bar: the scope (All / Installed / Registry) chips, then —
          when registry rows are visible — a labelled transport filter rendered
          as Stdio/Hosted TOGGLES (no second "All" chip; deselecting both means
          all). This avoids the duplicate-"All" confusion. */}
      <div className="flex flex-wrap items-center gap-2">
        <ChipTabs<FilterChip>
          className="flex flex-wrap items-center gap-2"
          value={activeChip}
          onChange={setActiveChip}
          items={[
            { id: 'all', label: t('mcp.tab.filter.all') },
            {
              id: 'installed',
              label: t('mcp.tab.filter.installed').replace(
                '{count}',
                String(filteredInstalled.length)
              ),
            },
            { id: 'registry', label: t('mcp.tab.filter.registry') },
          ]}
        />

        {showRegistry && (
          <>
            <span className="hidden sm:block h-5 w-px bg-line-subtle" aria-hidden="true" />
            <span className="text-xs font-medium text-content-muted">
              {t('mcp.tab.transportFilter.label')}
            </span>
            <div
              className="flex flex-wrap items-center gap-2"
              role="group"
              aria-label={t('mcp.tab.transportFilter.aria')}>
              {(['stdio', 'hosted'] as const).map(tp => {
                const active = transportFilter === tp;
                return (
                  <button
                    key={tp}
                    type="button"
                    aria-pressed={active}
                    onClick={() => setTransportFilter(prev => (prev === tp ? 'all' : tp))}
                    className={`rounded-full px-3 py-1 text-xs font-medium transition-colors ${
                      active
                        ? 'bg-content text-surface'
                        : 'border border-line text-content-muted hover:bg-surface-muted'
                    }`}>
                    {t(tp === 'stdio' ? 'mcp.tab.transport.local' : 'mcp.tab.transport.hosted')}
                  </button>
                );
              })}
            </div>
          </>
        )}
      </div>

      {loadError && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
          {loadError}
        </div>
      )}

      {/* Connection health + bulk lifecycle actions. Only meaningful once
          servers are installed; surfaces "Retry all" for error-state servers
          (the failed-connection retry affordance #4272 asks for) and
          "Disconnect all". Reads the polled statuses — no extra fetches. */}
      {(activeChip === 'all' || activeChip === 'installed') && statuses.length > 0 && (
        <McpConnectionHealthToolbar
          statuses={statuses}
          onReconnect={handleReconnectAll}
          onDisconnect={handleDisconnectAll}
        />
      )}

      {/* Table — horizontally scrollable so the Source/Author/Action columns
          aren't clipped when the panel is narrower than the table's natural
          width (the wrapper was `overflow-hidden`, which cut them off with no
          way to scroll). `min-w` keeps the columns readable rather than
          crushing them. */}
      <div className="rounded-lg border border-line overflow-x-auto">
        <table className="w-full min-w-[640px] text-sm">
          <thead>
            <tr className="border-b border-line-subtle bg-surface-muted">
              <th className="text-left px-4 py-2.5 text-xs font-medium text-content-muted">
                {t('mcp.tab.column.name')}
              </th>
              <th className="text-left px-4 py-2.5 text-xs font-medium text-content-muted hidden sm:table-cell w-28">
                {t('mcp.tab.column.type')}
              </th>
              <th className="text-left px-4 py-2.5 text-xs font-medium text-content-muted hidden sm:table-cell w-36">
                {t('mcp.tab.column.author')}
              </th>
              <th className="text-right px-4 py-2.5 text-xs font-medium text-content-muted w-28">
                {t('mcp.tab.column.action')}
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line-subtle dark:divide-neutral-800">
            {/* Installed servers */}
            {(activeChip === 'all' || activeChip === 'installed') &&
              filteredInstalled.map(server => {
                const status: ServerStatus =
                  statusMap.get(server.server_id)?.status ?? 'disconnected';
                return (
                  <tr
                    key={`installed-${server.server_id}`}
                    className="hover:bg-surface-muted dark:hover:bg-surface-muted/40 cursor-pointer transition-colors"
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
                          <span className="font-medium text-content truncate block">
                            {server.display_name}
                          </span>
                          {server.description && (
                            <span className="text-xs text-content-faint line-clamp-4 block">
                              {server.description}
                            </span>
                          )}
                        </div>
                      </div>
                    </td>
                    <td className="px-4 py-3 hidden sm:table-cell">
                      <span className="text-xs text-content-faint">—</span>
                    </td>
                    <td className="px-4 py-3 hidden sm:table-cell">
                      <span className="text-xs text-content-muted truncate block">
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

            {/* Registry servers — official first, then the registry's relevance
                order. Each row shows its transport, website/repo links, and the
                real auth surfaces on install. */}
            {showRegistry && catalogRows}
          </tbody>
        </table>

        {/* Registry fetch error — takes precedence over the empty state so a
            failed load reads as an error (with retry), not "no results". */}
        {showRegistry && catalogError && !catalogLoading && (
          <div
            data-testid="mcp-catalog-error"
            className="py-8 text-center text-sm text-coral-700 dark:text-coral-300 space-y-2">
            <p>{t('mcp.catalog.loadFailed')}</p>
            <Button
              variant="tertiary"
              size="xs"
              onClick={() => void fetchCatalog(searchQuery, transportFilter, 1, false)}
              className="text-primary-600 dark:text-primary-400 hover:underline">
              {t('common.retry')}
            </Button>
          </div>
        )}

        {/* Empty states */}
        {activeChip === 'installed' && filteredInstalled.length === 0 && (
          <div
            data-testid="mcp-installed-empty"
            className="py-8 text-center text-sm text-content-faint">
            {t('mcp.installed.empty')}
          </div>
        )}
        {activeChip === 'registry' &&
          availableCatalog.length === 0 &&
          !catalogLoading &&
          !catalogError && (
            <div
              data-testid="mcp-catalog-empty"
              className="py-8 text-center text-sm text-content-faint">
              {searchQuery
                ? t('mcp.catalog.noResultsFor').replace('{query}', searchQuery)
                : t('mcp.catalog.noResults')}
            </div>
          )}
        {activeChip === 'all' &&
          filteredInstalled.length === 0 &&
          availableCatalog.length === 0 &&
          !catalogLoading &&
          !catalogError && (
            <div
              data-testid="mcp-catalog-empty"
              className="py-8 text-center text-sm text-content-faint">
              {searchQuery
                ? t('mcp.catalog.noResultsFor').replace('{query}', searchQuery)
                : t('mcp.catalog.noResults')}
            </div>
          )}

        {/* Loading / load more */}
        {catalogLoading && (
          <div className="py-4 text-center text-xs text-content-faint">{t('common.loading')}</div>
        )}
        {!catalogLoading &&
          catalogPage < catalogTotalPages &&
          (activeChip === 'all' || activeChip === 'registry') && (
            <div className="py-3 text-center border-t border-line-subtle">
              <Button
                variant="tertiary"
                size="xs"
                onClick={handleLoadMore}
                className="text-primary-600 dark:text-primary-400 hover:underline">
                {t('mcp.catalog.loadMore')}
              </Button>
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
