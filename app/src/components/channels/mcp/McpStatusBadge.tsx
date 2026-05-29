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
    className:
      'bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400 border-stone-200 dark:border-neutral-700',
  },
  error: {
    i18nKey: 'channels.status.error',
    className: 'bg-coral-500/10 text-coral-700 border-coral-500/30 dark:text-coral-300',
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
