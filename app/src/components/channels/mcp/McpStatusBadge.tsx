/**
 * Status badge for MCP server connection states.
 * Mirrors ChannelStatusBadge but uses ServerStatus values; reuses the
 * shared `channels.status.*` i18n keys since the label vocabulary is
 * identical (Connected / Connecting / Disconnected / Error).
 */
import { useT } from '../../../lib/i18n/I18nContext';
import type { ServerStatus } from './types';

const STATUS_META: Record<ServerStatus, { i18nKey: string; className: string }> = {
  connected: {
    i18nKey: 'channels.status.connected',
    className: 'bg-sage-500/10 text-sage-700 border-sage-500/30 dark:text-sage-300',
  },
  connecting: {
    i18nKey: 'channels.status.connecting',
    className: 'bg-amber-500/10 text-amber-700 border-amber-500/30 dark:text-amber-300',
  },
  disconnected: {
    i18nKey: 'channels.status.disconnected',
    className: 'bg-surface-subtle text-content-muted border-line',
  },
  unauthorized: {
    i18nKey: 'mcp.status.unauthorized',
    className: 'bg-amber-500/10 text-amber-700 border-amber-500/30 dark:text-amber-300',
  },
  error: {
    i18nKey: 'channels.status.error',
    className: 'bg-coral-500/10 text-coral-700 border-coral-500/30 dark:text-coral-300',
  },
  disabled: {
    i18nKey: 'mcp.status.disabled',
    className: 'bg-surface-subtle text-content-faint border-line italic',
  },
};

interface McpStatusBadgeProps {
  status: ServerStatus;
  className?: string;
}

const McpStatusBadge = ({ status, className = '' }: McpStatusBadgeProps) => {
  const { t } = useT();
  const meta = STATUS_META[status] ?? STATUS_META.disconnected;
  return (
    <span
      role="status"
      aria-live="polite"
      className={`shrink-0 px-2 py-1 text-[11px] border rounded-full ${meta.className} ${className}`}>
      {t(meta.i18nKey)}
    </span>
  );
};

export default McpStatusBadge;
