/**
 * Aggregate health summary + bulk actions for the MCP server list.
 *
 * Lives in the left pane of `McpServersTab` above the list. Reads the
 * polled `statuses` array (no extra fetches) and surfaces:
 *
 *   - Live counts per state (connected / connecting / error / disconnected),
 *     announced through a `role="status" aria-live="polite"` region so
 *     screen readers hear updates as the polling loop refreshes.
 *   - `Retry all` button — visible only when there are servers in error
 *     state; iterates through them and calls `onReconnect` once.
 *   - `Disconnect all` button — visible only when there are connected
 *     servers; opens a confirm dialog (`role="dialog" aria-modal`) before
 *     firing `onDisconnect`.
 *
 * Parent (`McpServersTab`) owns the actual `mcpClientsApi` calls and the
 * subsequent `fetchStatuses()` refresh; this component only orchestrates
 * the user intent.
 */
import { useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogRoot,
  AlertDialogTitle,
} from '../../ui/AlertDialog';
import Button from '../../ui/Button';
import type { ConnStatus } from './types';

interface McpConnectionHealthToolbarProps {
  statuses: ConnStatus[];
  /** Reconnect every server in error state. Caller resolves after refresh. */
  onReconnect: (serverIds: string[]) => Promise<void>;
  /** Disconnect every currently-connected server. Caller resolves after refresh. */
  onDisconnect: (serverIds: string[]) => Promise<void>;
}

interface HealthCounts {
  connectedIds: string[];
  errorIds: string[];
  connectedCount: number;
  connectingCount: number;
  errorCount: number;
  disconnectedCount: number;
}

const computeHealthCounts = (statuses: ConnStatus[]): HealthCounts => {
  const connectedIds: string[] = [];
  const errorIds: string[] = [];
  let connectingCount = 0;
  let disconnectedCount = 0;
  for (const s of statuses) {
    switch (s.status) {
      case 'connected':
        connectedIds.push(s.server_id);
        break;
      case 'error':
        errorIds.push(s.server_id);
        break;
      case 'connecting':
        connectingCount += 1;
        break;
      // An `unauthorized` server is not connected, but it must NOT join the
      // `errorIds` set — "Retry all" blindly reconnects those, which would just
      // 401 again. Re-auth is a deliberate per-server action (sign in / token),
      // so we surface it under the disconnected tally rather than as an error.
      case 'unauthorized':
      case 'disconnected':
        disconnectedCount += 1;
        break;
    }
  }
  return {
    connectedIds,
    errorIds,
    connectedCount: connectedIds.length,
    connectingCount,
    errorCount: errorIds.length,
    disconnectedCount,
  };
};

const McpConnectionHealthToolbar = ({
  statuses,
  onReconnect,
  onDisconnect,
}: McpConnectionHealthToolbarProps) => {
  const { t } = useT();
  const [isOperating, setIsOperating] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const [opError, setOpError] = useState<string | null>(null);

  const counts = useMemo(() => computeHealthCounts(statuses), [statuses]);

  // Escape closing the "Disconnect all" confirmation WITHOUT firing the bulk
  // RPC is now `AlertDialog` / Radix's own default behavior — no hand-rolled
  // listener needed.

  // Nothing to summarise — match the parent's existing "hide chrome when
  // there's nothing installed" pattern.
  if (statuses.length === 0) return null;

  const runRetryAll = async () => {
    if (counts.errorIds.length === 0 || isOperating) return;
    setIsOperating(true);
    setOpError(null);
    try {
      await onReconnect(counts.errorIds);
    } catch (err) {
      setOpError(err instanceof Error ? err.message : t('mcp.health.opErrorGeneric'));
    } finally {
      setIsOperating(false);
    }
  };

  const runDisconnectAll = async () => {
    if (counts.connectedIds.length === 0 || isOperating) return;
    setConfirmDisconnect(false);
    setIsOperating(true);
    setOpError(null);
    try {
      await onDisconnect(counts.connectedIds);
    } catch (err) {
      setOpError(err instanceof Error ? err.message : t('mcp.health.opErrorGeneric'));
    } finally {
      setIsOperating(false);
    }
  };

  return (
    <div className="mb-2 rounded-lg border border-line bg-surface px-2.5 py-2">
      <div className="flex items-center justify-between gap-2 mb-1.5">
        <span className="text-[10px] font-semibold text-content-muted uppercase tracking-wide">
          {t('mcp.health.title')}
        </span>
        <div className="flex items-center gap-2">
          {counts.errorCount > 0 && (
            <Button
              variant="tertiary"
              size="xs"
              onClick={() => void runRetryAll()}
              disabled={isOperating}
              aria-label={t('mcp.health.retryAllAria').replace(
                '{count}',
                String(counts.errorCount)
              )}
              className="h-auto p-0 text-[10px] font-medium text-amber-700 hover:underline disabled:no-underline disabled:opacity-50 dark:text-amber-300">
              {t('mcp.health.retryAll').replace('{count}', String(counts.errorCount))}
            </Button>
          )}
          {counts.connectedCount > 0 && (
            <Button
              variant="tertiary"
              size="xs"
              onClick={() => setConfirmDisconnect(true)}
              disabled={isOperating}
              aria-label={t('mcp.health.disconnectAllAria').replace(
                '{count}',
                String(counts.connectedCount)
              )}
              className="h-auto p-0 text-[10px] font-medium text-content-secondary hover:underline disabled:no-underline disabled:opacity-50">
              {t('mcp.health.disconnectAll').replace('{count}', String(counts.connectedCount))}
            </Button>
          )}
        </div>
      </div>
      <div
        role="status"
        aria-live="polite"
        aria-label={t('mcp.health.summaryAria')}
        className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-content-secondary">
        <span className="inline-flex items-center gap-1">
          <span className="w-1.5 h-1.5 rounded-full bg-sage-500" aria-hidden="true" />
          <span>
            {t('mcp.health.connectedCount').replace('{count}', String(counts.connectedCount))}
          </span>
        </span>
        {counts.connectingCount > 0 && (
          <span className="inline-flex items-center gap-1">
            <span className="w-1.5 h-1.5 rounded-full bg-amber-400" aria-hidden="true" />
            <span>
              {t('mcp.health.connectingCount').replace('{count}', String(counts.connectingCount))}
            </span>
          </span>
        )}
        {counts.errorCount > 0 && (
          <span className="inline-flex items-center gap-1">
            <span className="w-1.5 h-1.5 rounded-full bg-coral-500" aria-hidden="true" />
            <span>{t('mcp.health.errorCount').replace('{count}', String(counts.errorCount))}</span>
          </span>
        )}
        <span className="inline-flex items-center gap-1">
          <span className="w-1.5 h-1.5 rounded-full bg-surface-strong" aria-hidden="true" />
          <span>
            {t('mcp.health.disconnectedCount').replace('{count}', String(counts.disconnectedCount))}
          </span>
        </span>
      </div>

      {opError && (
        <p
          role="alert"
          className="mt-1.5 text-[10px] text-coral-700 dark:text-coral-300 wrap-break-word">
          {opError}
        </p>
      )}

      <AlertDialogRoot open={confirmDisconnect} onOpenChange={setConfirmDisconnect}>
        <AlertDialogContent>
          <AlertDialogTitle>{t('mcp.health.disconnectConfirm.title')}</AlertDialogTitle>
          <AlertDialogDescription>
            {t('mcp.health.disconnectConfirm.body').replace(
              '{count}',
              String(counts.connectedCount)
            )}
          </AlertDialogDescription>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button variant="secondary" size="sm">
                {t('mcp.health.disconnectConfirm.cancel')}
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                variant="primary"
                tone="danger"
                size="sm"
                onClick={() => void runDisconnectAll()}>
                {t('mcp.health.disconnectConfirm.confirm')}
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialogRoot>
    </div>
  );
};

export default McpConnectionHealthToolbar;
