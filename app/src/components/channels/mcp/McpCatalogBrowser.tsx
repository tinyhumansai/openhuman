/**
 * MCP server catalog browser with debounced search and pagination.
 * Clicking "Install" on a card opens the InstallDialog flow.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useDebouncedValue } from '../../../hooks/useDebouncedValue';
import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import Button from '../../ui/Button';
import Input from '../../ui/Input';
import { mcpRegistryErrorMessage } from './mcpRegistryErrorMessage';
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
  const debouncedQuery = useDebouncedValue(query, DEBOUNCE_MS);
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
        const msg = mcpRegistryErrorMessage(err, t, 'mcp.catalog.loadFailed');
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
  const lastEffectFetchRef = useRef<{ query: string; fetchPage: typeof fetchPage } | null>(null);

  // Reset to page 1 whenever the debounced query changes.
  useEffect(() => {
    const lastFetch = lastEffectFetchRef.current;
    if (lastFetch?.query === debouncedQuery && lastFetch.fetchPage === fetchPage) {
      return;
    }
    lastEffectFetchRef.current = { query: debouncedQuery, fetchPage };
    void fetchPage(debouncedQuery, 1, false);
  }, [debouncedQuery, fetchPage]);

  const handleLoadMore = () => {
    void fetchPage(debouncedQuery, page + 1, true);
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <Input
          type="search"
          aria-label={t('mcp.catalog.searchAria')}
          placeholder={t('mcp.catalog.searchPlaceholder')}
          value={query}
          onChange={e => setQuery(e.target.value)}
          className="flex-1"
        />
      </div>

      {error && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {error}
        </div>
      )}

      {loading && servers.length === 0 ? (
        <div className="text-sm text-content-faint py-6 text-center">{t('common.loading')}</div>
      ) : servers.length === 0 ? (
        <div className="text-sm text-content-faint py-6 text-center">
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
              <Button variant="secondary" size="md" disabled={loading} onClick={handleLoadMore}>
                {loading ? t('common.loading') : t('mcp.catalog.loadMore')}
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
};

export default McpCatalogBrowser;
