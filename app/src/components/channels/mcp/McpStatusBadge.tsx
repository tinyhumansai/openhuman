/**
 * Status badge for MCP server connection states.
 * Mirrors ChannelStatusBadge but uses ServerStatus values.
 */
import type { ServerStatus } from './types';

const STATUS_STYLES: Record<ServerStatus, { label: string; className: string }> = {
  connected: {
    label: 'Connected',
    className: 'bg-sage-500/10 text-sage-700 border-sage-500/30 dark:text-sage-300',
  },
  connecting: {
    label: 'Connecting',
    className: 'bg-amber-500/10 text-amber-700 border-amber-500/30 dark:text-amber-300',
  },
  disconnected: {
    label: 'Disconnected',
    className:
      'bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400 border-stone-200 dark:border-neutral-700',
  },
  error: {
    label: 'Error',
    className: 'bg-coral-500/10 text-coral-700 border-coral-500/30 dark:text-coral-300',
  },
};

interface McpStatusBadgeProps {
  status: ServerStatus;
  className?: string;
}

const McpStatusBadge = ({ status, className = '' }: McpStatusBadgeProps) => {
  const style = STATUS_STYLES[status] ?? STATUS_STYLES.disconnected;
  return (
    <span
      className={`shrink-0 px-2 py-1 text-[11px] border rounded-full ${style.className} ${className}`}>
      {style.label}
    </span>
  );
};

export default McpStatusBadge;
