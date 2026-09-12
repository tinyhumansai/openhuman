import { useT } from '../../lib/i18n/I18nContext';
import { type NotificationItem } from '../../store/notificationSlice';
import Badge from '../ui/Badge';
import NotificationBody from './NotificationBody';

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

interface Props {
  notification: NotificationItem;
}

/**
 * Display-only fallback for a core-originated notification (from
 * `state.notifications.items`) that carries actions but has no dedicated card.
 *
 * Both action-producing domains today — the approval gate and flow approvals —
 * render through `GateApprovalCard` / `FlowApprovalCard`, so nothing currently
 * reaches this card. It stays as the catch-all so an action-carrying
 * notification from a future producer is still *shown* rather than silently
 * dropped: `NotificationCenter` sources core items only through this branch.
 * It deliberately renders no buttons — the meeting auto-join prompt that owned
 * them went with the Meet domain, and its
 * `openhuman.agent_meetings_notification_action` RPC no longer exists.
 */
const CoreNotificationCard = ({ notification: n }: Props) => {
  const { t } = useT();

  return (
    <div
      className={`w-full p-3 border-b border-line-subtle transition-colors duration-150 ${
        n.read ? 'bg-surface' : 'bg-primary-50/30'
      }`}
      data-testid="core-notification-card">
      <div className="flex items-start gap-3">
        {/* Unread dot — reserve space so text stays aligned whether read or unread */}
        <div className="mt-1.5 shrink-0 w-2">
          {!n.read && (
            <span className="block w-2 h-2 rounded-full bg-primary-500" aria-hidden="true" />
          )}
        </div>

        <div className="flex-1 min-w-0 text-left">
          {/* Header row: category badge + timestamp */}
          <div className="flex items-center gap-2 mb-1">
            <Badge>{t(`notifications.category.${n.category}`)}</Badge>
            <span className="ml-auto text-[11px] text-content-faint shrink-0">
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
        </div>
      </div>
    </div>
  );
};

export default CoreNotificationCard;
