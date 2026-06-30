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
      className="w-full text-left rounded-lg border border-line bg-surface-muted p-3 flex items-start gap-3 hover:border-primary-300 dark:hover:border-primary-500/40 hover:bg-surface-subtle/50 dark:hover:bg-surface-muted transition-colors cursor-pointer">
      {server.icon_url ? (
        <img
          src={server.icon_url}
          alt=""
          className="w-8 h-8 rounded shrink-0 object-contain bg-surface"
        />
      ) : (
        <div className="w-8 h-8 rounded shrink-0 bg-primary-100 dark:bg-primary-500/20 flex items-center justify-center text-sm">
          🔌
        </div>
      )}
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium text-content truncate">{server.display_name}</p>
        {server.description && (
          <p className="text-xs text-content-muted line-clamp-4 mt-0.5">{server.description}</p>
        )}
      </div>
    </button>
  );
};

export default McpServerCard;
