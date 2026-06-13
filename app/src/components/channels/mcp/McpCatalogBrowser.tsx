/**
 * MCP server catalog browser with debounced search and pagination.
 * Clicking "Install" on a card opens the InstallDialog flow.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import McpServerCard from './McpServerCard';
import type { SmitheryServer } from './types';

const log = debug('mcp-clients:catalog');
const DEBOUNCE_MS = 250;
const PAGE_SIZE = 20;

interface McpCatalogBrowserProps {
  onSelectInstall: (qualifiedName: string) => void;
}

const McpCatalogBrowser = ({ onSelectInstall }: McpCatalogBrowserProps) => {
  const { t } = useT();
  const [query, setQuery] = useState('');
  const [servers, setServers] = useState<SmitheryServer[]>([]);
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Monotonically-increasing counter used to discard stale registrySearch
  // responses when a newer request has already been issued.
  const requestSeqRef = useRef(0);

  const fetchPage = useCallback(
    async (searchQuery: string, pageNum: number, append: boolean) => {
      const seq = ++requestSeqRef.current;
      setLoading(true);
      setError(null);
      log('fetching page=%d query=%s seq=%d', pageNum, searchQuery, seq);
      try {
        const result = await mcpClientsApi.registrySearch({
          query: searchQuery || undefined,
          page: pageNum,
          page_size: PAGE_SIZE,
        });
        // Discard if a newer request has already been dispatched.
        if (seq !== requestSeqRef.current) {
          log('discarding stale response seq=%d (latest=%d)', seq, requestSeqRef.current);
          return;
        }
        setTotalPages(result.total_pages);
        setPage(result.page);
        // Guard against malformed envelope where `servers` is null/undefined.
        const incoming = result.servers ?? [];
        setServers(prev => (append ? [...prev, ...incoming] : incoming));
        log('loaded %d servers (append=%s)', incoming.length, append);
      } catch (err) {
        if (seq !== requestSeqRef.current) return;
        const msg = err instanceof Error ? err.message : t('mcp.catalog.loadFailed');
        log('catalog fetch error: %s', msg);
        setError(msg);
      } finally {
        if (seq === requestSeqRef.current) {
          setLoading(false);
        }
      }
    },
    [t]
  );

  // Debounce the query and reset to page 1 whenever it changes.
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      void fetchPage(query, 1, false);
    }, DEBOUNCE_MS);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [query, fetchPage]);

  const handleLoadMore = () => {
    void fetchPage(query, page + 1, true);
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <input
          type="search"
          aria-label={t('mcp.catalog.searchAria')}
          placeholder={t('mcp.catalog.searchPlaceholder')}
          value={query}
          onChange={e => setQuery(e.target.value)}
          className="flex-1 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40"
        />
      </div>

      {error && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {error}
        </div>
      )}

      {loading && servers.length === 0 ? (
        <div className="text-sm text-stone-400 dark:text-neutral-500 py-6 text-center">
          {t('common.loading')}
        </div>
      ) : servers.length === 0 ? (
        <div className="text-sm text-stone-400 dark:text-neutral-500 py-6 text-center">
          {query
            ? t('mcp.catalog.noResultsFor').replace('{query}', query)
            : t('mcp.catalog.noResults')}
        </div>
      ) : (
        <>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {servers.map(server => (
              <McpServerCard
                key={server.qualified_name}
                server={server}
                onSelect={onSelectInstall}
              />
            ))}
          </div>

          {page < totalPages && (
            <div className="flex justify-center pt-2">
              <button
                type="button"
                disabled={loading}
                onClick={handleLoadMore}
                className="rounded-lg border border-stone-200 dark:border-neutral-700 px-4 py-2 text-sm font-medium text-stone-600 dark:text-neutral-300 hover:border-stone-300 dark:hover:border-neutral-600 disabled:opacity-50">
                {loading ? t('common.loading') : t('mcp.catalog.loadMore')}
              </button>
            </div>
          )}
        </>
      )}
    </div>
  );
};

export default McpCatalogBrowser;
