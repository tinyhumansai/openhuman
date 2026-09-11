/**
 * Card component for a single MCP registry server.
 * Shows icon, title, description, and author derived from qualified name.
 */
import Button from '../../ui/Button';
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
    <Button
      variant="tertiary"
      onClick={() => onSelect(server.qualified_name)}
      className="h-auto w-full items-start justify-start gap-3 rounded-lg border border-line bg-surface-muted p-3 text-left font-normal hover:border-primary-300 hover:bg-surface-subtle/50 dark:hover:border-primary-500/40 dark:hover:bg-surface-muted">
      {server.icon_url ? (
        <img
          src={server.icon_url}
          alt=""
          className="h-8 w-8 shrink-0 rounded bg-surface object-contain"
        />
      ) : (
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded bg-primary-100 text-sm dark:bg-primary-500/20">
          🔌
        </div>
      )}
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-content">{server.display_name}</p>
        {server.description && (
          <p className="mt-0.5 line-clamp-4 text-xs text-content-muted">{server.description}</p>
        )}
      </div>
    </Button>
  );
};

export default McpServerCard;
