import debug from 'debug';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../services/coreRpcClient';
import { useAppDispatch } from '../../store/hooks';
import {
  clearNotificationActions,
  markRead,
  type NotificationItem,
} from '../../store/notificationSlice';
import Button from '../ui/Button';
import NotificationBody from './NotificationBody';

// Namespaced debug per project logging rules (mirrors nativeNotifications).
const log = debug('notifications:core-card');

/** Relative human-readable time string from epoch ms, e.g. "2m ago". */
function relativeTime(timestampMs: number): string {
  const diff = Math.max(0, Date.now() - timestampMs);
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

/**
 * Map a known meeting auto-join action id to its i18n key so button labels
 * are localized rather than trusting the (English) label the core sends.
 * Unknown action ids fall back to the server-provided label.
 */
const ACTION_LABEL_KEYS: Record<string, string> = {
  join_listen: 'notifications.meeting.joinListen',
  join_active: 'notifications.meeting.joinActive',
  skip: 'notifications.meeting.skip',
  always_join: 'notifications.meeting.alwaysJoin',
};

/** Primary (filled) vs secondary (outline) styling per action id. */
function isPrimaryAction(actionId: string): boolean {
  return actionId === 'join_listen' || actionId === 'join_active';
}

interface Props {
  notification: NotificationItem;
}

/**
 * Renders a core-originated notification (from `state.notifications.items`)
 * that carries action buttons — e.g. the calendar auto-join prompt
 * (issue #3507). Clicking a button dispatches the
 * `openhuman.agent_meetings_notification_action` RPC and marks the item read.
 */
const CoreNotificationCard = ({ notification: n }: Props) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [pendingActionId, setPendingActionId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleAction = async (actionId: string, payload: unknown) => {
    if (pendingActionId) return; // ignore double-clicks while a call is in flight
    setPendingActionId(actionId);
    setError(null);
    log('action click id=%s notification=%s', actionId, n.id);
    try {
      await callCoreRpc<{ ok: boolean }>({
        method: 'openhuman.agent_meetings_notification_action',
        params: { action_id: actionId, payload },
      });
      log('action ok id=%s', actionId);
      dispatch(markRead({ id: n.id }));
      // Remove the buttons so the handled prompt can't be re-clicked (which would
      // re-fire bot:join, or flip always_join after a skip). Without this the
      // card stays pinned in NotificationCenter with live actions.
      dispatch(clearNotificationActions({ id: n.id }));
    } catch (err) {
      log('action failed id=%s err=%o', actionId, err);
      setError(t('notifications.meeting.actionError'));
    } finally {
      setPendingActionId(null);
    }
  };

  return (
    <div
      className={`w-full p-3 border-b border-line-subtle transition-colors duration-150 ${
        n.read ? 'bg-surface' : 'bg-primary-50/30'
      }`}
      data-testid="core-notification-card">
      <div className="flex items-start gap-3">
        {/* Unread dot — reserve space so text stays aligned whether read or unread */}
        <div className="mt-1.5 flex-shrink-0 w-2">
          {!n.read && (
            <span className="block w-2 h-2 rounded-full bg-primary-500" aria-hidden="true" />
          )}
        </div>

        <div className="flex-1 min-w-0 text-left">
          {/* Header row: category badge + timestamp */}
          <div className="flex items-center gap-2 mb-1">
            <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium border bg-surface-subtle text-content-secondary border-line">
              {t(`notifications.category.${n.category}`)}
            </span>
            <span className="ml-auto text-[11px] text-content-faint flex-shrink-0">
              {relativeTime(n.timestamp)}
            </span>
          </div>

          {/* Title */}
          <p className="text-sm font-medium text-content">{n.title}</p>

          {/* Body */}
          {n.body && (
            <p data-testid="core-notification-body" className="text-xs text-content-muted mt-0.5">
              <NotificationBody body={n.body} />
            </p>
          )}

          {/* Action buttons */}
          {n.actions && n.actions.length > 0 && (
            <div className="flex flex-wrap items-center gap-2 mt-2">
              {n.actions.map(action => {
                const labelKey = ACTION_LABEL_KEYS[action.actionId];
                const label = labelKey ? t(labelKey) : action.label;
                const primary = isPrimaryAction(action.actionId);
                return (
                  <Button
                    key={action.actionId}
                    variant={primary ? 'primary' : 'secondary'}
                    size="xs"
                    disabled={pendingActionId !== null}
                    onClick={() => {
                      void handleAction(action.actionId, action.payload);
                    }}>
                    {label}
                  </Button>
                );
              })}
            </div>
          )}

          {error && <p className="text-xs text-red-600 mt-1.5">{error}</p>}
        </div>
      </div>
    </div>
  );
};

export default CoreNotificationCard;
