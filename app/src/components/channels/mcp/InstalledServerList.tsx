/**
 * List of installed MCP servers with status dot, name, and tool count.
 *
 * Supports an optional `filter` prop that case-insensitively matches
 * against `display_name`, `qualified_name`, and `description`. When
 * filtering is active the list shows a "X of Y servers" count via a
 * `role="status"` live region so assistive tech announces the new
 * total as the user types. ArrowUp / ArrowDown move focus between
 * server buttons (clamped at the edges); Enter/Space activate via
 * the underlying `<button>` semantics.
 */
import { type KeyboardEvent as ReactKeyboardEvent, useMemo, useRef } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { ConnStatus, InstalledServer, ServerStatus } from './types';

interface InstalledServerListProps {
  servers: InstalledServer[];
  statuses: ConnStatus[];
  selectedId: string | null;
  onSelect: (serverId: string) => void;
  onBrowseCatalog: () => void;
  /** Optional case-insensitive filter applied to display_name / qualified_name / description. */
  filter?: string;
}

const STATUS_DOT: Record<ServerStatus, string> = {
  connected: 'bg-sage-500',
  connecting: 'bg-amber-400',
  disconnected: 'bg-stone-300 dark:bg-neutral-600',
  error: 'bg-coral-500',
};

// i18n keys for the per-status tooltip on the status dot. Reuses the
// existing `channels.status.*` namespace (already shipped as the canonical
// status vocabulary by McpStatusBadge) so we don't fork translations for
// the same four words.
const STATUS_I18N_KEYS: Record<ServerStatus, string> = {
  connected: 'channels.status.connected',
  connecting: 'channels.status.connecting',
  disconnected: 'channels.status.disconnected',
  error: 'channels.status.error',
};

const InstalledServerList = ({
  servers,
  statuses,
  selectedId,
  onSelect,
  onBrowseCatalog,
  filter = '',
}: InstalledServerListProps) => {
  const { t } = useT();
  const listRef = useRef<HTMLUListElement>(null);
  const statusMap = useMemo(() => new Map((statuses ?? []).map(s => [s.server_id, s])), [statuses]);

  const trimmedFilter = filter.trim().toLowerCase();
  const isFiltering = trimmedFilter.length > 0;
  const filteredServers = useMemo(() => {
    if (!trimmedFilter) return servers;
    return servers.filter(s => {
      const haystack = [s.display_name, s.qualified_name, s.description ?? '']
        .join(' ')
        .toLowerCase();
      return haystack.includes(trimmedFilter);
    });
  }, [servers, trimmedFilter]);

  const handleItemKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const root = listRef.current;
    if (!root) return;
    const buttons = Array.from(root.querySelectorAll<HTMLButtonElement>('button[data-server-id]'));
    const currentIdx = buttons.indexOf(event.currentTarget);
    if (currentIdx < 0) return;
    event.preventDefault();
    const nextIdx =
      event.key === 'ArrowDown'
        ? Math.min(currentIdx + 1, buttons.length - 1)
        : Math.max(currentIdx - 1, 0);
    if (nextIdx !== currentIdx) {
      buttons[nextIdx].focus();
    }
  };

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between mb-2">
        <h3 className="text-xs font-semibold text-stone-500 dark:text-neutral-400 uppercase tracking-wide">
          {t('mcp.installed.title')}
        </h3>
        <button
          type="button"
          onClick={onBrowseCatalog}
          className="text-xs text-primary-600 dark:text-primary-300 hover:underline font-medium">
          {t('mcp.installed.browseCatalog')}
        </button>
      </div>

      {servers.length === 0 ? (
        <div className="flex-1 flex flex-col items-center justify-center text-center gap-3 py-8">
          <p className="text-sm text-stone-400 dark:text-neutral-500">{t('mcp.installed.empty')}</p>
          <button
            type="button"
            onClick={onBrowseCatalog}
            className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-medium text-white hover:bg-primary-600 transition-colors">
            {t('mcp.installed.browseCatalog')}
          </button>
        </div>
      ) : (
        <>
          {isFiltering && (
            <p
              role="status"
              aria-live="polite"
              className="mb-2 text-[11px] text-stone-500 dark:text-neutral-400">
              {t('mcp.installed.search.countMatches')
                .replace('{shown}', String(filteredServers.length))
                .replace('{total}', String(servers.length))}
            </p>
          )}
          {filteredServers.length === 0 ? (
            <div className="flex-1 flex items-center justify-center text-center py-8">
              <p className="text-sm text-stone-400 dark:text-neutral-500">
                {t('mcp.installed.search.noMatches').replace('{query}', filter.trim())}
              </p>
            </div>
          ) : (
            <ul ref={listRef} className="space-y-1 flex-1 overflow-y-auto">
              {filteredServers.map(server => {
                const connStatus = statusMap.get(server.server_id);
                const status: ServerStatus = connStatus?.status ?? 'disconnected';
                const toolCount = connStatus?.tool_count ?? 0;
                const isSelected = selectedId === server.server_id;

                return (
                  <li key={server.server_id}>
                    <button
                      type="button"
                      data-server-id={server.server_id}
                      onClick={() => onSelect(server.server_id)}
                      onKeyDown={handleItemKeyDown}
                      className={`w-full flex items-center gap-2.5 rounded-lg px-3 py-2.5 text-left transition-colors ${
                        isSelected
                          ? 'bg-primary-50 dark:bg-primary-500/15 border border-primary-200 dark:border-primary-500/30'
                          : 'hover:bg-stone-50 dark:hover:bg-neutral-800/60 border border-transparent'
                      }`}>
                      <span
                        className={`w-2 h-2 rounded-full shrink-0 ${STATUS_DOT[status]}`}
                        title={t(STATUS_I18N_KEYS[status])}
                      />
                      <span className="flex-1 min-w-0">
                        <span className="block text-sm font-medium text-stone-800 dark:text-neutral-100 truncate">
                          {server.display_name}
                        </span>
                        {status === 'connected' && toolCount > 0 && (
                          <span className="block text-[11px] text-stone-400 dark:text-neutral-500">
                            {t(
                              toolCount === 1
                                ? 'mcp.installed.toolSingular'
                                : 'mcp.installed.toolPlural'
                            ).replace('{count}', String(toolCount))}
                          </span>
                        )}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </>
      )}
    </div>
  );
};

export default InstalledServerList;
