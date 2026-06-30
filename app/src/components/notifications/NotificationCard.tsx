import { useT } from '../../lib/i18n/I18nContext';
import type { IntegrationNotification } from '../../types/notifications';
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

/** Provider badge color class based on slug. */
function providerBadgeClass(provider: string): string {
  switch (provider) {
    case 'gmail':
      return 'bg-red-100 text-red-700 border-red-200';
    case 'slack':
      return 'bg-purple-100 text-purple-700 border-purple-200';
    case 'whatsapp':
      return 'bg-green-100 text-green-700 border-green-200';
    case 'discord':
      return 'bg-indigo-100 text-indigo-700 border-indigo-200';
    case 'telegram':
      return 'bg-blue-100 text-blue-700 border-blue-200';
    case 'linkedin':
      return 'bg-sky-100 text-sky-700 border-sky-200';
    default:
      return 'bg-surface-subtle text-content-secondary border-line';
  }
}

/** Score badge color. */
function scoreBadgeClass(score: number): string {
  if (score >= 0.75) return 'bg-coral-500/20 text-red-600 border-red-200';
  if (score >= 0.4) return 'bg-amber-100 text-amber-700 border-amber-200';
  return 'bg-sage-500/20 text-green-700 border-green-200';
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
        <div className="mt-1.5 flex-shrink-0 w-2">
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
          className="flex-1 min-w-0 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-1 rounded-sm">
          {/* Header row: provider badge + timestamp */}
          <div className="flex items-center gap-2 mb-1">
            <span
              className={`inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium border ${providerBadgeClass(n.provider)}`}>
              {n.provider}
            </span>

            {n.importance_score !== undefined && (
              <span
                className={`inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium border ${scoreBadgeClass(n.importance_score)}`}
                title={t('notifications.card.importanceTitle').replace(
                  '{pct}',
                  (n.importance_score * 100).toFixed(0)
                )}>
                {(n.importance_score * 100).toFixed(0)}%
              </span>
            )}

            {n.triage_action && n.triage_action !== 'drop' && n.triage_action !== 'acknowledge' && (
              <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium border bg-amber-100 text-amber-700 border-amber-200">
                {n.triage_action}
              </span>
            )}

            <span className="ml-auto text-[11px] text-content-faint flex-shrink-0">
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
            className="mt-0.5 ml-1 flex-shrink-0 text-content-faint hover:text-content-secondary"
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
