import { useT } from '../../lib/i18n/I18nContext';
import type { FeedbackStatus } from '../../types/feedback';

/** Pill + dot colour per status, using the app's semantic tokens. */
const STATUS_STYLES: Record<FeedbackStatus, { pill: string; dot: string }> = {
  open: { pill: 'bg-primary-500/10 text-primary-600 dark:text-primary-400', dot: 'bg-primary-500' },
  planned: { pill: 'bg-amber-500/10 text-amber-600 dark:text-amber-400', dot: 'bg-amber-500' },
  completed: { pill: 'bg-sage-500/10 text-sage-600 dark:text-sage-400', dot: 'bg-sage-500' },
  closed: { pill: 'bg-neutral-500/10 text-content-muted', dot: 'bg-neutral-400' },
};

const STATUS_LABEL_KEYS: Record<FeedbackStatus, string> = {
  open: 'feedback.status.open',
  planned: 'feedback.status.planned',
  completed: 'feedback.status.completed',
  closed: 'feedback.status.closed',
};

export default function FeedbackStatusBadge({ status }: { status: FeedbackStatus }) {
  const { t } = useT();
  const style = STATUS_STYLES[status];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium ${style.pill}`}>
      <span className={`h-1.5 w-1.5 rounded-full ${style.dot}`} />
      {t(STATUS_LABEL_KEYS[status])}
    </span>
  );
}
