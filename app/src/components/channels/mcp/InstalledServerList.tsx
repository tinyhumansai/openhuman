/**
 * List of installed MCP servers with status dot, name, and tool count.
 */
import type { ConnStatus, InstalledServer, ServerStatus } from './types';

interface InstalledServerListProps {
  servers: InstalledServer[];
  statuses: ConnStatus[];
  selectedId: string | null;
  onSelect: (serverId: string) => void;
  onBrowseCatalog: () => void;
}

const STATUS_DOT: Record<ServerStatus, string> = {
  connected: 'bg-sage-500',
  connecting: 'bg-amber-400',
  disconnected: 'bg-stone-300 dark:bg-neutral-600',
  error: 'bg-coral-500',
};

const InstalledServerList = ({
  servers,
  statuses,
  selectedId,
  onSelect,
  onBrowseCatalog,
}: InstalledServerListProps) => {
  const statusMap = new Map(statuses.map(s => [s.server_id, s]));

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between mb-2">
        <h3 className="text-xs font-semibold text-stone-500 dark:text-neutral-400 uppercase tracking-wide">
          Installed
        </h3>
        <button
          type="button"
          onClick={onBrowseCatalog}
          className="text-xs text-primary-600 dark:text-primary-300 hover:underline font-medium">
          Browse catalog
        </button>
      </div>

      {servers.length === 0 ? (
        <div className="flex-1 flex flex-col items-center justify-center text-center gap-3 py-8">
          <p className="text-sm text-stone-400 dark:text-neutral-500">
            No MCP servers installed yet.
          </p>
          <button
            type="button"
            onClick={onBrowseCatalog}
            className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-medium text-white hover:bg-primary-600 transition-colors">
            Browse catalog
          </button>
        </div>
      ) : (
        <ul className="space-y-1 flex-1 overflow-y-auto">
          {servers.map(server => {
            const connStatus = statusMap.get(server.server_id);
            const status: ServerStatus = connStatus?.status ?? 'disconnected';
            const toolCount = connStatus?.tool_count ?? 0;
            const isSelected = selectedId === server.server_id;

            return (
              <li key={server.server_id}>
                <button
                  type="button"
                  onClick={() => onSelect(server.server_id)}
                  className={`w-full flex items-center gap-2.5 rounded-lg px-3 py-2.5 text-left transition-colors ${
                    isSelected
                      ? 'bg-primary-50 dark:bg-primary-500/15 border border-primary-200 dark:border-primary-500/30'
                      : 'hover:bg-stone-50 dark:hover:bg-neutral-800/60 border border-transparent'
                  }`}>
                  <span
                    className={`w-2 h-2 rounded-full shrink-0 ${STATUS_DOT[status]}`}
                    title={status}
                  />
                  <span className="flex-1 min-w-0">
                    <span className="block text-sm font-medium text-stone-800 dark:text-neutral-100 truncate">
                      {server.display_name}
                    </span>
                    {status === 'connected' && toolCount > 0 && (
                      <span className="block text-[11px] text-stone-400 dark:text-neutral-500">
                        {toolCount} tool{toolCount !== 1 ? 's' : ''}
                      </span>
                    )}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
};

export default InstalledServerList;
