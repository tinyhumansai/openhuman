/**
 * Registry — browse the upstream MCP directories.
 *
 * The third tab of the MCP page. A hosted server that can be dialled as
 * listed (one http(s) endpoint, nothing to fill in) is added in one step: it
 * is declared in **mcp.json** under its registry name, then connected, or the
 * sign-in dialog opens when the server asks for credentials. Every other row
 * opens the server's own page — its website, or its source repository — where
 * the install instructions live, and the user declares it in the **mcp.json**
 * tab. Rows already declared show as added.
 *
 * Everything here is fenced inside this component's own state. The directories
 * are a network hop away and can be down; that is rendered as a notice inside
 * this panel with a retry, while the user's installed servers in the other
 * tabs go on rendering. A failed add is reported on its own row.
 */
import debug from 'debug';
import { Check, ExternalLink as ExternalLinkIcon, Loader2, Plus, Search } from 'lucide-react';
import { memo, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useDebouncedValue } from '../../../hooks/useDebouncedValue';
import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import { openUrl } from '../../../utils/openUrl';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Card from '../../ui/Card';
import TextField from '../../ui/TextField';
import ConnectAuthModal from './ConnectAuthModal';
import { mcpRegistryErrorMessage } from './mcpRegistryErrorMessage';
import { declareServer, type HostedEntry, isAddCandidate, resolveHostedEntry } from './registryAdd';
import type { InstalledServer, SmitheryServer } from './types';

const log = debug('mcp-clients:registry');
const DEBOUNCE_MS = 300;
const PAGE_SIZE = 30;

/** Transport classification a catalog row can be filtered by. */
export type Transport = 'hosted' | 'stdio';

type AddStep = 'resolve' | 'declare' | 'connect';
type RowPhase = 'adding' | 'connecting' | 'needsSetup' | 'error';
type RowState = { phase: RowPhase; message?: string; retryStep?: AddStep };
type PendingAdd = { entry?: HostedEntry; installed?: InstalledServer; retryStep?: AddStep };

const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err));

/**
 * Classify a catalog row by how it runs: `hosted` = reachable over an HTTP
 * endpoint; `stdio` = run on-device as a subprocess. `is_deployed` is set by
 * the registry adapter when the server exposes a remote.
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

/**
 * The page a row opens: the server's own site when it declares one, else its
 * repository, else its listing on the directory it came from. Every row has
 * *somewhere* to go, because a directory entry the user cannot read is not
 * one they can act on.
 */
export const serverPageUrl = (server: SmitheryServer): string => {
  if (server.website_url) return server.website_url;
  const repo = deriveRepoUrl(server.qualified_name);
  if (repo) return repo;
  if (server.source === 'smithery') {
    return `https://smithery.ai/server/${server.qualified_name}`;
  }
  return `https://registry.modelcontextprotocol.io/?search=${encodeURIComponent(
    server.qualified_name
  )}`;
};

/**
 * Collapse catalog entries to a single row per `qualified_name`. The registry
 * can return the same server within a page or across paginated "load more"
 * fetches; first occurrence wins so the earliest (highest-ranked) result is
 * the one kept.
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
 * External link that opens in the system browser. Stops propagation so clicking
 * a server's website/repo never also triggers the row's own open action.
 */
const ExternalLink = ({ href, label }: { href: string; label: string }) => (
  <Button
    variant="tertiary"
    size="xs"
    onClick={e => {
      e.stopPropagation();
      void openUrl(href).catch(() => {});
    }}
    trailingIcon={<ExternalLinkIcon className="size-3" aria-hidden="true" />}
    className="h-auto gap-1 p-0 text-[11px] font-normal text-primary-600 hover:underline dark:text-primary-400">
    {label}
  </Button>
);

/**
 * One catalog row. Memoized so a parent re-render (the installed list's status
 * poll) doesn't re-render the potentially large catalog: the data, the row's
 * add state and the handlers are stable or primitive, so memo skips every row
 * whose state did not change.
 */
const CatalogRow = memo(
  ({
    server,
    declared,
    phase,
    message,
    onOpen,
    onAdd,
    onRetry,
  }: {
    server: SmitheryServer;
    declared: boolean;
    phase?: RowPhase;
    message?: string;
    onOpen: (server: SmitheryServer) => void;
    onAdd: (server: SmitheryServer) => void;
    onRetry: (server: SmitheryServer) => void;
  }) => {
    const { t } = useT();
    const repoUrl = deriveRepoUrl(server.qualified_name);
    const hosted = transportOf(server) === 'hosted';
    const busy = phase === 'adding' || phase === 'connecting';
    const canAdd = isAddCandidate(server) && phase !== 'needsSetup';

    let action: ReactNode;
    if (busy) {
      action = (
        <Button
          variant="secondary"
          size="sm"
          disabled
          leadingIcon={<Loader2 className="size-3.5 animate-spin" aria-hidden="true" />}
          className="shrink-0">
          {t('mcp.registry.action.adding')}
        </Button>
      );
    } else if (declared) {
      action = (
        <Button
          variant="secondary"
          size="sm"
          disabled
          leadingIcon={<Check className="size-3.5" aria-hidden="true" />}
          className="shrink-0">
          {t('mcp.registry.action.added')}
        </Button>
      );
    } else if (canAdd) {
      action = (
        <Button
          variant="primary"
          size="sm"
          aria-label={t('mcp.registry.aria.add').replace('{name}', server.display_name)}
          onClick={() => onAdd(server)}
          leadingIcon={<Plus className="size-3.5" aria-hidden="true" />}
          className="shrink-0">
          {t('mcp.registry.action.add')}
        </Button>
      );
    } else {
      action = (
        <Button
          variant="secondary"
          size="sm"
          aria-label={t('mcp.tab.aria.openServerPage').replace('{name}', server.display_name)}
          onClick={() => onOpen(server)}
          trailingIcon={<ExternalLinkIcon className="size-3.5" aria-hidden="true" />}
          className="shrink-0">
          {t('mcp.tab.action.openPage')}
        </Button>
      );
    }

    return (
      <li className="space-y-2 py-3 first:pt-0 last:pb-0" data-testid="mcp-registry-row">
        <div className="flex items-start gap-3">
          <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-md border border-line bg-surface-muted text-xs font-semibold text-content-muted">
            {server.icon_url ? (
              <img src={server.icon_url} alt="" className="size-full object-contain" />
            ) : (
              server.display_name.charAt(0).toUpperCase()
            )}
          </span>
          <div className="min-w-0 flex-1 space-y-1">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-medium text-content">{server.display_name}</span>
              {server.official && (
                <Badge variant="success" title={t('mcp.tab.officialHint')}>
                  {t('mcp.tab.officialBadge')}
                </Badge>
              )}
              <Badge
                variant="neutral"
                title={t(hosted ? 'mcp.tab.transport.hostedHint' : 'mcp.tab.transport.localHint')}>
                {t(hosted ? 'mcp.tab.transport.hosted' : 'mcp.tab.transport.local')}
              </Badge>
            </div>
            {/* The registry is full of look-alike names (a dozen "gmail"
                servers); the slug is the unique identifier that tells them
                apart. */}
            <p className="truncate font-mono text-xs text-content-muted">{server.qualified_name}</p>
            {server.description && (
              <p className="line-clamp-3 text-xs text-content-muted">{server.description}</p>
            )}
            {(server.website_url || repoUrl) && (
              <p className="flex items-center gap-3">
                {server.website_url && (
                  <ExternalLink href={server.website_url} label={t('mcp.tab.link.website')} />
                )}
                {repoUrl && <ExternalLink href={repoUrl} label={t('mcp.tab.link.repo')} />}
              </p>
            )}
            {phase === 'needsSetup' && (
              <p className="text-xs text-content-muted" data-testid="mcp-registry-needs-setup">
                {t('mcp.registry.needsSetup')}
              </p>
            )}
            {phase === 'error' && (
              <div
                role="alert"
                data-testid="mcp-registry-row-error"
                className="space-y-1 text-xs text-coral-700 dark:text-coral-300">
                <p>{message}</p>
                <Button
                  variant="tertiary"
                  size="xs"
                  onClick={() => onRetry(server)}
                  className="h-auto p-0 text-primary-600 hover:underline dark:text-primary-400">
                  {t('common.retry')}
                </Button>
              </div>
            )}
          </div>
          {action}
        </div>
      </li>
    );
  }
);
CatalogRow.displayName = 'CatalogRow';

interface McpRegistryBrowserProps {
  /** Names already declared in mcp.json; those rows show as added. */
  installedNames: ReadonlySet<string>;
  /** Re-reads the installed servers after a row was declared. */
  onDeclared: () => Promise<void>;
}

const McpRegistryBrowser = ({ installedNames, onDeclared }: McpRegistryBrowserProps) => {
  const { t } = useT();
  const [searchQuery, setSearchQuery] = useState('');
  const [transportFilter, setTransportFilter] = useState<'all' | Transport>('all');
  const filters = useMemo(
    () => ({ query: searchQuery, transport: transportFilter }),
    [searchQuery, transportFilter]
  );
  const debouncedFilters = useDebouncedValue(filters, DEBOUNCE_MS);

  const [catalogServers, setCatalogServers] = useState<SmitheryServer[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogPage, setCatalogPage] = useState(1);
  const [catalogTotalPages, setCatalogTotalPages] = useState(1);
  // Set when a fetch fails so the tab shows an error state (with retry)
  // instead of silently falling back to an empty/stale catalog.
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const requestSeqRef = useRef(0);

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
        setCatalogError(null);
      } catch (err) {
        if (seq !== requestSeqRef.current) return;
        log('catalog fetch error: %o', err);
        // A fresh (non-append) fetch that fails leaves no usable rows — surface
        // the error. A failed "load more" keeps the rows already shown.
        if (!append) setCatalogError(mcpRegistryErrorMessage(err, t, 'mcp.catalog.loadFailed'));
      } finally {
        if (seq === requestSeqRef.current) setCatalogLoading(false);
      }
    },
    [t]
  );

  // Fetch page 1 on mount and whenever the query or transport filter changes.
  useEffect(() => {
    void fetchCatalog(debouncedFilters.query, debouncedFilters.transport, 1, false);
  }, [debouncedFilters, fetchCatalog]);

  const handleOpen = useCallback((server: SmitheryServer) => {
    const url = serverPageUrl(server);
    log('opening server page %s', url);
    void openUrl(url).catch(() => {});
  }, []);

  const [rowStates, setRowStates] = useState<ReadonlyMap<string, RowState>>(() => new Map());
  const pendingRef = useRef(new Map<string, PendingAdd>());
  const [authFor, setAuthFor] = useState<InstalledServer | null>(null);

  const setRowState = useCallback((name: string, state: RowState | null) => {
    setRowStates(prev => {
      const next = new Map(prev);
      if (state) next.set(name, state);
      else next.delete(name);
      return next;
    });
  }, []);

  const runAdd = useCallback(
    async (server: SmitheryServer, from: AddStep) => {
      const name = server.qualified_name;
      const pending = pendingRef.current.get(name) ?? {};
      pendingRef.current.set(name, pending);
      let step: AddStep = from;
      log('add %s from step=%s', name, step);
      try {
        if (step === 'resolve') {
          setRowState(name, { phase: 'adding' });
          const detail = await mcpClientsApi.registryGet(name);
          const resolved = resolveHostedEntry(detail, server);
          if (!resolved.ok) {
            setRowState(name, { phase: 'needsSetup' });
            return;
          }
          pending.entry = resolved.entry;
          step = 'declare';
        }
        if (step === 'declare') {
          setRowState(name, { phase: 'adding' });
          if (!pending.entry) throw new Error(t('mcp.registry.addFailed'));
          const outcome = await declareServer(name, pending.entry);
          log('add %s declare=%s', name, outcome);
          await onDeclared();
          const installed = await mcpClientsApi.installedList();
          const row = installed.find(s => s.qualified_name === name);
          if (!row) throw new Error(t('mcp.registry.addFailed'));
          pending.installed = row;
          step = 'connect';
        }
        const row = pending.installed;
        if (!row) throw new Error(t('mcp.registry.connectFailed'));
        setRowState(name, { phase: 'connecting' });
        let kind: 'none' | 'token' | 'oauth';
        try {
          kind = (await mcpClientsApi.detectAuth(row.server_id)).kind;
        } catch (err) {
          log('add %s detect_auth failed: %s', name, errorText(err));
          kind = 'token';
        }
        log('add %s auth=%s', name, kind);
        if (kind !== 'none') {
          setRowState(name, null);
          setAuthFor(row);
          return;
        }
        await mcpClientsApi.connect(row.server_id);
        log('add %s connected', name);
        pendingRef.current.delete(name);
        setRowState(name, null);
      } catch (err) {
        const reason = errorText(err);
        log('add %s failed at step=%s: %s', name, step, reason);
        pending.retryStep = step;
        const headline = t(
          step === 'connect' ? 'mcp.registry.connectFailed' : 'mcp.registry.addFailed'
        );
        setRowState(name, {
          phase: 'error',
          message: reason && reason !== headline ? `${headline} ${reason}` : headline,
          retryStep: step,
        });
      }
    },
    [onDeclared, setRowState, t]
  );

  const handleAdd = useCallback(
    (server: SmitheryServer) => void runAdd(server, 'resolve'),
    [runAdd]
  );

  const handleRetry = useCallback(
    (server: SmitheryServer) => {
      const retryStep = pendingRef.current.get(server.qualified_name)?.retryStep ?? 'resolve';
      void runAdd(server, retryStep);
    },
    [runAdd]
  );

  const catalogRows = useMemo(
    () =>
      catalogServers.map(server => {
        const state = rowStates.get(server.qualified_name);
        return (
          <CatalogRow
            key={`catalog-${server.qualified_name}`}
            server={server}
            declared={installedNames.has(server.qualified_name)}
            phase={state?.phase}
            message={state?.message}
            onOpen={handleOpen}
            onAdd={handleAdd}
            onRetry={handleRetry}
          />
        );
      }),
    [catalogServers, rowStates, installedNames, handleOpen, handleAdd, handleRetry]
  );

  return (
    <section className="space-y-3" data-testid="mcp-registry-browser">
      <h2 className="text-xs font-medium uppercase tracking-wide text-content-muted">
        {t('mcp.registry.title')}
      </h2>
      <p className="text-sm text-content-muted">{t('mcp.registry.intro')}</p>

      <div className="flex flex-wrap items-end gap-2">
        <TextField
          type="search"
          value={searchQuery}
          onChange={e => setSearchQuery(e.target.value)}
          placeholder={t('mcp.catalog.searchPlaceholder')}
          aria-label={t('mcp.catalog.searchAria')}
          className="min-w-48 flex-1"
        />
        <div
          className="flex flex-wrap items-center gap-2"
          role="group"
          aria-label={t('mcp.tab.transportFilter.aria')}>
          {(['stdio', 'hosted'] as const).map(tp => {
            const active = transportFilter === tp;
            return (
              <Button
                key={tp}
                variant={active ? 'primary' : 'secondary'}
                size="xs"
                aria-pressed={active}
                onClick={() => setTransportFilter(prev => (prev === tp ? 'all' : tp))}
                className={`rounded-full font-medium ${active ? 'bg-content text-surface' : ''}`}>
                {t(tp === 'stdio' ? 'mcp.tab.transport.local' : 'mcp.tab.transport.hosted')}
              </Button>
            );
          })}
        </div>
      </div>

      {catalogError && !catalogLoading ? (
        <div
          data-testid="mcp-catalog-error"
          className="space-y-2 rounded-md border border-coral-500/30 bg-coral-500/10 px-3 py-3 text-sm text-coral-700 dark:text-coral-300">
          <p>{catalogError}</p>
          <Button
            variant="tertiary"
            size="xs"
            onClick={() =>
              void fetchCatalog(debouncedFilters.query, debouncedFilters.transport, 1, false)
            }
            className="h-auto p-0 text-primary-600 hover:underline dark:text-primary-400">
            {t('common.retry')}
          </Button>
        </div>
      ) : (
        <Card padded divided={false}>
          {catalogLoading && catalogServers.length === 0 ? (
            <p className="flex items-center gap-1.5 text-xs text-content-muted">
              <Loader2 className="size-3 animate-spin" aria-hidden="true" />
              {t('common.loading')}
            </p>
          ) : catalogServers.length === 0 ? (
            <p className="text-sm text-content-muted" data-testid="mcp-catalog-empty">
              {searchQuery
                ? t('mcp.catalog.noResultsFor').replace('{query}', searchQuery)
                : t('mcp.catalog.noResults')}
            </p>
          ) : (
            <>
              <ul className="divide-y divide-line-subtle">{catalogRows}</ul>
              {!catalogLoading && catalogPage < catalogTotalPages && (
                <div className="mt-3 border-t border-line-subtle pt-3 text-center">
                  <Button
                    variant="tertiary"
                    size="xs"
                    onClick={() =>
                      void fetchCatalog(
                        debouncedFilters.query,
                        debouncedFilters.transport,
                        catalogPage + 1,
                        true
                      )
                    }
                    leadingIcon={<Search className="size-3" aria-hidden="true" />}
                    className="text-primary-600 hover:underline dark:text-primary-400">
                    {t('mcp.catalog.loadMore')}
                  </Button>
                </div>
              )}
              {catalogLoading && (
                <p className="mt-3 flex items-center gap-1.5 text-xs text-content-muted">
                  <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                  {t('common.loading')}
                </p>
              )}
            </>
          )}
        </Card>
      )}

      {authFor && (
        <ConnectAuthModal
          server={authFor}
          onClose={() => setAuthFor(null)}
          onConnected={() => {
            setAuthFor(null);
            void onDeclared();
          }}
        />
      )}
    </section>
  );
};

export default McpRegistryBrowser;
