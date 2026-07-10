import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { FeedbackItem } from '../../types/feedback';
import FeedbackAdminMenu from './FeedbackAdminMenu';
import FeedbackComments from './FeedbackComments';
import FeedbackStatusBadge from './FeedbackStatusBadge';
import FeedbackVoteControl from './FeedbackVoteControl';

interface FeedbackItemRowProps {
  item: FeedbackItem;
  isAdmin: boolean;
  /** Bubbles an updated item (from a vote or status change) up to the list. */
  onChange: (updated: FeedbackItem) => void;
  /** Bubbles a comment-count bump by id so the parent merges it against the latest
   * row, rather than this row passing a reconstructed item built from stale props. */
  onCommentAdded?: (id: string) => void;
}

// Deterministic avatar tint from the author id, drawn from the app's palette.
const AVATAR_TINTS = [
  'bg-primary-500/15 text-primary-600 dark:text-primary-400',
  'bg-sage-500/15 text-sage-600 dark:text-sage-400',
  'bg-amber-500/15 text-amber-600 dark:text-amber-400',
  'bg-coral-500/15 text-coral-600 dark:text-coral-400',
];

function avatarTint(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i += 1) hash = (hash + id.charCodeAt(i)) % AVATAR_TINTS.length;
  return AVATAR_TINTS[hash];
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? ''
    : date.toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}

export default function FeedbackItemRow({
  item,
  isAdmin,
  onChange,
  onCommentAdded,
}: FeedbackItemRowProps) {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);

  const typeLabel = item.type === 'bug' ? t('feedback.type.bug') : t('feedback.type.feature');
  const typeClass =
    item.type === 'bug'
      ? 'bg-coral-500/10 text-coral-600 dark:text-coral-400'
      : 'bg-primary-500/10 text-primary-600 dark:text-primary-400';
  const handle = item.createdBy.length > 4 ? item.createdBy.slice(-4) : item.createdBy;
  const authorName = item.createdByName?.trim() || `@${handle}`;
  const avatarInitial = (item.createdByName?.trim() || handle).charAt(0).toUpperCase();

  return (
    <div className="group flex items-start gap-3 rounded-2xl border border-line bg-surface p-4 transition-all hover:border-line-strong hover:shadow-soft dark:hover:border-line-strong">
      <FeedbackVoteControl item={item} onVoted={onChange} />

      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${typeClass}`}>
            {typeLabel}
          </span>
          <FeedbackStatusBadge status={item.status} />
        </div>

        <h3 className="mt-2 break-words font-title text-[15px] font-semibold leading-snug text-content">
          {item.title}
        </h3>

        <p
          className={`mt-1 whitespace-pre-wrap break-words text-sm text-content-muted ${
            expanded ? '' : 'line-clamp-2'
          }`}>
          {item.body}
        </p>

        <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs text-content-faint">
          <span className="flex items-center gap-1.5">
            <span
              className={`flex h-5 w-5 items-center justify-center rounded-full text-[10px] font-semibold ${avatarTint(
                item.createdBy
              )}`}>
              {avatarInitial}
            </span>
            <span className="font-medium text-content-muted">{authorName}</span>
          </span>
          <span>·</span>
          <span>{formatDate(item.createdAt)}</span>
          <button
            type="button"
            onClick={() => setExpanded(prev => !prev)}
            className="flex items-center gap-1 transition-colors hover:text-primary-500">
            <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
              />
            </svg>
            {item.commentCount} {t('feedback.comments')}
          </button>
          <button
            type="button"
            onClick={() => setExpanded(prev => !prev)}
            className="transition-colors hover:text-primary-500">
            {expanded ? t('feedback.collapse') : t('feedback.expand')}
          </button>
          {item.github?.issueUrl && (
            <a
              href={item.github.issueUrl}
              target="_blank"
              rel="noreferrer noopener"
              className="text-primary-500 hover:underline">
              {t('feedback.viewIssue')}
            </a>
          )}
        </div>

        {expanded && (
          <FeedbackComments feedbackId={item.id} onCommentAdded={() => onCommentAdded?.(item.id)} />
        )}

        {isAdmin && (
          <div className="mt-3 border-t border-line pt-3">
            <FeedbackAdminMenu item={item} onUpdated={onChange} />
          </div>
        )}
      </div>
    </div>
  );
}
