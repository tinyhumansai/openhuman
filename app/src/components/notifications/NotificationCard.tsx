import { useT } from '../../lib/i18n/I18nContext';
import type { IntegrationNotification } from '../../types/notifications';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import NotificationBody from './NotificationBody';

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/** Relative human-readable time string, e.g. "2m ago". */
function relativeTime(isoString: string): string {
  const diff = Date.now() - new Date(isoString).getTime();
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

/**
 * Provider badge tone. Six providers would need six hues and the app has four
 * themeable ramps — three of which the importance badge beside it already
 * spends on high / medium / low. A provider painted coral would read as a
 * failed notification, so the whole set takes the neutral pair; the badge
 * prints the provider slug, which is what a reader actually scans.
 *
 * Restoring a per-provider tint means adding brand tokens (the same product
 * decision the Telegram / Discord / iMessage plates in `skills/skillIcons.tsx`
 * are parked on), not reaching back for a stock Tailwind ramp.
 * See `gitbooks/developing/theming.md`.
 */
const PROVIDER_BADGE_CLASS = 'bg-surface-subtle text-content-secondary border-line';

/** Importance badge tone: high / medium / low on coral / amber / sage. */
function scoreBadgeClass(score: number): string {
  if (score >= 0.75) return 'bg-coral-100 text-coral-700 border-coral-200';
  if (score >= 0.4) return 'bg-amber-100 text-amber-700 border-amber-200';
  return 'bg-sage-100 text-sage-700 border-sage-200';
}

// ─────────────────────────────────────────────────────────────────────────────
// Component
// ─────────────────────────────────────────────────────────────────────────────

interface Props {
  notification: IntegrationNotification;
  onMarkRead: (id: string) => void;
  onNavigate?: (id: string) => void;
  onDismiss?: (id: string) => void;
}

const NotificationCard = ({ notification: n, onMarkRead, onNavigate, onDismiss }: Props) => {
  const { t } = useT();
  const isUnread = n.status === 'unread';

  const handleBodyClick = () => {
    if (onNavigate) {
      onNavigate(n.id);
    } else if (isUnread) {
      onMarkRead(n.id);
    }
  };

  return (
    <div
      className={`w-full p-3 border-b border-line-subtle hover:bg-surface-hover transition-colors duration-150 ${
        isUnread ? 'bg-primary-50/30' : 'bg-surface'
      }`}>
      <div className="flex items-start gap-3">
        {/* Unread dot — reserve space so text stays aligned whether read or unread */}
        <div className="mt-1.5 shrink-0 w-2">
          {isUnread && (
            <span className="block w-2 h-2 rounded-full bg-primary-500" aria-hidden="true" />
          )}
        </div>

        {/* `role="button"` + key handler instead of a real `<button>` because
            this wrapper contains `NotificationLinkPill` (also a `<button>`),
            and nested interactive elements break keyboard / screen-reader
            behaviour (HTML spec disallows it). */}
        <div
          role="button"
          tabIndex={0}
          onClick={handleBodyClick}
          onKeyDown={e => {
            // Ignore bubbled keydown from inner controls (e.g. the link pill).
            // Without this, pressing Enter/Space on a focused pill would also
            // activate the card body.
            if (e.target !== e.currentTarget) return;
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              handleBodyClick();
            }
          }}
          className="flex-1 min-w-0 text-left focus:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-1 rounded-sm">
          {/* Header row: provider badge + timestamp */}
          <div className="flex items-center gap-2 mb-1">
            <Badge className={PROVIDER_BADGE_CLASS}>{n.provider}</Badge>

            {n.importance_score !== undefined && (
              <Badge
                className={scoreBadgeClass(n.importance_score)}
                title={t('notifications.card.importanceTitle').replace(
                  '{pct}',
                  (n.importance_score * 100).toFixed(0)
                )}>
                {(n.importance_score * 100).toFixed(0)}%
              </Badge>
            )}

            {n.triage_action && n.triage_action !== 'drop' && n.triage_action !== 'acknowledge' && (
              <Badge variant="warning">{n.triage_action}</Badge>
            )}

            <span className="ml-auto text-[11px] text-content-faint shrink-0">
              {relativeTime(n.received_at)}
            </span>
          </div>

          {/* Title */}
          <p className="text-sm font-medium text-content truncate">{n.title}</p>

          {/* Body preview — `<openhuman-link>` tags render as pills */}
          {n.body && (
            <p
              data-testid="notification-card-body"
              className="text-xs text-content-muted mt-0.5 line-clamp-2">
              <NotificationBody body={n.body} />
            </p>
          )}
        </div>
        {onDismiss && (
          <Button
            iconOnly
            variant="tertiary"
            size="xs"
            onClick={() => onDismiss(n.id)}
            className="mt-0.5 ml-1 shrink-0 text-content-faint hover:text-content-secondary"
            aria-label={t('notifications.card.dismiss')}>
            <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </Button>
        )}
      </div>
    </div>
  );
};

export default NotificationCard;
