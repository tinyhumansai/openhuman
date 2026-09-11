import { useT } from '../../lib/i18n/I18nContext';
import type { FeedbackStatus } from '../../types/feedback';

/** Pill + dot colour per status, using the app's semantic tokens. */
const STATUS_STYLES: Record<FeedbackStatus, { pill: string; dot: string }> = {
  open: { pill: 'bg-primary-500/10 text-primary-600 dark:text-primary-400', dot: 'bg-primary-500' },
  planned: { pill: 'bg-amber-500/10 text-amber-600 dark:text-amber-400', dot: 'bg-amber-500' },
  completed: { pill: 'bg-sage-500/10 text-sage-600 dark:text-sage-400', dot: 'bg-sage-500' },
  closed: { pill: 'bg-content-muted/10 text-content-muted', dot: 'bg-content-faint' },
};

const STATUS_LABEL_KEYS: Record<FeedbackStatus, string> = {
  open: 'feedback.status.open',
  planned: 'feedback.status.planned',
  completed: 'feedback.status.completed',
  closed: 'feedback.status.closed',
};

/**
 * Style for a status this build does not know about. Neutral on purpose — an
 * unrecognised status should not borrow the visual weight of "completed".
 */
const UNKNOWN_STATUS_STYLE = {
  pill: 'bg-content-muted/10 text-content-muted',
  dot: 'bg-content-faint',
};

export default function FeedbackStatusBadge({ status }: { status: FeedbackStatus }) {
  const { t } = useT();

  // Both lookups are guarded even though `FeedbackStatus` is a four-value
  // union, because that union is a claim about the *server's* data and nothing
  // validates it at the boundary: `feedbackApi` casts the JSON into these types.
  // A status the backend adds (or an item saved before one was renamed) arrives
  // as a string outside the union, `STATUS_STYLES[status]` is undefined, and
  // reading `.pill` off it threw — taking the whole feedback page down over one
  // row's badge. Falling back renders that row plainly instead.
  const style = STATUS_STYLES[status] ?? UNKNOWN_STATUS_STYLE;
  const labelKey = STATUS_LABEL_KEYS[status];

  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium ${style.pill}`}>
      <span className={`h-1.5 w-1.5 rounded-full ${style.dot}`} />
      {/* Show the raw value when there is no translation for it: a badge
          reading "in_review" is worse than the label but far better than a
          blank pill, and it names the unhandled value for whoever debugs it. */}
      {labelKey ? t(labelKey) : String(status ?? '')}
    </span>
  );
}
