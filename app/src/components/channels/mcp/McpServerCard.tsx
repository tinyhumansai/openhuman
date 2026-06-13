/**
 * Card component for a single MCP registry server.
 * Shows icon, title, description, and author derived from qualified name.
 */
import type { SmitheryServer } from './types';

interface McpServerCardProps {
  server: SmitheryServer;
  onSelect: (qualifiedName: string) => void;
}

export function deriveAuthor(qualifiedName: string): string | null {
  const slashIdx = qualifiedName.indexOf('/');
  if (slashIdx < 1) return null;
  const prefix = qualifiedName.slice(0, slashIdx);
  const lastDot = prefix.lastIndexOf('.');
  return lastDot >= 0 ? prefix.slice(lastDot + 1) : prefix;
}

const McpServerCard = ({ server, onSelect }: McpServerCardProps) => {
  return (
    <button
      type="button"
      onClick={() => onSelect(server.qualified_name)}
      className="w-full text-left rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3 flex items-start gap-3 hover:border-primary-300 dark:hover:border-primary-500/40 hover:bg-stone-100/50 dark:hover:bg-neutral-800 transition-colors cursor-pointer">
      {server.icon_url ? (
        <img
          src={server.icon_url}
          alt=""
          className="w-8 h-8 rounded shrink-0 object-contain bg-white dark:bg-neutral-900"
        />
      ) : (
        <div className="w-8 h-8 rounded shrink-0 bg-primary-100 dark:bg-primary-500/20 flex items-center justify-center text-sm">
          🔌
        </div>
      )}
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium text-stone-900 dark:text-neutral-100 truncate">
          {server.display_name}
        </p>
        {server.description && (
          <p className="text-xs text-stone-500 dark:text-neutral-400 line-clamp-4 mt-0.5">
            {server.description}
          </p>
        )}
      </div>
    </button>
  );
};

export default McpServerCard;
