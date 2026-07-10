import debugFactory from 'debug';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { feedbackApi } from '../../services/api/feedbackApi';
import type { FeedbackItem, FeedbackStatus } from '../../types/feedback';

const log = debugFactory('feedback:admin');

const ADMIN_STATUSES: FeedbackStatus[] = ['open', 'planned', 'completed', 'closed'];

const STATUS_LABEL_KEYS: Record<FeedbackStatus, string> = {
  open: 'feedback.status.open',
  planned: 'feedback.status.planned',
  completed: 'feedback.status.completed',
  closed: 'feedback.status.closed',
};

interface FeedbackAdminMenuProps {
  item: FeedbackItem;
  onUpdated: (updated: FeedbackItem) => void;
}

/**
 * Admin-only status control. The button is also gated by `requireAdmin` on the
 * server, so this is presentation/convenience, not the security boundary.
 */
export default function FeedbackAdminMenu({ item, onUpdated }: FeedbackAdminMenuProps) {
  const { t } = useT();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleChange = async (status: FeedbackStatus) => {
    if (pending || status === item.status) return;
    setPending(true);
    setError(null);
    try {
      const updated = await feedbackApi.updateStatus(item.id, status);
      onUpdated(updated);
    } catch (err) {
      log('updateStatus failed id=%s status=%s error=%O', item.id, status, err);
      setError(err instanceof Error ? err.message : t('feedback.admin.updateFailed'));
    } finally {
      setPending(false);
    }
  };

  return (
    <div className="flex items-center gap-2">
      <label htmlFor={`feedback-status-${item.id}`} className="text-xs text-content-muted">
        {t('feedback.admin.status')}
      </label>
      <select
        id={`feedback-status-${item.id}`}
        value={item.status}
        disabled={pending}
        onChange={e => handleChange(e.target.value as FeedbackStatus)}
        className="text-xs px-2 py-1 rounded-md bg-surface/5 border border-white/10 focus:outline-none focus:ring-2 focus:ring-primary-500/50 disabled:opacity-50">
        {ADMIN_STATUSES.map(status => (
          <option key={status} value={status}>
            {t(STATUS_LABEL_KEYS[status])}
          </option>
        ))}
      </select>
      {error && <span className="text-xs text-coral-500">{error}</span>}
    </div>
  );
}
